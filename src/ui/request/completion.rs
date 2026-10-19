use std::collections::BTreeMap;
use std::sync::Arc;

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
        let editor = self.request_body_editor.read(cx);
        if !editor.focus_handle(cx).is_focused(window) {
            self.dismiss_request_body_completion(cx);
            return;
        }
        let offset = editor.cursor();
        let Some(token) = token_at_cursor(editor.text(), offset) else {
            self.dismiss_request_body_completion(cx);
            return;
        };

        let Some(variables) = self.load_completion_environment_variables(window, cx) else {
            self.dismiss_request_body_completion(cx);
            self.request_body_completion_pending = true;
            return;
        };
        self.request_body_completion_pending = false;
        let items = completion_items(&token, &variables);
        self.request_body_editor.update(cx, |editor, cx| {
            editor.present_completion_items(token.start, token.prefix, items, cx);
        });
    }

    /// Loads one environment snapshot for both completion UIs. Menu dismissal
    /// only cancels presentation; the snapshot and its load survive token changes.
    pub(in crate::ui) fn load_completion_environment_variables(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Arc<Vec<EnvironmentVariable>>> {
        let environment_id = self.selected_environment_id_for_view();
        if let Some((cached_id, variables)) = &self.completion_environment_variables
            && *cached_id == environment_id
        {
            return Some(variables.clone());
        }
        if self
            .completion_environment_task
            .as_ref()
            .is_some_and(|(pending_id, _)| *pending_id == environment_id)
        {
            return None;
        }

        self.completion_environment_variables = None;
        self.completion_environment_task = None;
        let path = environment_id.and_then(|id| self.environment_file_path_from_shell(id));
        let variables = cx.background_spawn(async move { load_environment_variables(path) });
        let task = cx.spawn_in(window, async move |this, cx| {
            let variables = variables.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.completion_environment_task = None;
                let body_pending = std::mem::take(&mut this.request_body_completion_pending);
                let url_pending = std::mem::take(&mut this.url_completion_pending);
                if this.selected_environment_id_for_view() != environment_id {
                    return;
                }
                let load_failed = variables.is_none();
                // An empty snapshot lets both editors present built-ins on failure.
                this.completion_environment_variables =
                    Some((environment_id, Arc::new(variables.unwrap_or_default())));
                // Each editor rechecks its focus and current token before presenting.
                if body_pending {
                    this.update_request_body_completion(window, cx);
                }
                if url_pending {
                    this.update_url_completion(window, cx);
                }
                // Keep the fallback only for presentation; the next edit retries the file.
                if load_failed {
                    this.completion_environment_variables = None;
                }
            });
        });
        self.completion_environment_task = Some((environment_id, task));
        None
    }

    pub(in crate::ui) fn dismiss_request_body_completion(&mut self, cx: &mut Context<Self>) {
        self.request_body_completion_pending = false;
        self.request_body_editor.update(cx, |editor, cx| {
            editor.dismiss_completion_overlay(cx);
        });
    }
}

// Completion discovery stays bounded even in a single-line, multi-megabyte body.
// Longer variable names can still be entered and resolved normally.
const MAX_TOKEN_BYTES: usize = 4096;

pub(super) struct VariableToken {
    pub(super) start: usize,
    /// End of the token to replace, including a closing `}}` that is already typed.
    pub(super) end: usize,
    pub(super) prefix: String,
    range: lsp_types::Range,
}

pub(super) fn token_at_cursor(rope: &Rope, offset: usize) -> Option<VariableToken> {
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
    // Only consume a trailing word when it belongs to an existing variable
    // token (i.e. it is closed by `}`/`}}`). A fresh `{{` inserted in front
    // of plain text (e.g. `The {{|name`) must insert instead of replacing.
    let mut tentative = offset;
    let mut truncated = false;
    while let Some(ch) = after.peek().copied() {
        if !ch.is_alphanumeric() && !matches!(ch, '_' | '-' | '.' | '$') {
            break;
        }
        tentative += ch.len_utf8();
        after.next();
        if tentative - start > MAX_TOKEN_BYTES {
            truncated = true;
            break;
        }
    }
    let mut closing = tentative;
    if !truncated {
        while matches!(after.peek(), Some(' ' | '\t')) {
            closing += 1;
            after.next();
            if closing - start > MAX_TOKEN_BYTES {
                truncated = true;
                break;
            }
        }
    }
    let mut end = offset;
    if !truncated && after.next() == Some('}') {
        end = closing + 1;
        if after.next() == Some('}') {
            end += 1;
        }
    }
    Some(VariableToken {
        start,
        end,
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

/// A variable the user can insert, as the completion UIs list it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct VariableCandidate {
    pub(super) name: String,
    pub(super) detail: &'static str,
}

impl VariableCandidate {
    /// The text that replaces the typed token.
    pub(super) fn insertion(&self) -> String {
        format!("{{{{{}}}}}", self.name)
    }
}

/// Variables whose name starts with `prefix`, sorted by name. Environment
/// variables shadow dynamic ones of the same name.
pub(super) fn variable_candidates(
    prefix: &str,
    variables: &[EnvironmentVariable],
) -> Vec<VariableCandidate> {
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
        .filter(|(name, _)| name.starts_with(prefix))
        .map(|(name, detail)| VariableCandidate {
            name: name.to_string(),
            detail,
        })
        .collect()
}

/// `None` means an environment file was selected but could not be read or parsed.
fn load_environment_variables(
    path: Option<std::path::PathBuf>,
) -> Option<Vec<EnvironmentVariable>> {
    let Some(path) = path else {
        return Some(Vec::new());
    };
    let content = std::fs::read_to_string(path).ok()?;
    toml::from_str::<EnvironmentFile>(&content)
        .ok()
        .map(|file| file.variables)
}

fn completion_items(
    token: &VariableToken,
    variables: &[EnvironmentVariable],
) -> Vec<CompletionItem> {
    variable_candidates(&token.prefix, variables)
        .into_iter()
        .map(|candidate| CompletionItem {
            label: candidate.name.clone(),
            kind: Some(CompletionItemKind::VARIABLE),
            detail: Some(candidate.detail.to_string()),
            filter_text: Some(token.prefix.clone()),
            text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                range: token.range,
                new_text: candidate.insertion(),
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
        // Oversized trailing words are not consumed; completion inserts at
        // the cursor instead of scanning the whole suffix.
        let token = token_at_cursor(&rope, 2).expect("insertion token");
        assert_eq!(token.start, 0);
        assert_eq!(token.range.start, lsp_types::Position::new(0, 0));
        assert_eq!(token.range.end, lsp_types::Position::new(0, 2));
        assert!(token_at_cursor(&rope, MAX_TOKEN_BYTES + 2).is_none());
        let rope = Rope::from("😀{{ho");
        assert!(token_at_cursor(&rope, 1).is_none());
        assert!(token_at_cursor(&rope, rope.len() + 1).is_none());
    }

    #[test]
    fn inserts_before_plain_word_without_replacing_it() {
        assert_eq!(complete("The {{name", 6, "host"), "The {{host}}name");
        assert_eq!(complete("The {{name}}", 6, "host"), "The {{host}}");
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

#[cfg(test)]
#[path = "completion/tests.rs"]
mod cache_tests;
