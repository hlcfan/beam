pub(super) mod history;
pub(super) mod html_preview;
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

/// Response bodies at least this large are formatted off the UI thread.
const BACKGROUND_FORMAT_MIN_BYTES: usize = 256 * 1024;

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

    /// Show `snapshot` in the response pane. With `for_selection`, also restore the selected
    /// request's scroll offset and in-flight status. A large body is formatted off the UI thread
    /// while the current response stays on screen, and both are swapped when it is ready.
    pub(in crate::ui) fn apply_response_snapshot(
        &mut self,
        mut snapshot: StoredResponseSnapshot,
        for_selection: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.shell.theme.auto_format_response
            || snapshot.body.len() < BACKGROUND_FORMAT_MIN_BYTES
        {
            let body =
                self.response_body_for_display(&snapshot.body, snapshot.content_type.as_deref());
            self.show_response(snapshot, body, for_selection, window, cx);
            return;
        }

        let formatted = cx.background_spawn(async move {
            let body = std::mem::take(&mut snapshot.body);
            let body = formatted_response_body(&body, snapshot.content_type.as_deref());
            (snapshot, body)
        });
        self.response_body_format_task = Some(cx.spawn_in(window, async move |this, cx| {
            let (snapshot, body) = formatted.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.show_response(snapshot, body, for_selection, window, cx);
                cx.notify();
            });
        }));
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
        self.script_result = load_script_result(&self.current_workspace_paths, request_id);
        if let Some(snapshot) = self.response_history_entries.first().map(|entry| {
            load_response_snapshot_for_history_entry(&self.current_workspace_paths, entry)
        }) {
            self.apply_response_snapshot(snapshot, true, window, cx);
        } else {
            self.clear_response_pane(window, cx);
            self.apply_selected_request_execution_summary();
        }
    }

    /// Replace the response pane's status, headers, and body. `snapshot.body` is ignored in
    /// favor of `body`, which is already formatted for display.
    fn show_response(
        &mut self,
        snapshot: StoredResponseSnapshot,
        body: String,
        for_selection: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let language = response_body_editor_language(snapshot.content_type.as_deref());
        self.response_status = snapshot.status;
        self.response_status_code = snapshot.status_code;
        self.response_time = snapshot.time;
        self.response_size = snapshot.size;
        self.response_headers_raw = snapshot.headers_raw;
        self.response_content_type = snapshot.content_type;
        self.response_body_language = language;
        self.update_response_body_editor_with_scroll_persistence_suppressed(
            window,
            cx,
            |input, window, cx| {
                input.set_highlighter(language, cx);
                input.set_value(body, window, cx);
            },
        );
        if for_selection {
            self.restore_selected_request_response_scroll_offset(window, cx);
            self.apply_selected_request_execution_summary();
        }
    }

    /// A request that is still sending shows its in-flight status over its last response.
    fn apply_selected_request_execution_summary(&mut self) {
        let (status, status_code, time, size) = response_summary_for_selected_request(
            self.shell.workspace_tree.selected_request_id(),
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
        formatted_response_body(body, content_type)
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

/// `body` formatted for its content type, or unchanged when it cannot be formatted.
fn formatted_response_body(body: &str, content_type: Option<&str>) -> String {
    format_body_text(body, BodyFormatHint::FromContentType(content_type))
        .unwrap_or_else(|_| body.to_string())
}
