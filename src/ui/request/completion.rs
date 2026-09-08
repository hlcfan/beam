use std::collections::BTreeMap;

use gpui_kit::component::input::{Rope, RopeExt};
use gpui_kit::{AppContext, Context, Focusable, Window};
use lsp_types::{CompletionItem, CompletionItemKind, CompletionTextEdit, TextEdit};

use crate::models::{EnvironmentFile, EnvironmentVariable};
use crate::template_variables::DYNAMIC_VARIABLE_NAMES;
use crate::ui::BeamView;

impl BeamView {
    pub(super) fn update_request_body_completion(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_request_body_completion(cx);
        let editor = self.request_body_editor.read(cx);
        if !editor.focus_handle(cx).is_focused(window) {
            return;
        }
        let offset = editor.cursor();
        let Some(token) = token_at_cursor(editor.text(), offset) else {
            return;
        };
        let path = self
            .selected_environment_id_for_view()
            .and_then(|id| self.environment_file_path_from_shell(id));
        let editor = self.request_body_editor.downgrade();
        let prefix = token.prefix.clone();
        let start = token.start;
        let items = cx.background_spawn(async move {
            let variables = path
                .and_then(|path| std::fs::read_to_string(path).ok())
                .and_then(|content| toml::from_str::<EnvironmentFile>(&content).ok())
                .map(|file| file.variables)
                .unwrap_or_default();
            completion_items(&token, &variables)
        });
        self.request_body_completion_task = Some(cx.spawn_in(window, async move |_, cx| {
            let items = items.await;
            let _ = editor.update_in(cx, |editor, window, cx| {
                if editor.cursor() == offset && editor.focus_handle(cx).is_focused(window) {
                    // Each result owns its token range; no document-prefix query is constructed.
                    editor.present_completion_items(start, prefix, items, cx);
                }
            });
        }));
    }

    pub(super) fn dismiss_request_body_completion(&mut self, cx: &mut Context<Self>) {
        self.request_body_completion_task = None;
        self.request_body_editor.update(cx, |editor, cx| {
            editor.dismiss_completion_overlay(cx);
        });
    }
}

// Completion discovery stays bounded even in a single-line, multi-megabyte body.
// Longer variable names can still be entered and resolved normally.
const MAX_TOKEN_BYTES: usize = 4096;

struct VariableToken {
    start: usize,
    prefix: String,
    range: lsp_types::Range,
}

fn token_at_cursor(rope: &Rope, offset: usize) -> Option<VariableToken> {
    if offset > rope.len() || !rope.is_char_boundary(offset) {
        return None;
    }
    let mut before = rope.chars_at(offset);
    let mut start = offset;
    loop {
        let ch = before.prev()?;
        start -= ch.len_utf8();
        if offset - start > MAX_TOKEN_BYTES {
            return None;
        }
        match ch {
            '{' => {
                if before.prev()? != '{' {
                    return None;
                }
                start -= 1;
                break;
            }
            '}' | '\n' | '\r' => return None,
            _ => {}
        }
    }

    let mut after = rope.chars_at(offset).peekable();
    let mut end = offset;
    while let Some(ch) = after.peek().copied() {
        if !ch.is_alphanumeric() && !matches!(ch, '_' | '-' | '.' | '$') {
            break;
        }
        end += ch.len_utf8();
        after.next();
        if end - start > MAX_TOKEN_BYTES {
            return None;
        }
    }
    let mut closing = end;
    while matches!(after.peek(), Some(' ' | '\t')) {
        closing += 1;
        after.next();
        if closing - start > MAX_TOKEN_BYTES {
            return None;
        }
    }
    if after.next() == Some('}') {
        end = closing + 1;
        if after.next() == Some('}') {
            end += 1;
        }
    }
    Some(VariableToken {
        start,
        prefix: rope
            .slice(start + 2..offset)
            .to_string()
            .trim_start()
            .to_string(),
        range: lsp_types::Range::new(
            completion_position(rope, start),
            completion_position(rope, end),
        ),
    })
}

fn completion_position(rope: &Rope, offset: usize) -> lsp_types::Position {
    let point = rope.offset_to_point(offset);
    // Match gpui-kit's character-column convention, using rope indexes instead of
    // counting every character from the start of a potentially very long line.
    let column = rope.byte_to_char_idx(offset) - rope.byte_to_char_idx(offset - point.column);
    lsp_types::Position::new(point.row as u32, column as u32)
}

