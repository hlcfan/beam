use super::super::*;

impl BeamView {
    pub(in crate::ui) fn hydrate_request_from_selection(
        request: &mut RequestAuthoringState,
        shell: &AppShellState,
    ) {
        let Some(request_id) = shell.workspace_tree.selected_request_id() else {
            return;
        };

        let Some(pane_data) = shell.request_pane_data.get(&request_id) else {
            let selected_node = shell.workspace_tree.node(request_id).cloned();
            let Some(node) = selected_node else {
                return;
            };
            request.method = node.request_method.unwrap_or(HttpMethod::Get);
            request.url = node.request_url.unwrap_or_default();
            return;
        };

        request.method = pane_data.method;
        request.url = pane_data.url.clone();
        request.headers = pane_data.headers.clone();
        request.query_params = pane_data.query_params.clone();
        request.auth = pane_data.auth.clone();
        request.body = pane_data.body.clone();
        request.post_script = pane_data.post_script.clone();
        request.ensure_trailing_empty_row();
    }

    pub(in crate::ui) fn sync_request_editor_from_selection(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.refresh_active_request_cache();
        self.show_invalid_url_border = false;
        let active_tab = self.request.active_tab;
        self.request = RequestAuthoringState {
            active_tab,
            ..RequestAuthoringState::default()
        };
        Self::hydrate_request_from_selection(&mut self.request, &self.shell);
        self.request.ensure_trailing_empty_row();
        self.rebuild_request_param_inputs(window, cx);
        self.rebuild_request_header_inputs(window, cx);
        let next_script = self.request.post_script.clone().unwrap_or_default();
        let next_body_language = body_editor_language(&self.request.body);
        let selected_request_id = self.shell.workspace_tree.selected_request_id();
        if let Some(request_id) = selected_request_id {
            // Cache URL editor
            if let Some(cached_url) = self.request_url_editor_cache.get(&request_id) {
                self.url_input = cached_url.clone();
                self.resubscribe_request_url_editor(window, cx);
            } else {
                let url = Self::build_request_url_editor(&self.request, window, cx);
                self.url_input = url.clone();
                self.cache_url_editor(request_id, url);
                self.resubscribe_request_url_editor(window, cx);
            }
            // Cache body editor
            if let Some(cached_editor) = self.request_body_editor_cache.get(&request_id) {
                let cached_editor = cached_editor.clone();
                self.request_body_editor = cached_editor.clone();
                self.request_body_editor.update(cx, |input, cx| {
                    input.set_soft_wrap(self.shell.theme.wrap_body_editor, window, cx);
                });
                self.resubscribe_request_body_editor(window, cx);
            } else {
                let editor = Self::build_request_body_editor(
                    &self.request,
                    self.shell.theme.wrap_body_editor,
                    window,
                    cx,
                );
                self.request_body_editor = editor.clone();
                self.cache_body_editor(request_id, editor);
                self.resubscribe_request_body_editor(window, cx);
            }
        } else {
            let next_url = self.request.url.clone();
            self.url_input.update(cx, |input, cx| {
                input.set_value(next_url, window, cx);
            });
            self.request_body_editor.update(cx, |input, cx| {
                input.set_highlighter(next_body_language, cx);
                input.set_value(body_editor_text(&self.request.body), window, cx);
            });
            self.resubscribe_request_body_editor(window, cx);
        }
        self.post_script_editor.update(cx, |input, cx| {
            input.set_value(next_script, window, cx);
        });
        self.sync_request_auth_inputs(window, cx);
        self.sync_response_pane_from_selection(window, cx);
    }

    pub(in crate::ui) fn clear_request_param_inputs(&mut self) {
        self.request_param_name_inputs.clear();
        self.request_param_value_inputs.clear();
        self.request_param_input_subscriptions.clear();
    }

    pub(in crate::ui) fn clear_request_header_inputs(&mut self) {
        self.request_header_name_inputs.clear();
        self.request_header_value_inputs.clear();
        self.request_header_input_subscriptions.clear();
    }

