pub(super) mod history;
pub(super) mod persistence;
pub(super) mod render;

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ResponseTab {
    Body,
    Headers,
}

pub(super) const MACOS_COMMAND_ICON_PATH: &str = "icons/command.svg";
pub(super) const NON_MACOS_COMMAND_ICON_PATH: &str = "icons/chevron-up.svg";

impl BeamView {
    pub(in crate::ui) fn clear_response_pane(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.response_status = "—".to_string();
        self.response_status_code = None;
        self.response_time = "—".to_string();
        self.response_size = "—".to_string();
        self.response_headers_raw.clear();
        self.response_content_type = None;
        self.update_response_body_editor_with_scroll_persistence_suppressed(
            window,
            cx,
            |input, window, cx| input.set_value(String::new(), window, cx),
        );
    }

    pub(in crate::ui) fn apply_response_snapshot(
        &mut self,
        snapshot: &StoredResponseSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let content_type = snapshot.content_type.clone();
        let formatted_body =
            self.response_body_for_display(&snapshot.body, content_type.as_deref());
        let language = response_body_editor_language(content_type.as_deref());
        self.response_status = snapshot.status.clone();
        self.response_status_code = snapshot.status_code;
        self.response_time = snapshot.time.clone();
        self.response_size = snapshot.size.clone();
        self.response_headers_raw = snapshot.headers_raw.clone();
        self.response_content_type = content_type;
        self.update_response_body_editor_with_scroll_persistence_suppressed(
            window,
            cx,
            |input, window, cx| {
                input.set_highlighter(language, cx);
                input.set_value(formatted_body.clone(), window, cx);
            },
        );
        self.response_body_language = language;
    }

    pub(in crate::ui) fn sync_response_pane_from_selection(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(request_id) = self.shell.workspace_tree.selected_request_id() else {
            self.response_history_entries.clear();
            self.selected_response_history_index = None;
            self.clear_response_pane(window, cx);
            self.script_result = None;
            return;
        };

        self.response_history_entries =
            load_response_history_entries(&self.current_workspace_paths, request_id);
        self.selected_response_history_index =
            (!self.response_history_entries.is_empty()).then_some(0);
        if let Some(snapshot) = self.response_history_entries.first().map(|entry| {
            load_response_snapshot_for_history_entry(&self.current_workspace_paths, entry)
        }) {
            self.apply_response_snapshot(&snapshot, window, cx);
            self.restore_selected_request_response_scroll_offset(window, cx);
        } else {
            self.clear_response_pane(window, cx);
        }
        self.script_result = load_script_result(&self.current_workspace_paths, request_id);

        let (status, status_code, time, size) = response_summary_for_selected_request(
            Some(request_id),
            &self.request_execution_states,
            &self.response_status,
            self.response_status_code,
            &self.response_time,
            &self.response_size,
        );
        self.response_status = status;
        self.response_status_code = status_code;
        self.response_time = time;
        self.response_size = size;
    }

    pub(in crate::ui) fn status_code_in_color(status: Option<u16>, cx: &App) -> Hsla {
        match status {
            Some(200..=299) => cx.theme().success,
            Some(300..=399) => cx.theme().warning,
            Some(400..=599) => cx.theme().danger,
            Some(100..=199) => cx.theme().info,
            _ => cx.theme().muted_foreground,
        }
    }

    pub(in crate::ui) fn response_body_for_display(
        &self,
        body: &str,
        content_type: Option<&str>,
    ) -> String {
        if !self.shell.theme.auto_format_response {
            return body.to_string();
        }
        format_body_text(body, BodyFormatHint::FromContentType(content_type))
            .unwrap_or_else(|_| body.to_string())
    }

    pub(in crate::ui) fn format_response_body(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let current_text = self.response_body_editor.read(cx).value().to_string();
        if current_text.trim().is_empty() {
            return;
        }

        let formatted = format_body_text(
            &current_text,
            BodyFormatHint::FromContentType(self.response_content_type.as_deref()),
        )
        .unwrap_or_else(|_| current_text.clone());

        if formatted == current_text {
            return;
        }

        self.update_response_body_editor_with_scroll_persistence_suppressed(
            window,
            cx,
            |input, window, cx| Self::replace_editor_text(input, formatted, window, cx),
        );
        cx.notify();
    }
}
