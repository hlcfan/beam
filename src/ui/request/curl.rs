use super::super::*;

impl BeamView {
    pub(in crate::ui) fn apply_curl_plan(request: &mut RequestAuthoringState, plan: CurlPlan) {
        request.method = plan.method;
        request.url = plan.url;
        request.headers = plan.headers;
        request.query_params = plan.query;
        request.body = plan.body;
        request.auth = plan.auth;
        request.ensure_trailing_empty_row();
    }

    pub(in crate::ui) fn import_curl_from_url_input(
        &mut self,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !is_curl(value) {
            return false;
        }

        let plan = match parse_curl(value) {
            Ok(plan) => plan,
            Err(error) => {
                window.push_notification(format!("Could not parse cURL command: {error}"), cx);
                return true;
            }
        };

        Self::apply_curl_plan(&mut self.request, plan);
        self.rebuild_request_param_inputs(window, cx);
        self.rebuild_request_header_inputs(window, cx);
        self.sync_request_auth_inputs(window, cx);

        let url = self.request.url.clone();
        self.url_input.update(cx, |input, cx| {
            input.set_value(url, window, cx);
            input.focus(window, cx);
        });

        let body_language = body_editor_language(&self.request.body);
        let body_text = body_editor_text(&self.request.body);
        self.request_body_editor.update(cx, |input, cx| {
            input.set_highlighter(body_language, cx);
            input.set_value(body_text, window, cx);
        });

        self.show_invalid_url_border = false;
        self.schedule_request_save(cx);
        cx.notify();
        true
    }

    pub(in crate::ui) fn resubscribe_request_url_editor(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let editor = self.url_input.clone();
        self.request_url_editor_change_sub = Some(cx.subscribe_in(
            &editor,
            window,
            move |this, _, ev: &InputEvent, window, cx| match ev {
                InputEvent::Change => {
                    let value = this.url_input.read(cx).value().to_string();
                    if this.import_curl_from_url_input(&value, window, cx) {
                        return;
                    }
                    this.request.url = value;
                    this.show_invalid_url_border = false;
                    this.schedule_request_save(cx);
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => {
                    let value = this.url_input.read(cx).value().to_string();
                    if this.import_curl_from_url_input(&value, window, cx) {
                        return;
                    }
                    this.request.url = value;
                    this.schedule_request_save(cx);
                    this.handle_send_or_cancel_action(window, cx);
                    cx.notify();
                }
                _ => {}
            },
        ));
    }
}

impl BeamView {
    pub(in crate::ui) fn quote_shell_arg(value: &str) -> String {
        format!("'{}'", value.replace('\'', "'\\''"))
    }

    pub(in crate::ui) fn build_curl_for_request(&self, request_id: Ulid) -> Option<String> {
        let pane = self.shell.request_pane_data.get(&request_id)?;
        let mut url = pane.url.clone();
        let query_pairs: Vec<String> = pane
            .query_params
            .iter()
            .filter(|param| param.enabled && !param.name.trim().is_empty())
            .map(|param| format!("{}={}", param.name.trim(), param.value.trim()))
            .collect();
        if !query_pairs.is_empty() {
            let joiner = if url.contains('?') { "&" } else { "?" };
            url.push_str(joiner);
            url.push_str(&query_pairs.join("&"));
        }

        let mut parts = vec![
            "curl".to_string(),
            "-X".to_string(),
            format!("{:?}", pane.method).to_uppercase(),
            Self::quote_shell_arg(&url),
        ];

        for header in pane
            .headers
            .iter()
            .filter(|header| header.enabled && !header.name.trim().is_empty())
        {
            parts.push("-H".to_string());
            parts.push(Self::quote_shell_arg(&format!(
                "{}: {}",
                header.name.trim(),
                header.value
            )));
        }

        let body_payload = match &pane.body {
            BodyConfig::None => None,
            BodyConfig::Raw { text, .. } | BodyConfig::Json { text } | BodyConfig::Xml { text } => {
                (!text.is_empty()).then_some(text.clone())
            }
            BodyConfig::Graphql {
                query,
                variables_json,
            } => {
                let mut payload = String::new();
                payload.push_str("{\"query\":");
                payload.push_str(&serde_json::to_string(query).ok()?);
                if let Some(variables) = variables_json.as_ref().filter(|v| !v.trim().is_empty()) {
                    payload.push_str(",\"variables\":");
                    if serde_json::from_str::<serde_json::Value>(variables).is_ok() {
                        payload.push_str(variables);
                    } else {
                        payload.push_str(&serde_json::to_string(variables).ok()?);
                    }
                }
                payload.push('}');
                Some(payload)
            }
            BodyConfig::FormUrlEncoded { fields } | BodyConfig::Multipart { fields } => {
                let payload = fields
                    .iter()
                    .filter(|field| !field.name.trim().is_empty())
                    .map(|field| format!("{}={}", field.name.trim(), field.value))
                    .collect::<Vec<_>>()
                    .join("&");
                (!payload.is_empty()).then_some(payload)
            }
        };

        if let Some(payload) = body_payload {
            parts.push("--data-raw".to_string());
            parts.push(Self::quote_shell_arg(&payload));
        }

        Some(parts.join(" "))
    }

    pub(in crate::ui) fn copy_request_as_curl_from_tree_node(
        &mut self,
        request_id: Ulid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(curl) = self.build_curl_for_request(request_id) else {
            window.push_notification("Unable to build cURL command for this request.", cx);
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(curl));
    }
}