    pub(in crate::ui) fn sync_request_auth_inputs(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.suppress_request_auth_change_events = true;

        let (bearer_token, basic_username, basic_password, api_key_name, api_key_value) =
            match &self.request.auth {
                AuthConfig::None => (
                    String::new(),
                    String::new(),
                    String::new(),
                    DEFAULT_API_KEY_HEADER_NAME.to_string(),
                    String::new(),
                ),
                AuthConfig::Bearer { token } => (
                    token.clone().unwrap_or_default(),
                    String::new(),
                    String::new(),
                    DEFAULT_API_KEY_HEADER_NAME.to_string(),
                    String::new(),
                ),
                AuthConfig::Basic { username, password } => (
                    String::new(),
                    username.clone().unwrap_or_default(),
                    password.clone().unwrap_or_default(),
                    DEFAULT_API_KEY_HEADER_NAME.to_string(),
                    String::new(),
                ),
                AuthConfig::ApiKey { key, value, .. } => (
                    String::new(),
                    String::new(),
                    String::new(),
                    key.clone()
                        .unwrap_or_else(|| DEFAULT_API_KEY_HEADER_NAME.to_string()),
                    value.clone().unwrap_or_default(),
                ),
            };

        self.request_auth_bearer_token_input
            .update(cx, |input, cx| {
                input.set_value(bearer_token, window, cx);
            });
        self.request_auth_basic_username_input
            .update(cx, |input, cx| {
                input.set_value(basic_username, window, cx);
            });
        self.request_auth_basic_password_input
            .update(cx, |input, cx| {
                input.set_value(basic_password, window, cx);
            });
        self.request_auth_api_key_name_input
            .update(cx, |input, cx| {
                input.set_value(api_key_name, window, cx);
            });
        self.request_auth_api_key_value_input
            .update(cx, |input, cx| {
                input.set_value(api_key_value, window, cx);
            });

        self.suppress_request_auth_change_events = false;
    }

    pub(in crate::ui) fn clear_request_auth_input_subscriptions(&mut self) {
        self.request_auth_input_subscriptions.clear();
    }

    pub(in crate::ui) fn rebuild_request_auth_input_subscriptions(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.clear_request_auth_input_subscriptions();

        let bearer_input = self.request_auth_bearer_token_input.clone();
        self.request_auth_input_subscriptions.push(cx.subscribe_in(
            &self.request_auth_bearer_token_input,
            window,
            move |this, _, ev: &InputEvent, _, cx| {
                if !matches!(ev, InputEvent::Change) || this.suppress_request_auth_change_events {
                    return;
                }
                if let AuthConfig::Bearer { token } = &mut this.request.auth {
                    let value = bearer_input.read(cx).value().to_string();
                    *token = (!value.trim().is_empty()).then_some(value);
                    this.schedule_request_save(cx);
                    cx.notify();
                }
            },
        ));

        let username_input = self.request_auth_basic_username_input.clone();
        self.request_auth_input_subscriptions.push(cx.subscribe_in(
            &self.request_auth_basic_username_input,
            window,
            move |this, _, ev: &InputEvent, _, cx| {
                if !matches!(ev, InputEvent::Change) || this.suppress_request_auth_change_events {
                    return;
                }
                if let AuthConfig::Basic { username, .. } = &mut this.request.auth {
                    let value = username_input.read(cx).value().to_string();
                    *username = (!value.trim().is_empty()).then_some(value);
                    this.schedule_request_save(cx);
                    cx.notify();
                }
            },
        ));

        let password_input = self.request_auth_basic_password_input.clone();
        self.request_auth_input_subscriptions.push(cx.subscribe_in(
            &self.request_auth_basic_password_input,
            window,
            move |this, _, ev: &InputEvent, _, cx| {
                if !matches!(ev, InputEvent::Change) || this.suppress_request_auth_change_events {
                    return;
                }
                if let AuthConfig::Basic { password, .. } = &mut this.request.auth {
                    let value = password_input.read(cx).value().to_string();
                    *password = (!value.trim().is_empty()).then_some(value);
                    this.schedule_request_save(cx);
                    cx.notify();
                }
            },
        ));

        let api_key_name_input = self.request_auth_api_key_name_input.clone();
        self.request_auth_input_subscriptions.push(cx.subscribe_in(
            &self.request_auth_api_key_name_input,
            window,
            move |this, _, ev: &InputEvent, _, cx| {
                if !matches!(ev, InputEvent::Change) || this.suppress_request_auth_change_events {
                    return;
                }
                if let AuthConfig::ApiKey { key, .. } = &mut this.request.auth {
                    let value = api_key_name_input.read(cx).value().to_string();
                    *key = if value.trim().is_empty() {
                        None
                    } else {
                        Some(value)
                    };
                    this.schedule_request_save(cx);
                    cx.notify();
                }
            },
        ));

        let api_key_value_input = self.request_auth_api_key_value_input.clone();
        self.request_auth_input_subscriptions.push(cx.subscribe_in(
            &self.request_auth_api_key_value_input,
            window,
            move |this, _, ev: &InputEvent, _, cx| {
                if !matches!(ev, InputEvent::Change) || this.suppress_request_auth_change_events {
                    return;
                }
                if let AuthConfig::ApiKey { value, .. } = &mut this.request.auth {
                    let next_value = api_key_value_input.read(cx).value().to_string();
                    *value = if next_value.trim().is_empty() {
                        None
                    } else {
                        Some(next_value)
                    };
                    this.schedule_request_save(cx);
                    cx.notify();
                }
            },
        ));
    }

