use std::collections::BTreeMap;

use gpui_kit::component::input::{CompletionProvider, Rope, RopeExt};
use gpui_kit::{App, AppContext, Task, WeakEntity, Window};
use lsp_types::{
    CompletionContext, CompletionItem, CompletionItemKind, CompletionResponse, CompletionTextEdit,
    TextEdit,
};

use crate::models::{EnvironmentFile, EnvironmentVariable};
use crate::template_variables::DYNAMIC_VARIABLE_NAMES;
use crate::ui::BeamView;

pub(super) struct VariableCompletionProvider {
    view: WeakEntity<BeamView>,
}

impl VariableCompletionProvider {
    pub(super) fn new(view: WeakEntity<BeamView>) -> Self {
        Self { view }
    }
}

impl CompletionProvider for VariableCompletionProvider {
    fn completions(
        &self,
        text: &Rope,
        offset: usize,
        _: CompletionContext,
        _: &mut Window,
        cx: &mut App,
    ) -> Task<anyhow::Result<CompletionResponse>> {
        let Some(token) = token_at_cursor(text, offset) else {
            return Task::ready(Ok(CompletionResponse::Array(vec![])));
        };
        let view = self.view.clone();
        // Defer reading the view: programmatic editor changes can occur while it is updating.
        cx.spawn(async move |cx| {
            let path = view.read_with(cx, |view, _| {
                view.selected_environment_id_for_view()
                    .and_then(|id| view.environment_file_path_from_shell(id))
            })?;
            let items = cx
                .background_spawn(async move {
                    let variables = path
                        .and_then(|path| std::fs::read_to_string(path).ok())
                        .and_then(|content| toml::from_str::<EnvironmentFile>(&content).ok())
                        .map(|file| file.variables)
                        .unwrap_or_default();
                    completion_items(&token, &variables)
                })
                .await;
            Ok(CompletionResponse::Array(items))
        })
    }

    fn is_completion_trigger(&self, _: usize, _: &str, _: &mut App) -> bool {
        // Recompute on edits, including deletion and closing braces, to clear stale suggestions.
        true
    }
}

struct VariableToken {
    prefix: String,
    range: lsp_types::Range,
}

fn token_at_cursor(rope: &Rope, offset: usize) -> Option<VariableToken> {
    let text = rope.to_string();
    let before = text.get(..offset)?;
    let start = before.rfind("{{")?;
    let prefix = &before[start + 2..];
    if prefix
        .chars()
        .any(|ch| matches!(ch, '{' | '}' | '\n' | '\r'))
    {
        return None;
    }
    // Replace the remaining name and any existing closing braces, without consuming JSON/XML.
    let suffix = &text[offset..];
    let name_end = suffix
        .find(|ch: char| !ch.is_alphanumeric() && !matches!(ch, '_' | '-' | '.' | '$'))
        .unwrap_or(suffix.len());
    let mut end = offset + name_end;
    let after_name = &text[end..];
    let whitespace = after_name.len() - after_name.trim_start_matches([' ', '\t']).len();
    let closing = &after_name[whitespace..];
    if closing.starts_with("}}") {
        end += whitespace + 2;
    } else if closing.starts_with('}') {
        end += whitespace + 1;
    }
    Some(VariableToken {
        prefix: prefix.trim_start().to_string(),
        // Use the editor's own position convention for edits, including non-ASCII text.
        range: lsp_types::Range::new(rope.offset_to_position(start), rope.offset_to_position(end)),
    })
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
