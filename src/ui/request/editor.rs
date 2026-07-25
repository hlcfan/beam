use super::super::*;

pub(in crate::ui) const BODY_EDITOR_CACHE_CAP: usize = 32;
pub(in crate::ui) const URL_EDITOR_CACHE_CAP: usize = 32;

impl BeamView {
    pub(in crate::ui) fn format_human_timestamp(timestamp: &str) -> String {
        chrono::DateTime::parse_from_rfc3339(timestamp)
            .map(|parsed| {
                parsed
                    .with_timezone(&Local)
                    .format("%Y-%m-%d %H:%M")
                    .to_string()
            })
            .unwrap_or_else(|_| timestamp.to_string())
    }

    pub(in crate::ui) fn format_human_time(timestamp: &str) -> String {
        chrono::DateTime::parse_from_rfc3339(timestamp)
            .map(|parsed| parsed.with_timezone(&Local).format("%H:%M:%S").to_string())
            .unwrap_or_else(|_| timestamp.to_string())
    }

    pub(in crate::ui) fn method_label(method: HttpMethod) -> &'static str {
        match method {
            HttpMethod::Get => "GET",
            HttpMethod::Post => "POST",
            HttpMethod::Put => "PUT",
            HttpMethod::Delete => "DELETE",
            HttpMethod::Patch => "PATCH",
            HttpMethod::Head => "HEAD",
            HttpMethod::Options => "OPTIONS",
            HttpMethod::Query => "QUERY",
        }
    }

    pub(in crate::ui) fn supported_http_methods() -> [HttpMethod; 8] {
        [
            HttpMethod::Get,
            HttpMethod::Post,
            HttpMethod::Put,
            HttpMethod::Delete,
            HttpMethod::Patch,
            HttpMethod::Head,
            HttpMethod::Options,
            HttpMethod::Query,
        ]
    }

    fn method_badge_colors(method: HttpMethod, cx: &App) -> (Hsla, Hsla) {
        match method {
            HttpMethod::Get => (
                cx.theme().success.opacity(1.0),
                cx.theme().success_foreground,
            ),
            HttpMethod::Post => (
                cx.theme().warning.opacity(1.0),
                cx.theme().warning_foreground,
            ),
            HttpMethod::Put | HttpMethod::Patch | HttpMethod::Query => {
                (cx.theme().info.opacity(1.0), cx.theme().info_foreground)
            }
            HttpMethod::Delete => (cx.theme().danger.opacity(1.0), cx.theme().danger_foreground),
            HttpMethod::Head | HttpMethod::Options => {
                (cx.theme().secondary, cx.theme().secondary_foreground)
            }
        }
    }

    pub(in crate::ui) fn render_method_badge(method: HttpMethod, cx: &App) -> Div {
        let (badge_bg, badge_text) = Self::method_badge_colors(method, cx);
        div()
            .px_1()
            .py(px(1.0))
            .rounded(px(4.0))
            .bg(badge_bg)
            .text_xs()
            .font_semibold()
            .text_color(badge_text)
            .child(format!("{method:?}").to_uppercase())
    }

    pub(in crate::ui) fn set_request_body_format(
        &mut self,
        format: RequestBodyFormat,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let current_text = self.request_body_editor.read(cx).value().to_string();
        self.request.body = body_from_format(format, current_text);
        self.request.active_tab = RequestTab::Body;

        let editor_text = body_editor_text(&self.request.body);
        let language = body_editor_language(&self.request.body);
        self.request_body_editor.update(cx, |input, cx| {
            input.set_highlighter(language, cx);
            input.set_value(editor_text, window, cx);
            input.focus(window, cx);
        });

        self.schedule_request_save(cx);
        cx.notify();
    }

    pub(in crate::ui) fn format_request_body(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = cx.entity();
        let body = self.request.body.clone();
        let current_text = self.request_body_editor.read(cx).value().to_string();
        let source_text = current_text.clone();

        cx.spawn_in(window, async move |_, cx| {
            let result =
                cx.background_executor()
                    .spawn(async move {
                        format_body_text(&current_text, BodyFormatHint::FromConfig(&body))
                    })
                    .await;

            let _ = view.update_in(cx, |this, window, cx| {
                let latest_text = this.request_body_editor.read(cx).value().to_string();
                if latest_text != source_text {
                    window.push_notification(
                        (
                            gpui_component::notification::NotificationType::Warning,
                            "Body changed while formatting. Please run Format again.",
                        ),
                        cx,
                    );
                    cx.notify();
                    return;
                }

                let formatted = match result {
                    Ok(formatted) => formatted,
                    Err(error) => {
                        window.push_notification(
                            (
                                gpui_component::notification::NotificationType::Error,
                                SharedString::from(format!(
                                    "Failed to format request body: {error}"
                                )),
                            ),
                            cx,
                        );
                        cx.notify();
                        return;
                    }
                };

                if formatted == latest_text {
                    return;
                }

                this.request.body = body_with_updated_text(&this.request.body, formatted.clone());
                this.request.active_tab = RequestTab::Body;
                this.request_body_editor.update(cx, |input, cx| {
                    Self::replace_editor_text(input, formatted, window, cx);
                });
                this.schedule_request_save(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(in crate::ui) fn replace_editor_text(
        input: &mut InputState,
        text: String,
        window: &mut Window,
        cx: &mut Context<InputState>,
    ) {
        let scroll_offset = input.scroll_offset();
        input.replace_all(text, window, cx);
        input.set_scroll_offset(scroll_offset, cx);
        input.focus(window, cx);
    }

    pub(in crate::ui) fn build_request_body_editor(
        request: &RequestAuthoringState,
        wrap_body_editor: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        let body_text = body_editor_text(&request.body);
        let body_language = body_editor_language(&request.body);
        cx.new(|cx| {
            InputState::new(window, cx)
                .code_editor(body_language)
                .line_number(true)
                .tab_size(TabSize {
                    tab_size: 2,
                    hard_tabs: false,
                })
                .soft_wrap(wrap_body_editor)
                .searchable(true)
                .placeholder("Enter request body...")
                .default_value(body_text)
        })
    }

    pub(in crate::ui) fn resubscribe_request_body_editor(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let editor = self.request_body_editor.clone();
        self.request_body_editor_change_sub =
            Some(
                cx.subscribe_in(&editor, window, move |this, _, ev: &InputEvent, _, cx| {
                    if !matches!(ev, InputEvent::Change) {
                        return;
                    }
                    let next_body_text = this.request_body_editor.read(cx).value().to_string();
                    this.request.body = body_with_updated_text(&this.request.body, next_body_text);
                    this.schedule_request_save(cx);
                    cx.notify();
                }),
            );
    }

    pub(in crate::ui) fn build_request_url_editor(
        request: &RequestAuthoringState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("https://api.example.com/resource")
                .default_value(request.url.clone())
        })
    }

    pub(in crate::ui) fn cache_url_editor(&mut self, request_id: Ulid, editor: Entity<InputState>) {
        Self::insert_editor_cache_entry(
            &mut self.request_url_editor_cache,
            &mut self.request_url_editor_cache_order,
            URL_EDITOR_CACHE_CAP,
            request_id,
            editor,
        );
    }

    pub(in crate::ui) fn cache_body_editor(
        &mut self,
        request_id: Ulid,
        editor: Entity<InputState>,
    ) {
        Self::insert_editor_cache_entry(
            &mut self.request_body_editor_cache,
            &mut self.request_body_editor_cache_order,
            BODY_EDITOR_CACHE_CAP,
            request_id,
            editor,
        );
    }

    pub(in crate::ui) fn insert_editor_cache_entry(
        cache: &mut HashMap<Ulid, Entity<InputState>>,
        order: &mut Vec<Ulid>,
        cap: usize,
        request_id: Ulid,
        editor: Entity<InputState>,
    ) {
        cache.insert(request_id, editor);
        order.push(request_id);
        if cache.len() > cap
            && let Some(oldest) = order.first().copied()
        {
            cache.remove(&oldest);
            order.remove(0);
        }
    }

    pub(in crate::ui) fn render_url_bar(&self, cx: &mut Context<Self>) -> Div {
        let send_state = self.send_button_state_for_view();
        let send_disabled = matches!(
            send_state,
            SendButtonState::Disabled(SendDisabledReason::EmptyUrl)
        );
        let send_icon_path = if matches!(send_state, SendButtonState::Sending) {
            "icons/cancel.svg"
        } else {
            "icons/send.svg"
        };
        let highlight_invalid_url = self.show_invalid_url_border;
        let current_method = self.request.method;
        let url_has_selection = !self.url_input.read(cx).selected_range().is_empty();
        let selected_environment_id = self.selected_environment_id_for_view();
        let environment_options = self.active_environment_options();
        let env_label = self.selected_environment_label();
        let method_view = cx.entity();
        let environment_view = method_view.clone();

        h_flex()
            .items_center()
            .gap_2()
            .w_full()
            .child(
                div()
                    .flex_1()
                    .rounded(px(10.0))
                    .border_1()
                    .border_color(if highlight_invalid_url {
                        cx.theme().danger
                    } else {
                        cx.theme().border
                    })
                    .bg(cx.theme().background)
                    .p_1()
                    .child(
                        h_flex()
                            .items_center()
                            .gap_1()
                            .w_full()
                            .child(
                                Button::new("request-method-dropdown")
                                    .ghost()
                                    .small()
                                    .px_1()
                                    .cursor_pointer()
                                    .label(Self::method_label(self.request.method))
                                    .dropdown_menu(move |menu, window, _| {
                                        let mut menu = menu.min_w(px(120.));
                                        for method in Self::supported_http_methods() {
                                            let checked = method == current_method;
                                            menu = menu.item(
                                                PopupMenuItem::element(move |_, _| {
                                                    div()
                                                        .w_full()
                                                        .cursor_pointer()
                                                        .child(Self::method_label(method))
                                                })
                                                .checked(checked)
                                                .on_click(window.listener_for(
                                                    &method_view,
                                                    move |this, _, _, cx| {
                                                        this.request.method = method;
                                                        this.schedule_request_save(cx);
                                                        cx.notify();
                                                    },
                                                )),
                                            );
                                        }
                                        menu
                                    }),
                            )
                            .child({
                                let url_entity = self.url_input.clone();
                                div()
                                    .id("env-hover-url")
                                    .flex_1()
                                    .on_mouse_move(cx.listener(
                                        move |this, event: &MouseMoveEvent, _, cx| {
                                            this.update_env_var_hover_for_input(
                                                &url_entity,
                                                event.position,
                                                cx,
                                            );
                                        },
                                    ))
                                    .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                        if !hovered {
                                            this.clear_env_var_hover(cx);
                                        }
                                    }))
                                    .child(
                                        Input::new(&self.url_input)
                                            .flex_1()
                                            .small()
                                            .appearance(false)
                                            .context_menu({
                                                move |menu, _, cx| {
                                                    build_text_edit_context_menu(
                                                        menu,
                                                        url_has_selection,
                                                        cx.theme().muted_foreground,
                                                    )
                                                }
                                            }),
                                    )
                            })
                            .child(
                                Button::new("send-request")
                                    .ghost()
                                    .small()
                                    .h(px(28.0))
                                    .min_w(px(36.0))
                                    .rounded(px(6.0))
                                    .cursor_pointer()
                                    .disabled(send_disabled)
                                    .icon(
                                        Icon::default()
                                            .path(send_icon_path)
                                            .size(px(16.0))
                                            .text_color(cx.theme().muted_foreground),
                                    )
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.handle_send_or_cancel_action(window, cx);
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
            .child(
                Button::new("environment-dropdown")
                    .ghost()
                    .small()
                    .h(px(36.0))
                    .min_w(px(180.0))
                    .px_2()
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().background)
                    .cursor_pointer()
                    .justify_start()
                    .child(div().w_full().child(env_label))
                    .dropdown_menu(move |menu, window, _| {
                        let mut menu = menu.min_w(px(220.));
                        let environment_view = environment_view.clone();

                        let no_env_selected = selected_environment_id.is_none_or(|selected_id| {
                            !environment_options
                                .iter()
                                .any(|(environment_id, _)| *environment_id == selected_id)
                        });
                        menu = menu.item(
                            PopupMenuItem::element(move |_, _| {
                                div().w_full().cursor_pointer().child("No environment")
                            })
                            .checked(no_env_selected)
                            .on_click(window.listener_for(
                                &environment_view,
                                move |this, _, window, cx| {
                                    this.clear_selected_environment_for_view();
                                    if let Err(error) = this.persist_environment_selection_state() {
                                        window.push_notification(error, cx);
                                    }
                                    cx.notify();
                                },
                            )),
                        );

                        for (environment_id, label) in environment_options.clone() {
                            let checked = Some(environment_id) == selected_environment_id;
                            let item_view = environment_view.clone();
                            menu = menu.item(
                                PopupMenuItem::element(move |_, _| {
                                    div().w_full().cursor_pointer().child(label.clone())
                                })
                                .checked(checked)
                                .on_click(window.listener_for(
                                    &item_view,
                                    move |this, _, window, cx| {
                                        this.set_selected_environment_for_view(environment_id);
                                        if let Err(error) =
                                            this.persist_environment_selection_state()
                                        {
                                            window.push_notification(error, cx);
                                        }
                                        cx.notify();
                                    },
                                )),
                            );
                        }
                        menu = menu.separator().item(
                            PopupMenuItem::element(move |_, _| {
                                div().w_full().cursor_pointer().child("Manage environment")
                            })
                            .on_click(window.listener_for(
                                &environment_view,
                                move |this, _, window, cx| {
                                    this.open_environment_manager(window, cx);
                                },
                            )),
                        );
                        menu
                    }),
            )
    }

    pub(in crate::ui) fn render_request_tabs(&self, cx: &mut Context<Self>) -> Div {
        let mut tabs = h_flex().items_center().gap_1().w_full();
        let body_tab_view = cx.entity();
        let current_body_format = body_format_from_config(&self.request.body);
        let body_tab_label = body_tab_label(current_body_format);
        let body_tab_button = Button::new("tab-Body")
            .small()
            .ghost()
            .cursor_pointer()
            .selected(self.request.active_tab == RequestTab::Body)
            .child(
                h_flex().items_center().gap_1().child(body_tab_label).child(
                    Icon::default()
                        .path("icons/chevron-down.svg")
                        .size(px(12.0))
                        .text_color(cx.theme().muted_foreground),
                ),
            );
        if self.request.active_tab == RequestTab::Body {
            tabs = tabs.child(body_tab_button.dropdown_menu(move |menu, window, _| {
                let mut menu = menu.min_w(px(180.0));
                for format in supported_body_formats() {
                    let item_label = body_format_label(format);
                    let checked = format == current_body_format;
                    let format_view = body_tab_view.clone();
                    menu = menu.item(
                        PopupMenuItem::element(move |_, _| {
                            div().w_full().cursor_pointer().child(item_label)
                        })
                        .checked(checked)
                        .on_click(window.listener_for(
                            &format_view,
                            move |this, _, window, cx| {
                                this.set_request_body_format(format, window, cx);
                            },
                        )),
                    );
                }
                menu
            }));
        } else {
            tabs = tabs.child(body_tab_button.on_click(cx.listener(|this, _, window, cx| {
                this.request.active_tab = RequestTab::Body;
                this.request_body_editor.update(cx, |state, cx| {
                    state.focus(window, cx);
                });
            })));
        }

        let tab_specs = [
            (RequestTab::Params, "Params"),
            (RequestTab::Headers, "Headers"),
            (RequestTab::Auth, "Auth"),
        ];

        for (tab, label) in tab_specs {
            tabs = tabs.child(
                Button::new(format!("tab-{label}"))
                    .small()
                    .ghost()
                    .cursor_pointer()
                    .selected(self.request.active_tab == tab)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.request.active_tab = tab;
                        if tab == RequestTab::Body {
                            this.request_body_editor.update(cx, |state, cx| {
                                state.focus(window, cx);
                            });
                        } else if tab == RequestTab::PostScript {
                            this.post_script_editor.update(cx, |state, cx| {
                                state.focus(window, cx);
                            });
                        }
                    }))
                    .label(label),
            );
        }

        let has_script = self
            .request
            .post_script
            .as_ref()
            .is_some_and(|value| !value.trim().is_empty());
        let indicator_color = if !has_script {
            None
        } else if let Some(result) = self.script_result.as_ref() {
            Some(if result.success {
                cx.theme().success
            } else {
                cx.theme().danger
            })
        } else {
            Some(cx.theme().muted_foreground)
        };
        let post_script_label = if let Some(color) = indicator_color {
            h_flex()
                .items_center()
                .gap_1()
                .child("Script")
                .child(div().w(px(6.0)).h(px(6.0)).rounded_full().bg(color))
        } else {
            h_flex().items_center().gap_1().child("Script")
        };
        let post_script_help_trigger = HoverCard::new("tab-Post Script-help")
            .anchor(gpui::Anchor::BottomLeft)
            .open_delay(Duration::from_millis(100))
            .close_delay(Duration::from_millis(150))
            .trigger(
                div()
                    .id("tab-Post Script-help-trigger")
                    .flex()
                    .flex_shrink_0()
                    .w(px(18.0))
                    .h(px(18.0))
                    .rounded_full()
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().secondary)
                    .cursor_pointer()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .text_xs()
                            .font_semibold()
                            .line_height(relative(1.0))
                            .text_color(cx.theme().secondary_foreground)
                            .child("i"),
                    ),
            )
            .child(
                div().w(px(360.0)).h(px(520.0)).overflow_hidden().child(
                    div().size_full().overflow_y_scrollbar().child(
                        markdown(POST_SCRIPT_API_HELP_MARKDOWN)
                            .w_full()
                            .text_sm()
                            .selectable(true),
                    ),
                ),
            );
        tabs = tabs.child(
            h_flex()
                .items_center()
                .gap_1()
                .child(
                    Button::new("tab-Post Script")
                        .small()
                        .ghost()
                        .cursor_pointer()
                        .selected(self.request.active_tab == RequestTab::PostScript)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.request.active_tab = RequestTab::PostScript;
                            this.post_script_editor.update(cx, |state, cx| {
                                state.focus(window, cx);
                            });
                        }))
                        .child(post_script_label),
                )
                .child(post_script_help_trigger),
        );

        tabs
    }

    pub(in crate::ui) fn render_request_editor_surface(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match self.request.active_tab {
            RequestTab::Body => {
                if self.request.method == HttpMethod::Get {
                    return v_flex()
                        .h_full()
                        .w_full()
                        .child(div().flex_1())
                        .child(
                            h_flex()
                                .w_full()
                                .justify_between()
                                .child(div().flex_1())
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("No Body"),
                                )
                                .child(div().flex_1()),
                        )
                        .child(div().flex_1())
                        .into_any_element();
                }

                let request_body_has_selection = !self
                    .request_body_editor
                    .read(cx)
                    .selected_range()
                    .is_empty();
                {
                    let request_body_editor = self.request_body_editor.clone();
                    div()
                        .id("env-hover-request-body")
                        .h_full()
                        .w_full()
                        .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
                            this.update_env_var_hover_for_input(
                                &request_body_editor,
                                event.position,
                                cx,
                            );
                        }))
                        .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                            if !hovered {
                                this.clear_env_var_hover(cx);
                            }
                        }))
                        .child(
                            Input::new(&self.request_body_editor)
                                .h_full()
                                .p_0()
                                .border_0()
                                .focus_bordered(false)
                                .font_family(cx.theme().mono_font_family.clone())
                                .text_size(cx.theme().mono_font_size)
                                .context_menu(move |menu, _window, cx| {
                                    let muted_foreground = cx.theme().muted_foreground;
                                    let menu = menu
                                        .menu_with_icon(
                                            "Format",
                                            Icon::default().path("icons/indent.svg"),
                                            Box::new(FormatRequestBody),
                                        )
                                        .separator();
                                    build_text_edit_context_menu_with_find(
                                        menu,
                                        request_body_has_selection,
                                        muted_foreground,
                                    )
                                }),
                        )
                        .into_any_element()
                }
            }
            RequestTab::PostScript => self.render_post_script_editor_and_results(window, cx),
            RequestTab::Params => {
                let mut table = v_flex().h_full().w_full().gap_2();

                for (index, param) in self.request.query_params.iter().enumerate() {
                    let key_input = self.request_param_name_inputs[index].clone();
                    let value_input = self.request_param_value_inputs[index].clone();
                    let key_has_selection = !key_input.read(cx).selected_range().is_empty();
                    let value_has_selection = !value_input.read(cx).selected_range().is_empty();
                    table = table.child(
                        h_flex()
                            .w_full()
                            .items_center()
                            .gap_2()
                            .px_2()
                            .py_1()
                            .rounded(px(6.0))
                            .border_1()
                            .border_color(cx.theme().border)
                            .child(
                                div().w(px(28.0)).child(
                                    gpui_component::checkbox::Checkbox::new(format!(
                                        "request-param-enabled-{index}"
                                    ))
                                    .small()
                                    .checked(param.enabled)
                                    .on_click(cx.listener(
                                        move |this, checked: &bool, _, cx| {
                                            if let Some(item) =
                                                this.request.query_params.get_mut(index)
                                            {
                                                item.enabled = *checked;
                                                this.schedule_request_save(cx);
                                                cx.notify();
                                            }
                                        },
                                    )),
                                ),
                            )
                            .child({
                                let key_entity = key_input.clone();
                                div()
                                    .id(("env-hover-param-key", index as u64))
                                    .flex_1()
                                    .on_mouse_move(cx.listener(
                                        move |this, event: &MouseMoveEvent, _, cx| {
                                            this.update_env_var_hover_for_input(
                                                &key_entity,
                                                event.position,
                                                cx,
                                            );
                                        },
                                    ))
                                    .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                        if !hovered {
                                            this.clear_env_var_hover(cx);
                                        }
                                    }))
                                    .child(
                                        Input::new(&key_input)
                                            .small()
                                            .w_full()
                                            .appearance(false)
                                            .context_menu({
                                                move |menu, _, cx| {
                                                    build_text_edit_context_menu(
                                                        menu,
                                                        key_has_selection,
                                                        cx.theme().muted_foreground,
                                                    )
                                                }
                                            }),
                                    )
                            })
                            .child({
                                let value_entity = value_input.clone();
                                div()
                                    .id(("env-hover-param-value", index as u64))
                                    .flex_1()
                                    .on_mouse_move(cx.listener(
                                        move |this, event: &MouseMoveEvent, _, cx| {
                                            this.update_env_var_hover_for_input(
                                                &value_entity,
                                                event.position,
                                                cx,
                                            );
                                        },
                                    ))
                                    .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                        if !hovered {
                                            this.clear_env_var_hover(cx);
                                        }
                                    }))
                                    .child(
                                        Input::new(&value_input)
                                            .small()
                                            .w_full()
                                            .appearance(false)
                                            .context_menu({
                                                move |menu, _, cx| {
                                                    build_text_edit_context_menu(
                                                        menu,
                                                        value_has_selection,
                                                        cx.theme().muted_foreground,
                                                    )
                                                }
                                            }),
                                    )
                            })
                            .child(
                                div().w(px(28.0)).child(
                                    Button::new(format!("delete-request-param-{index}"))
                                        .small()
                                        .ghost()
                                        .cursor_pointer()
                                        .icon(
                                            Icon::default()
                                                .path("icons/delete.svg")
                                                .size(px(14.0))
                                                .text_color(cx.theme().muted_foreground),
                                        )
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.delete_request_param_row(index, window, cx);
                                        })),
                                ),
                            ),
                    );
                }

                table.into_any_element()
            }
            RequestTab::Headers => {
                let mut table = v_flex().h_full().w_full().gap_2();

                for (index, header) in self.request.headers.iter().enumerate() {
                    let key_input = self.request_header_name_inputs[index].clone();
                    let value_input = self.request_header_value_inputs[index].clone();
                    let key_has_selection = !key_input.read(cx).selected_range().is_empty();
                    let value_has_selection = !value_input.read(cx).selected_range().is_empty();
                    table = table.child(
                        h_flex()
                            .w_full()
                            .items_center()
                            .gap_2()
                            .px_2()
                            .py_1()
                            .rounded(px(6.0))
                            .border_1()
                            .border_color(cx.theme().border)
                            .child(
                                div().w(px(28.0)).child(
                                    gpui_component::checkbox::Checkbox::new(format!(
                                        "request-header-enabled-{index}"
                                    ))
                                    .small()
                                    .checked(header.enabled)
                                    .on_click(cx.listener(
                                        move |this, checked: &bool, _, cx| {
                                            if let Some(item) = this.request.headers.get_mut(index)
                                            {
                                                item.enabled = *checked;
                                                this.schedule_request_save(cx);
                                                cx.notify();
                                            }
                                        },
                                    )),
                                ),
                            )
                            .child({
                                let key_entity = key_input.clone();
                                div()
                                    .id(("env-hover-header-key", index as u64))
                                    .flex_1()
                                    .on_mouse_move(cx.listener(
                                        move |this, event: &MouseMoveEvent, _, cx| {
                                            this.update_env_var_hover_for_input(
                                                &key_entity,
                                                event.position,
                                                cx,
                                            );
                                        },
                                    ))
                                    .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                        if !hovered {
                                            this.clear_env_var_hover(cx);
                                        }
                                    }))
                                    .child(
                                        Input::new(&key_input)
                                            .small()
                                            .w_full()
                                            .appearance(false)
                                            .context_menu({
                                                move |menu, _, cx| {
                                                    build_text_edit_context_menu(
                                                        menu,
                                                        key_has_selection,
                                                        cx.theme().muted_foreground,
                                                    )
                                                }
                                            }),
                                    )
                            })
                            .child({
                                let value_entity = value_input.clone();
                                div()
                                    .id(("env-hover-header-value", index as u64))
                                    .flex_1()
                                    .on_mouse_move(cx.listener(
                                        move |this, event: &MouseMoveEvent, _, cx| {
                                            this.update_env_var_hover_for_input(
                                                &value_entity,
                                                event.position,
                                                cx,
                                            );
                                        },
                                    ))
                                    .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                        if !hovered {
                                            this.clear_env_var_hover(cx);
                                        }
                                    }))
                                    .child(
                                        Input::new(&value_input)
                                            .small()
                                            .w_full()
                                            .appearance(false)
                                            .context_menu({
                                                move |menu, _, cx| {
                                                    build_text_edit_context_menu(
                                                        menu,
                                                        value_has_selection,
                                                        cx.theme().muted_foreground,
                                                    )
                                                }
                                            }),
                                    )
                            })
                            .child(
                                div().w(px(28.0)).child(
                                    Button::new(format!("delete-request-header-{index}"))
                                        .small()
                                        .ghost()
                                        .cursor_pointer()
                                        .icon(
                                            Icon::default()
                                                .path("icons/delete.svg")
                                                .size(px(14.0))
                                                .text_color(cx.theme().muted_foreground),
                                        )
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.delete_request_header_row(index, window, cx);
                                        })),
                                ),
                            ),
                    );
                }

                table.into_any_element()
            }
            RequestTab::Auth => div()
                .h_full()
                .w_full()
                .gap_3()
                .child({
                    let bearer_input = self.request_auth_bearer_token_input.clone();
                    let basic_username_input = self.request_auth_basic_username_input.clone();
                    let basic_password_input = self.request_auth_basic_password_input.clone();
                    let api_key_name_input = self.request_auth_api_key_name_input.clone();
                    let api_key_value_input = self.request_auth_api_key_value_input.clone();
                    let is_none = matches!(self.request.auth, AuthConfig::None);
                    let is_bearer = matches!(self.request.auth, AuthConfig::Bearer { .. });
                    let is_basic = matches!(self.request.auth, AuthConfig::Basic { .. });
                    let is_api_key = matches!(self.request.auth, AuthConfig::ApiKey { .. });

                    h_flex()
                        .items_center()
                        .gap_1()
                        .w_full()
                        .child(
                            Button::new("auth-mode-none")
                                .small()
                                .ghost()
                                .cursor_pointer()
                                .selected(is_none)
                                .label("None")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.request.auth = AuthConfig::None;
                                    this.schedule_request_save(cx);
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("auth-mode-bearer")
                                .small()
                                .ghost()
                                .cursor_pointer()
                                .selected(is_bearer)
                                .label("Bearer Token")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let token = bearer_input.read(cx).value().to_string();
                                    this.request.auth = AuthConfig::Bearer {
                                        token: (!token.trim().is_empty()).then_some(token),
                                    };
                                    this.schedule_request_save(cx);
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("auth-mode-basic")
                                .small()
                                .ghost()
                                .cursor_pointer()
                                .selected(is_basic)
                                .label("Basic Auth")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let username =
                                        basic_username_input.read(cx).value().to_string();
                                    let password =
                                        basic_password_input.read(cx).value().to_string();
                                    this.request.auth = AuthConfig::Basic {
                                        username: (!username.trim().is_empty()).then_some(username),
                                        password: (!password.trim().is_empty()).then_some(password),
                                    };
                                    this.schedule_request_save(cx);
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("auth-mode-apikey")
                                .small()
                                .ghost()
                                .cursor_pointer()
                                .selected(is_api_key)
                                .label("API Key")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let key = api_key_name_input.read(cx).value().to_string();
                                    let value = api_key_value_input.read(cx).value().to_string();
                                    this.request.auth = AuthConfig::ApiKey {
                                        key: if key.trim().is_empty() {
                                            Some(DEFAULT_API_KEY_HEADER_NAME.to_string())
                                        } else {
                                            Some(key)
                                        },
                                        value: (!value.trim().is_empty()).then_some(value),
                                        location: crate::models::ApiKeyLocation::Header,
                                    };
                                    this.schedule_request_save(cx);
                                    cx.notify();
                                })),
                        )
                })
                .child({
                    let bearer_has_selection = !self
                        .request_auth_bearer_token_input
                        .read(cx)
                        .selected_range()
                        .is_empty();
                    let basic_username_has_selection = !self
                        .request_auth_basic_username_input
                        .read(cx)
                        .selected_range()
                        .is_empty();
                    let basic_password_has_selection = !self
                        .request_auth_basic_password_input
                        .read(cx)
                        .selected_range()
                        .is_empty();
                    let api_key_value_has_selection = !self
                        .request_auth_api_key_value_input
                        .read(cx)
                        .selected_range()
                        .is_empty();
                    let api_key_name_has_selection = !self
                        .request_auth_api_key_name_input
                        .read(cx)
                        .selected_range()
                        .is_empty();
                    match &self.request.auth {
                        AuthConfig::None => div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child("No auth header will be added.")
                            .into_any_element(),
                        AuthConfig::Bearer { .. } => {
                            let bearer_entity = self.request_auth_bearer_token_input.clone();
                            v_flex()
                                .w_full()
                                .gap_2()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("Token"),
                                )
                                .child(
                                    div()
                                        .id("env-hover-auth-bearer")
                                        .on_mouse_move(cx.listener(
                                            move |this, event: &MouseMoveEvent, _, cx| {
                                                this.update_env_var_hover_for_input(
                                                    &bearer_entity,
                                                    event.position,
                                                    cx,
                                                );
                                            },
                                        ))
                                        .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                            if !hovered {
                                                this.clear_env_var_hover(cx);
                                            }
                                        }))
                                        .child(
                                            Input::new(&self.request_auth_bearer_token_input)
                                                .small()
                                                .w_full()
                                                .context_menu({
                                                    move |menu, _, cx| {
                                                        build_text_edit_context_menu(
                                                            menu,
                                                            bearer_has_selection,
                                                            cx.theme().muted_foreground,
                                                        )
                                                    }
                                                }),
                                        ),
                                )
                                .into_any_element()
                        }
                        AuthConfig::Basic { .. } => {
                            let username_entity = self.request_auth_basic_username_input.clone();
                            let password_entity = self.request_auth_basic_password_input.clone();
                            v_flex()
                                .w_full()
                                .gap_2()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("Username"),
                                )
                                .child(
                                    div()
                                        .id("env-hover-auth-basic-username")
                                        .on_mouse_move(cx.listener(
                                            move |this, event: &MouseMoveEvent, _, cx| {
                                                this.update_env_var_hover_for_input(
                                                    &username_entity,
                                                    event.position,
                                                    cx,
                                                );
                                            },
                                        ))
                                        .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                            if !hovered {
                                                this.clear_env_var_hover(cx);
                                            }
                                        }))
                                        .child(
                                            Input::new(&self.request_auth_basic_username_input)
                                                .small()
                                                .w_full()
                                                .context_menu({
                                                    move |menu, _, cx| {
                                                        build_text_edit_context_menu(
                                                            menu,
                                                            basic_username_has_selection,
                                                            cx.theme().muted_foreground,
                                                        )
                                                    }
                                                }),
                                        ),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("Password"),
                                )
                                .child(
                                    div()
                                        .id("env-hover-auth-basic-password")
                                        .on_mouse_move(cx.listener(
                                            move |this, event: &MouseMoveEvent, _, cx| {
                                                this.update_env_var_hover_for_input(
                                                    &password_entity,
                                                    event.position,
                                                    cx,
                                                );
                                            },
                                        ))
                                        .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                            if !hovered {
                                                this.clear_env_var_hover(cx);
                                            }
                                        }))
                                        .child(
                                            Input::new(&self.request_auth_basic_password_input)
                                                .small()
                                                .w_full()
                                                .context_menu({
                                                    move |menu, _, cx| {
                                                        build_text_edit_context_menu(
                                                            menu,
                                                            basic_password_has_selection,
                                                            cx.theme().muted_foreground,
                                                        )
                                                    }
                                                }),
                                        ),
                                )
                                .into_any_element()
                        }
                        AuthConfig::ApiKey { location, .. } => {
                            let using_header =
                                matches!(location, crate::models::ApiKeyLocation::Header);
                            let using_query =
                                matches!(location, crate::models::ApiKeyLocation::Query);
                            v_flex()
                                .w_full()
                                .gap_2()
                                .child(
                                    h_flex()
                                        .items_center()
                                        .gap_1()
                                        .child(
                                            Button::new("auth-apikey-location-header")
                                                .small()
                                                .ghost()
                                                .cursor_pointer()
                                                .selected(using_header)
                                                .label("Header")
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    if let AuthConfig::ApiKey {
                                                        key, value, ..
                                                    } = &this.request.auth
                                                    {
                                                        this.request.auth = AuthConfig::ApiKey {
                                                        key: key.clone(),
                                                        value: value.clone(),
                                                        location:
                                                            crate::models::ApiKeyLocation::Header,
                                                    };
                                                        this.schedule_request_save(cx);
                                                        cx.notify();
                                                    }
                                                })),
                                        )
                                        .child(
                                            Button::new("auth-apikey-location-query")
                                                .small()
                                                .ghost()
                                                .cursor_pointer()
                                                .selected(using_query)
                                                .label("Query")
                                                .on_click(
                                                    cx.listener(move |this, _, _, cx| {
                                                        if let AuthConfig::ApiKey {
                                                            key,
                                                            value,
                                                            ..
                                                        } = &this.request.auth
                                                        {
                                                            this.request.auth = AuthConfig::ApiKey {
                                                        key: key.clone(),
                                                        value: value.clone(),
                                                        location:
                                                            crate::models::ApiKeyLocation::Query,
                                                    };
                                                            this.schedule_request_save(cx);
                                                            cx.notify();
                                                        }
                                                    }),
                                                ),
                                        ),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("Key Value"),
                                )
                                .child({
                                    let api_key_value_entity =
                                        self.request_auth_api_key_value_input.clone();
                                    div()
                                        .id("env-hover-auth-apikey-value")
                                        .on_mouse_move(cx.listener(
                                            move |this, event: &MouseMoveEvent, _, cx| {
                                                this.update_env_var_hover_for_input(
                                                    &api_key_value_entity,
                                                    event.position,
                                                    cx,
                                                );
                                            },
                                        ))
                                        .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                            if !hovered {
                                                this.clear_env_var_hover(cx);
                                            }
                                        }))
                                        .child(
                                            Input::new(&self.request_auth_api_key_value_input)
                                                .small()
                                                .w_full()
                                                .context_menu({
                                                    move |menu, _, cx| {
                                                        build_text_edit_context_menu(
                                                            menu,
                                                            api_key_value_has_selection,
                                                            cx.theme().muted_foreground,
                                                        )
                                                    }
                                                }),
                                        )
                                })
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("Header / Query Name"),
                                )
                                .child({
                                    let api_key_name_entity =
                                        self.request_auth_api_key_name_input.clone();
                                    div()
                                        .id("env-hover-auth-apikey-name")
                                        .on_mouse_move(cx.listener(
                                            move |this, event: &MouseMoveEvent, _, cx| {
                                                this.update_env_var_hover_for_input(
                                                    &api_key_name_entity,
                                                    event.position,
                                                    cx,
                                                );
                                            },
                                        ))
                                        .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                            if !hovered {
                                                this.clear_env_var_hover(cx);
                                            }
                                        }))
                                        .child(
                                            Input::new(&self.request_auth_api_key_name_input)
                                                .small()
                                                .w_full()
                                                .context_menu({
                                                    move |menu, _, cx| {
                                                        build_text_edit_context_menu(
                                                            menu,
                                                            api_key_name_has_selection,
                                                            cx.theme().muted_foreground,
                                                        )
                                                    }
                                                }),
                                        )
                                })
                                .into_any_element()
                        }
                    }
                })
                .into_any_element(),
        }
    }

    pub(in crate::ui) fn render_script_tests_section(
        &self,
        result: &PersistedScriptResult,
        cx: &App,
    ) -> AnyElement {
        if result.test_results.is_empty() {
            return div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("No tests recorded.")
                .into_any_element();
        }
        let mut column = v_flex().w_full().gap_1();
        for test in &result.test_results {
            let status = if test.passed { "PASS" } else { "FAIL" };
            let color = if test.passed {
                cx.theme().success
            } else {
                cx.theme().danger
            };
            let summary = match (&test.expected, &test.actual) {
                (Some(expected), Some(actual)) if expected != actual => {
                    format!("expected={expected}, actual={actual}")
                }
                _ => String::new(),
            };
            let detail = test
                .error_message
                .as_ref()
                .filter(|message| !message.is_empty())
                .cloned()
                .or_else(|| {
                    if summary.is_empty() {
                        None
                    } else {
                        Some(summary)
                    }
                });
            let line = if let Some(detail) = detail {
                format!("[{status}] {} ({detail})", test.name)
            } else {
                format!("[{status}] {}", test.name)
            };
            column = column.child(div().text_xs().text_color(color).child(line));
        }
        column.into_any_element()
    }

    pub(in crate::ui) fn render_script_env_changes_section(
        &self,
        result: &PersistedScriptResult,
        cx: &App,
    ) -> AnyElement {
        let mut column = v_flex().w_full().gap_1();
        if result.no_environment_selected_with_env_writes {
            column = column.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().danger)
                    .child("Warning: Script wrote environment changes, but \"No environment\" was selected. Changes were not persisted."),
            );
        }
        if result.environment_diff.is_empty() {
            return column
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("No environment changes."),
                )
                .into_any_element();
        }
        for change in &result.environment_diff {
            let line = match change.kind {
                EnvironmentChangeKind::Added => {
                    format!(
                        "[added] {} = {}",
                        change.key,
                        change.new_value.clone().unwrap_or_default()
                    )
                }
                EnvironmentChangeKind::Updated => format!(
                    "[updated] {}: {} -> {}",
                    change.key,
                    change.old_value.clone().unwrap_or_default(),
                    change.new_value.clone().unwrap_or_default()
                ),
                EnvironmentChangeKind::Removed => format!(
                    "[removed] {} (was {})",
                    change.key,
                    change.old_value.clone().unwrap_or_default()
                ),
            };
            column = column.child(div().text_xs().child(line));
        }
        column.into_any_element()
    }

    pub(in crate::ui) fn render_script_console_section(
        &self,
        result: &PersistedScriptResult,
        cx: &App,
    ) -> AnyElement {
        if result.console_output.is_empty() {
            return div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("No console output.")
                .into_any_element();
        }
        let mut column = v_flex().w_full().gap_1();
        for line in &result.console_output {
            let prefix = line.level.to_uppercase();
            column = column.child(div().text_xs().child(format!(
                "{} [{}] {}",
                Self::format_human_time(&line.timestamp),
                prefix,
                line.message
            )));
        }
        column.into_any_element()
    }

    pub(in crate::ui) fn render_post_script_results(
        &self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut panel = v_flex()
            .h_full()
            .min_h_0()
            .w_full()
            .overflow_hidden()
            .p_2()
            .gap_2();

        if let Some(result) = self.script_result.as_ref() {
            let mut content = v_flex().w_full().gap_2();
            content = content
                .child(
                    h_flex()
                        .w_full()
                        .items_center()
                        .justify_between()
                        .child(div().text_sm().font_semibold().child(if result.success {
                            "Script Succeeded"
                        } else {
                            "Script Failed"
                        }))
                        .child(
                            Button::new("clear-script-results")
                                .small()
                                .ghost()
                                .label("Clear")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.script_result = None;
                                    if let Some(request_id) =
                                        this.shell.workspace_tree.selected_request_id()
                                    {
                                        if let Err(error) = clear_script_result_for_request(
                                            &this.current_workspace_paths,
                                            request_id,
                                        ) {
                                            log::error!("Failed to clear script result: {error}");
                                        }
                                    }
                                    cx.notify();
                                })),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!(
                            "Updated {}",
                            Self::format_human_timestamp(&result.updated_at)
                        )),
                );

            if let Some(error_message) = &result.error_message {
                if !error_message.is_empty() {
                    content = content.child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().danger)
                            .child(format!("Error: {error_message}")),
                    );
                }
            }

            content = content.child(div().text_xs().font_semibold().child("Tests"));
            content = content.child(self.render_script_tests_section(result, cx));
            content = content.child(div().text_xs().font_semibold().child("Environment Changes"));
            content = content.child(self.render_script_env_changes_section(result, cx));
            content = content.child(div().text_xs().font_semibold().child("Console"));
            content = content.child(self.render_script_console_section(result, cx));

            panel = panel.child(
                div()
                    .w_full()
                    .h_full()
                    .overflow_y_scrollbar()
                    .child(content),
            );
        } else {
            panel = panel.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("Send a request to see script results"),
            );
        }

        panel.into_any_element()
    }

    pub(in crate::ui) fn render_post_script_editor_and_results(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let post_script_has_selection =
            !self.post_script_editor.read(cx).selected_range().is_empty();
        // TODO: Keep this as a single parent card with one divider between editor/results.
        // It avoids double-border overlap and is less error-prone than separate bordered panes.
        v_flex()
            .h_full()
            .w_full()
            .gap_0()
            .rounded(px(8.0))
            .border_dashed()
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .child(
                div()
                    .h_1_2()
                    .min_h_0()
                    .w_full()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .p_0()
                    .child(
                        div().w_full().h_full().overflow_y_scrollbar().child(
                            Input::new(&self.post_script_editor)
                                .h_full()
                                .p_0()
                                .border_0()
                                .focus_bordered(false)
                                .font_family(cx.theme().mono_font_family.clone())
                                .text_size(cx.theme().mono_font_size)
                                .context_menu({
                                    move |menu, _, cx| {
                                        build_text_edit_context_menu_with_find(
                                            menu,
                                            post_script_has_selection,
                                            cx.theme().muted_foreground,
                                        )
                                    }
                                }),
                        ),
                    ),
            )
            .child(
                div()
                    .h_1_2()
                    .min_h_0()
                    .w_full()
                    .child(self.render_post_script_results(window, cx)),
            )
            .into_any_element()
    }

    pub(in crate::ui) fn render_request_panel(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let editor_container = match self.request.active_tab {
            RequestTab::Body => div()
                .flex_1()
                .w_full()
                .rounded(px(8.0))
                .border_1()
                .border_color(cx.theme().border)
                .p_0()
                .child(self.render_request_editor_surface(window, cx)),
            RequestTab::PostScript => div()
                .flex_1()
                .w_full()
                .child(self.render_request_editor_surface(window, cx)),
            _ => div()
                .flex_1()
                .w_full()
                .rounded(px(8.0))
                .border_1()
                .border_color(cx.theme().border)
                .p_3()
                .child(self.render_request_editor_surface(window, cx)),
        };

        v_flex()
            .h_full()
            .w_full()
            .gap_2()
            .p_3()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(self.render_request_tabs(cx))
            .child(editor_container)
    }
}