fn completion_items(
    token: &VariableToken,
    variables: &[EnvironmentVariable],
) -> Vec<CompletionItem> {
    let mut names = BTreeMap::new();
    for name in DYNAMIC_VARIABLE_NAMES {
        names.insert(name, "Dynamic variable");
    }
    for variable in variables.iter().filter(|variable| variable.enabled) {
        let name = variable.name.trim();
        if !name.is_empty() {
            names.insert(name, "Environment variable");
        }
    }
    names
        .into_iter()
        .filter(|(name, _)| name.starts_with(&token.prefix))
        .map(|(name, detail)| CompletionItem {
            label: name.to_string(),
            kind: Some(CompletionItemKind::VARIABLE),
            detail: Some(detail.to_string()),
            filter_text: Some(token.prefix.clone()),
            text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                range: token.range,
                new_text: format!("{{{{{name}}}}}"),
            })),
            ..Default::default()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn complete(text: &str, offset: usize, name: &str) -> String {
        let rope = Rope::from(text);
        let token = token_at_cursor(&rope, offset).expect("variable token");
        let variables = [EnvironmentVariable {
            name: name.into(),
            value: "secret".into(),
            enabled: true,
            description: None,
        }];
        let item = completion_items(&token, &variables)
            .into_iter()
            .find(|item| item.label == name)
            .unwrap();
        let Some(CompletionTextEdit::Edit(edit)) = item.text_edit else {
            panic!("text edit")
        };
        let mut result = text.to_string();
        result.replace_range(
            rope.position_to_offset(&edit.range.start)..rope.position_to_offset(&edit.range.end),
            &edit.new_text,
        );
        result
    }

    #[test]
    fn replaces_partial_and_existing_tokens() {
        assert_eq!(complete("{{ho", 4, "host"), "{{host}}");
        assert_eq!(complete("{{ho}}", 4, "host"), "{{host}}");
        assert_eq!(complete("{{ho}", 4, "host"), "{{host}}");
        assert_eq!(complete("{{hostname}}", 4, "host"), "{{host}}");
        assert_eq!(complete("{{ ho  }}", 5, "host"), "{{host}}");
        assert_eq!(
            complete("{\"url\":\"{{ho\"}", 12, "host"),
            "{\"url\":\"{{host}}\"}"
        );
    }

    #[test]
    fn preserves_unicode_and_multiline_surroundings() {
        let text = "😀\n中文 {{主}}!";
        assert_eq!(
            complete(text, "😀\n中文 {{主".len(), "主机"),
            "😀\n中文 {{主机}}!"
        );
    }

    #[test]
    fn only_suggests_inside_open_tokens() {
        for text in ["", "{", "{\"key\":", "{{host}}", "{{host}} plain", "{{\n"] {
            assert!(
                token_at_cursor(&Rope::from(text), text.len()).is_none(),
                "{text}"
            );
        }
        let text = "{{host}} {{";
        assert_eq!(
            token_at_cursor(&Rope::from(text), text.len())
                .unwrap()
                .prefix,
            ""
        );
    }

    #[test]
    fn discovers_tokens_in_large_single_line_bodies() {
        let padding = "😀".repeat(256 * 1024);
        let rope = Rope::from(format!("{padding}{{{{ho}}}}{padding}"));
        let token = token_at_cursor(&rope, padding.len() + 4).unwrap();
        assert_eq!(token.prefix, "ho");
        assert_eq!(token.start, padding.len());
        assert_eq!(token.range.start, lsp_types::Position::new(0, 256 * 1024));
        assert_eq!(token.range.end, lsp_types::Position::new(0, 256 * 1024 + 6));
    }

    #[test]
    fn discovery_is_bounded_and_rejects_invalid_offsets() {
        let long_name = "x".repeat(MAX_TOKEN_BYTES + 1);
        let rope = Rope::from(format!("{{{{{long_name}}}}}"));
        assert!(token_at_cursor(&rope, 2).is_none());
        assert!(token_at_cursor(&rope, MAX_TOKEN_BYTES + 2).is_none());
        let rope = Rope::from("😀{{ho");
        assert!(token_at_cursor(&rope, 1).is_none());
        assert!(token_at_cursor(&rope, rope.len() + 1).is_none());
    }

    #[test]
    fn later_completion_does_not_affect_earlier_token() {
        let rope = Rope::from("{{ho}} later {{ho}}");
        let later = token_at_cursor(&rope, rope.len() - 2).unwrap();
        let earlier = token_at_cursor(&rope, 4).unwrap();
        assert!(later.start > earlier.start);
        assert_eq!(earlier.start, 0);
        assert_eq!(earlier.prefix, "ho");
        assert_eq!(earlier.range.end, lsp_types::Position::new(0, 6));
    }

    #[test]
    #[ignore = "manual performance measurement"]
    fn measure_completion_discovery() {
        for size in [10_000, 10_000_000] {
            let rope = Rope::from(format!("{}{{{{ho}}}}", "x".repeat(size)));
            let start = std::time::Instant::now();
            for _ in 0..1_000 {
                std::hint::black_box(token_at_cursor(&rope, rope.len() - 2));
            }
            eprintln!(
                "{size} byte prefix: {:?} for 1000 token lookups",
                start.elapsed()
            );
        }
    }

    #[test]
    fn filters_enabled_names_and_prefers_environment_over_dynamic() {
        let variable = |name: &str, enabled| EnvironmentVariable {
            name: name.into(),
            value: "secret".into(),
            enabled,
            description: None,
        };
        let variables = [
            variable(" host ", true),
            variable("hidden", false),
            variable("", true),
            variable("host", true),
            variable("$guid", true),
        ];
        let token = token_at_cursor(&Rope::from("{{"), 2).unwrap();
        let items = completion_items(&token, &variables);
        assert_eq!(items.len(), 5);
        assert_eq!(
            items
                .iter()
                .find(|item| item.label == "$guid")
                .unwrap()
                .detail
                .as_deref(),
            Some("Environment variable")
        );
        let token = token_at_cursor(&Rope::from("{{$g"), 4).unwrap();
        let items = completion_items(&token, &[]);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].label, "$guid");
        let token = token_at_cursor(&Rope::from("{{missing"), 9).unwrap();
        assert!(completion_items(&token, &variables).is_empty());
    }
}