    pub(in crate::ui) fn sync_selected_request_pane_data(
        &mut self,
    ) -> Option<(Ulid, RequestPaneData)> {
        let Some(request_id) = self.shell.workspace_tree.selected_request_id() else {
            return None;
        };
        let response_scroll_offset = self
            .shell
            .request_pane_data
            .get(&request_id)
            .map(|pane_data| pane_data.response_scroll_offset)
            .unwrap_or(point(px(0.), px(0.)));
        let pane_data = RequestPaneData {
            method: self.request.method,
            url: self.request.url.clone(),
            headers: self.request.headers.clone(),
            query_params: self.request.query_params.clone(),
            auth: self.request.auth.clone(),
            body: self.request.body.clone(),
            post_script: self.request.post_script.clone(),
            response_scroll_offset,
        };
        self.shell
            .request_pane_data
            .insert(request_id, pane_data.clone());
        Some((request_id, pane_data))
    }

    pub(in crate::ui) fn schedule_request_save_with_delay(
        &mut self,
        delay: Duration,
        cx: &mut Context<Self>,
    ) {
        if self.shell.workspace_tree.selected_request_id().is_none() {
            return;
        }
        self.pending_request_save_due_at = Some(Instant::now() + delay);
        if self.request_save_tick_scheduled {
            return;
        }
        self.request_save_tick_scheduled = true;
        self.schedule_request_save_tick(cx);
    }

    pub(in crate::ui) fn schedule_request_save(&mut self, cx: &mut Context<Self>) {
        self.schedule_request_save_with_delay(Duration::from_millis(350), cx);
    }

    pub(in crate::ui) fn schedule_request_save_tick(&self, cx: &mut Context<Self>) {
        let view = cx.entity();
        cx.spawn(async move |_, cx| {
            cx.background_executor()
                .spawn(async move {
                    std::thread::sleep(Duration::from_millis(25));
                })
                .await;
            let _ = view.update(cx, |this, cx| {
                this.process_pending_request_save(cx);
            });
        })
        .detach();
    }

    pub(in crate::ui) fn build_request_snapshot_for_save(
        &mut self,
        request_id: Ulid,
        pane_data: RequestPaneData,
    ) -> Result<RequestFile, String> {
        let mut request_file = self
            .active_request_cache
            .clone()
            .ok_or_else(|| "No active request cache available for save.".to_string())?;
        if request_file.meta.request_id != request_id {
            return Err(format!(
                "Active request cache mismatch: expected {request_id}, found {}.",
                request_file.meta.request_id
            ));
        }
        request_file.request.method = pane_data.method;
        request_file.request.url = pane_data.url;
        request_file.request.headers = pane_data.headers;
        request_file.request.query_params = pane_data.query_params;
        request_file.auth = pane_data.auth;
        request_file.body = pane_data.body;
        request_file.scripts.post_response = pane_data.post_script;
        request_file.meta.updated_at = Utc::now();
        self.active_request_cache = Some(request_file.clone());
        Ok(request_file)
    }

    pub(in crate::ui) fn process_pending_request_save(&mut self, cx: &mut Context<Self>) {
        if self.request_save_in_flight {
            self.request_save_tick_scheduled = false;
            return;
        }
        let Some(due_at) = self.pending_request_save_due_at else {
            self.request_save_tick_scheduled = false;
            return;
        };
        if Instant::now() < due_at {
            self.schedule_request_save_tick(cx);
            return;
        }
        self.pending_request_save_due_at = None;
        self.request_save_tick_scheduled = false;
        let Some((request_id, pane_data)) = self.sync_selected_request_pane_data() else {
            return;
        };
        self.refresh_active_request_cache();
        match self.build_request_snapshot_for_save(request_id, pane_data) {
            Ok(request_file) => {
                let command = AppCommand::SaveRequest {
                    request_file,
                    command_id: next_command_id(),
                };
                if let Err(error) = self.publish_app_command(command) {
                    log::error!("{error}");
                    if error.starts_with("Backpressure:") {
                        self.pending_request_save_due_at =
                            Some(Instant::now() + Duration::from_millis(100));
                    }
                }
            }
            Err(error) => log::error!("{error}"),
        }
        self.request_save_in_flight = false;
        if self.pending_request_save_due_at.is_some() && !self.request_save_tick_scheduled {
            self.request_save_tick_scheduled = true;
            self.schedule_request_save_tick(cx);
        }
    }

    pub(in crate::ui) fn rebuild_request_param_inputs(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.clear_request_param_inputs();
        for index in 0..self.request.query_params.len() {
            let param_name = self.request.query_params[index].name.clone();
            let param_value = self.request.query_params[index].value.clone();

            let key_input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Param key")
                    .default_value(param_name)
            });
            let value_input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Param value")
                    .default_value(param_value)
            });

            let key_input_handle = key_input.clone();
            let key_subscription = cx.subscribe_in(
                &key_input,
                window,
                move |this, _, ev: &InputEvent, window, cx| match ev {
                    InputEvent::Change => {
                        let name = key_input_handle.read(cx).value().to_string();
                        let value = this
                            .request
                            .query_params
                            .get(index)
                            .map(|param| param.value.clone())
                            .unwrap_or_default();
                        this.request.set_param_value(index, name, value);
                        if this.request_param_name_inputs.len() != this.request.query_params.len() {
                            this.rebuild_request_param_inputs(window, cx);
                        }
                        this.schedule_request_save(cx);
                        cx.notify();
                    }
                    InputEvent::Focus => {
                        if index + 1 == this.request.query_params.len() {
                            this.request
                                .query_params
                                .push(crate::models::QueryParamField {
                                    name: String::new(),
                                    value: String::new(),
                                    enabled: true,
                                    description: None,
                                });
                            this.rebuild_request_param_inputs(window, cx);
                            if let Some(input) = this.request_param_name_inputs.get(index).cloned()
                            {
                                input.update(cx, |state, cx| {
                                    state.focus(window, cx);
                                });
                            }
                            cx.notify();
                        }
                    }
                    _ => {}
                },
            );

            let value_input_handle = value_input.clone();
            let value_subscription = cx.subscribe_in(
                &value_input,
                window,
                move |this, _, ev: &InputEvent, window, cx| match ev {
                    InputEvent::Change => {
                        let name = this
                            .request
                            .query_params
                            .get(index)
                            .map(|param| param.name.clone())
                            .unwrap_or_default();
                        let value = value_input_handle.read(cx).value().to_string();
                        this.request.set_param_value(index, name, value);
                        if this.request_param_name_inputs.len() != this.request.query_params.len() {
                            this.rebuild_request_param_inputs(window, cx);
                        }
                        this.schedule_request_save(cx);
                        cx.notify();
                    }
                    InputEvent::Focus => {
                        if index + 1 == this.request.query_params.len() {
                            this.request
                                .query_params
                                .push(crate::models::QueryParamField {
                                    name: String::new(),
                                    value: String::new(),
                                    enabled: true,
                                    description: None,
                                });
                            this.rebuild_request_param_inputs(window, cx);
                            if let Some(input) = this.request_param_value_inputs.get(index).cloned()
                            {
                                input.update(cx, |state, cx| {
                                    state.focus(window, cx);
                                });
                            }
                            cx.notify();
                        }
                    }
                    _ => {}
                },
            );

            self.request_param_name_inputs.push(key_input);
            self.request_param_value_inputs.push(value_input);
            self.request_param_input_subscriptions
                .push(key_subscription);
            self.request_param_input_subscriptions
                .push(value_subscription);
        }
    }

    pub(in crate::ui) fn rebuild_request_header_inputs(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.clear_request_header_inputs();
        for index in 0..self.request.headers.len() {
            let header_name = self.request.headers[index].name.clone();
            let header_value = self.request.headers[index].value.clone();

            let key_input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Header key")
                    .default_value(header_name)
            });
            let value_input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Header value")
                    .default_value(header_value)
            });

            let key_input_handle = key_input.clone();
            let key_subscription = cx.subscribe_in(
                &key_input,
                window,
                move |this, _, ev: &InputEvent, window, cx| match ev {
                    InputEvent::Change => {
                        let name = key_input_handle.read(cx).value().to_string();
                        let value = this
                            .request
                            .headers
                            .get(index)
                            .map(|header| header.value.clone())
                            .unwrap_or_default();
                        this.request.set_header_value(index, name, value);
                        if this.request_header_name_inputs.len() != this.request.headers.len() {
                            this.rebuild_request_header_inputs(window, cx);
                        }
                        this.schedule_request_save(cx);
                        cx.notify();
                    }
                    InputEvent::Focus => {
                        if index + 1 == this.request.headers.len() {
                            this.request.headers.push(crate::models::HeaderField {
                                name: String::new(),
                                value: String::new(),
                                enabled: true,
                                description: None,
                            });
                            this.rebuild_request_header_inputs(window, cx);
                            if let Some(input) = this.request_header_name_inputs.get(index).cloned()
                            {
                                input.update(cx, |state, cx| {
                                    state.focus(window, cx);
                                });
                            }
                            cx.notify();
                        }
                    }
                    _ => {}
                },
            );

            let value_input_handle = value_input.clone();
            let value_subscription = cx.subscribe_in(
                &value_input,
                window,
                move |this, _, ev: &InputEvent, window, cx| match ev {
                    InputEvent::Change => {
                        let name = this
                            .request
                            .headers
                            .get(index)
                            .map(|header| header.name.clone())
                            .unwrap_or_default();
                        let value = value_input_handle.read(cx).value().to_string();
                        this.request.set_header_value(index, name, value);
                        if this.request_header_name_inputs.len() != this.request.headers.len() {
                            this.rebuild_request_header_inputs(window, cx);
                        }
                        this.schedule_request_save(cx);
                        cx.notify();
                    }
                    InputEvent::Focus => {
                        if index + 1 == this.request.headers.len() {
                            this.request.headers.push(crate::models::HeaderField {
                                name: String::new(),
                                value: String::new(),
                                enabled: true,
                                description: None,
                            });
                            this.rebuild_request_header_inputs(window, cx);
                            if let Some(input) =
                                this.request_header_value_inputs.get(index).cloned()
                            {
                                input.update(cx, |state, cx| {
                                    state.focus(window, cx);
                                });
                            }
                            cx.notify();
                        }
                    }
                    _ => {}
                },
            );

            self.request_header_name_inputs.push(key_input);
            self.request_header_value_inputs.push(value_input);
            self.request_header_input_subscriptions
                .push(key_subscription);
            self.request_header_input_subscriptions
                .push(value_subscription);
        }
    }

    pub(in crate::ui) fn delete_request_param_row(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if index >= self.request.query_params.len() {
            return;
        }
        if self.request.query_params.len() == 1 {
            if let Some(param) = self.request.query_params.get_mut(index) {
                param.name.clear();
                param.value.clear();
            }
        } else {
            self.request.query_params.remove(index);
        }
        self.rebuild_request_param_inputs(window, cx);
        self.schedule_request_save(cx);
        cx.notify();
    }

    pub(in crate::ui) fn delete_request_header_row(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if index >= self.request.headers.len() {
            return;
        }
        if self.request.headers.len() == 1 {
            if let Some(header) = self.request.headers.get_mut(index) {
                header.name.clear();
                header.value.clear();
            }
        } else {
            self.request.headers.remove(index);
        }
        self.rebuild_request_header_inputs(window, cx);
        self.schedule_request_save(cx);
        cx.notify();
    }
}
