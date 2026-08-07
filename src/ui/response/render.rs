use super::super::*;

impl BeamView {
    fn render_response_tabs(&self, _window: &mut Window, cx: &mut Context<Self>) -> Div {
        let mut tabs = h_flex().items_center().gap_1().w_full();
        let tab_specs = [
            (ResponseTab::Body, "Body"),
            (ResponseTab::Headers, "Headers"),
        ];

        for (tab, label) in tab_specs {
            tabs = tabs.child(
                Button::new(format!("response-tab-{label}"))
                    .small()
                    .ghost()
                    .cursor_pointer()
                    .selected(self.active_response_tab == tab)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.active_response_tab = tab;
                        cx.notify();
                    }))
                    .label(label),
            );
        }

        let response_histories = self.response_history_entries.clone();
        let selected_response_history_index = self.selected_response_history_index;
        let response_history_view = cx.entity();
        tabs = tabs.child(
            Button::new("response-history-dropdown")
                .small()
                .ghost()
                .cursor_pointer()
                .rounded(px(6.0))
                .disabled(self.shell.workspace_tree.selected_request_id().is_none())
                .icon(
                    Icon::default()
                        .path("icons/history.svg")
                        .size(px(14.0))
                        .text_color(cx.theme().muted_foreground),
                )
                .dropdown_menu(move |menu, _window, menu_cx| {
                    let list_width_px = 180.0;
                    let mut menu = menu.min_w(px(list_width_px));

                    if response_histories.is_empty() {
                        return menu.item(
                            PopupMenuItem::element(move |_, cx| {
                                div()
                                    .w_full()
                                    .px_2()
                                    .py_1()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("No response history")
                            })
                            .disabled(true),
                        );
                    }

                    let popup_menu = menu_cx.entity().clone();
                    let row_width_px = 172.0;
                    let row_height_px = 32.0;
                    let row_height = px(row_height_px);
                    let row_content_height = px(28.0);
                    let list_height =
                        px((response_histories.len() as f32 * row_height_px).min(280.0));
                    let row_sizes = Rc::new(
                        response_histories
                            .iter()
                            .map(|_| size(px(row_width_px), row_height))
                            .collect::<Vec<_>>(),
                    );
                    let menu_response_histories = response_histories.clone();
                    let menu_selected_response_history_index =
                        selected_response_history_index;
                    let menu_response_history_view = response_history_view.clone();
                    let menu_popup_menu = popup_menu.clone();
                    let scroll_handle = VirtualListScrollHandle::new();
                    if let Some(index) = selected_response_history_index {
                        scroll_handle.scroll_to_item(index, ScrollStrategy::Center);
                    }

                    menu = menu.item(
                        PopupMenuItem::element(move |_, _cx| {
                            let row_sizes = row_sizes.clone();
                            let list_response_histories = menu_response_histories.clone();
                            let selected_response_history_index =
                                menu_selected_response_history_index;
                            let list_response_history_view = menu_response_history_view.clone();
                            let list_popup_menu = menu_popup_menu.clone();
                            let scroll_handle = scroll_handle.clone();

                            div()
                                .min_w(px(list_width_px))
                                .mx(px(-8.0))
                                .p_1()
                                .h(list_height)
                                .child(
                                v_virtual_list(
                                    list_response_history_view,
                                    "response-history-dropdown-list",
                                    row_sizes,
                                    move |_, visible_range, _, cx| {
                                        visible_range
                                            .map(|ix| {
                                                let entry = list_response_histories[ix].clone();
                                                let popup_menu = list_popup_menu.clone();
                                                let timestamp_text = entry.timestamp_text.clone();
                                                let status_text = entry.status_text.clone();
                                                let history_entry = entry.clone();
                                                let status_color =
                                                    Self::status_code_in_color(
                                                        entry.execution.status,
                                                        cx,
                                                    );
                                                div().w_full().h(row_height).pb(px(4.0)).child(
                                                    ListItem::new(format!(
                                                        "response-history-dropdown-row-{ix}"
                                                    ))
                                                    .w_full()
                                                    .h(row_content_height)
                                                    .rounded(px(6.0))
                                                    .cursor_pointer()
                                                    .selected(
                                                        selected_response_history_index == Some(ix),
                                                    )
                                                    .px_1()
                                                    .py_1()
                                                    .child(
                                                        h_flex()
                                                            .w_full()
                                                            .items_center()
                                                            .justify_between()
                                                            .text_sm()
                                                            .child(
                                                                div()
                                                                    .text_color(
                                                                        cx.theme().muted_foreground,
                                                                    )
                                                                    .child(timestamp_text),
                                                            )
                                                            .child(
                                                                div()
                                                                    .text_color(status_color)
                                                                    .child(status_text),
                                                            ),
                                                    )
                                                    .on_click(
                                                        cx.listener(move |this, _, window, cx| {
                                                            cx.stop_propagation();
                                                            window.prevent_default();
                                                            let snapshot =
                                                                load_response_snapshot_for_history_entry(
                                                                    &this.current_workspace_paths,
                                                                    &history_entry,
                                                                );
                                                            this.apply_response_snapshot(
                                                                &snapshot,
                                                                window,
                                                                cx,
                                                            );
                                                            this.selected_response_history_index =
                                                                Some(ix);
                                                            popup_menu.update(cx, |_, cx| {
                                                                cx.emit(DismissEvent)
                                                            });
                                                            cx.notify();
                                                        }),
                                                    ),
                                                )
                                            })
                                            .collect::<Vec<_>>()
                                    },
                                )
                                .track_scroll(&scroll_handle),
                            )
                        })
                        .disabled(true),
                    );

                    menu
                }),
        );

        tabs
    }

    fn render_response_editor_surface(&self, cx: &mut Context<Self>) -> AnyElement {
        match self.active_response_tab {
            ResponseTab::Body => {
                let response_body_text = self.response_body_editor.read(cx).value().to_string();
                let trimmed_body = response_body_text.trim();
                if trimmed_body.is_empty() {
                    return self.render_response_body_shortcuts_empty_state(cx);
                }
                let response_body_has_selection = !self
                    .response_body_editor
                    .read(cx)
                    .selected_range()
                    .is_empty();
                Input::new(&self.response_body_editor)
                    .h_full()
                    .p_0()
                    .border_0()
                    .focus_bordered(false)
                    .disabled(true)
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_size(cx.theme().mono_font_size)
                    .context_menu(move |menu, _window, cx| {
                        let muted_foreground = cx.theme().muted_foreground;
                        let menu = menu
                            .menu_with_icon(
                                "Format",
                                Icon::default().path("icons/indent.svg"),
                                Box::new(FormatResponseBody),
                            )
                            .separator();
                        build_text_edit_context_menu(
                            menu,
                            response_body_has_selection,
                            muted_foreground,
                        )
                    })
                    .into_any_element()
            }
            ResponseTab::Headers => self.render_response_headers_table(cx),
        }
    }

    fn command_key_icon_path() -> &'static str {
        if cfg!(target_os = "macos") {
            MACOS_COMMAND_ICON_PATH
        } else {
            NON_MACOS_COMMAND_ICON_PATH
        }
    }

    fn render_shortcut_key_with_icon(
        &self,
        icon_path: &'static str,
        key_text: Option<&'static str>,
        cx: &mut Context<Self>,
    ) -> Div {
        let mut key = h_flex()
            .items_center()
            .gap_1()
            .px_1p5()
            .h(px(22.0))
            .rounded(px(6.0))
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().secondary)
            .child(
                Icon::default()
                    .path(icon_path)
                    .size(px(12.0))
                    .text_color(cx.theme().muted_foreground),
            );
        if let Some(key_text) = key_text {
            key = key.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(key_text),
            );
        }
        key
    }

    fn render_response_shortcut_row(
        &self,
        label: &'static str,
        key: Div,
        cx: &mut Context<Self>,
    ) -> Div {
        h_flex()
            .items_center()
            .justify_between()
            .w(px(240.0))
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(label)
            .child(key)
    }

    fn render_response_body_shortcuts_empty_state(&self, cx: &mut Context<Self>) -> AnyElement {
        let command_icon_path = Self::command_key_icon_path();
        let send_key = self.render_shortcut_key_with_icon(command_icon_path, Some("Enter"), cx);
        let new_request_key = self.render_shortcut_key_with_icon(command_icon_path, Some("N"), cx);
        let focus_url_key = self.render_shortcut_key_with_icon(command_icon_path, Some("L"), cx);

        h_flex()
            .h_full()
            .w_full()
            .items_center()
            .justify_center()
            .child(
                v_flex()
                    .items_center()
                    .gap_2()
                    .child(self.render_response_shortcut_row("Send Request", send_key, cx))
                    .child(self.render_response_shortcut_row("New Request", new_request_key, cx))
                    .child(self.render_response_shortcut_row("Focus URL", focus_url_key, cx)),
            )
            .into_any_element()
    }

    fn render_response_headers_table(&self, cx: &App) -> AnyElement {
        let rows = parse_response_headers(&self.response_headers_raw);
        if rows.is_empty() {
            return div()
                .h_full()
                .w_full()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child("No response headers. Send a request to view headers.")
                .into_any_element();
        }

        fn escape_html(text: &str) -> String {
            text.replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
                .replace('"', "&quot;")
                .replace('\'', "&#39;")
        }

        let mut table = String::from(
            "<table style=\"width:100%; border-collapse:collapse; font-size:12px;\">\
             <thead>\
             <tr>\
             <th style=\"text-align:left; padding:6px 8px; border-bottom:1px solid currentColor; width:220px;\">Header</th>\
             <th style=\"text-align:left; padding:6px 8px; border-bottom:1px solid currentColor;\">Value</th>\
             </tr>\
             </thead><tbody>",
        );

        for (key, value) in rows {
            table.push_str(&format!(
                "<tr>\
                 <td style=\"padding:6px 8px; border-bottom:1px solid currentColor; vertical-align:top; white-space:pre-wrap;\">{}</td>\
                 <td style=\"padding:6px 8px; border-bottom:1px solid currentColor; vertical-align:top; white-space:pre-wrap;\">{}</td>\
                 </tr>",
                escape_html(&key),
                escape_html(&value)
            ));
        }
        table.push_str("</tbody></table>");

        html(table)
            .w_full()
            .h_full()
            .scrollable(true)
            .selectable(true)
            .into_any_element()
    }

    fn render_shimmer_loading_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let color = cx.theme().progress_bar;
        div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .h(px(2.0))
            .overflow_hidden()
            .rounded_t(px(8.0))
            .bg(color.opacity(0.18))
            .child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .w(relative(0.28))
                    .bg(color.opacity(0.85))
                    .rounded_full()
                    .with_animation(
                        "shimmer-loading",
                        Animation::new(Duration::from_millis(1400)).repeat(),
                        move |this, delta| this.left(relative(delta)),
                    ),
            )
    }

    pub(in crate::ui) fn render_response_panel(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let is_sending = self
            .shell
            .workspace_tree
            .selected_request_id()
            .map(|id| self.is_request_sending(id))
            .unwrap_or(false);

        let response_container = match self.active_response_tab {
            ResponseTab::Body => div()
                .flex_1()
                .w_full()
                .relative()
                .rounded(px(8.0))
                .border_1()
                .border_color(cx.theme().border)
                .p_0()
                .child(
                    div()
                        .w_full()
                        .h_full()
                        .when(is_sending, |d| d.opacity(0.45))
                        .child(self.render_response_editor_surface(cx)),
                )
                .when(is_sending, |d| d.child(self.render_shimmer_loading_bar(cx))),
            ResponseTab::Headers => div()
                .flex_1()
                .w_full()
                .relative()
                .rounded(px(8.0))
                .border_1()
                .border_color(cx.theme().border)
                .p_3()
                .when(is_sending, |d| d.opacity(0.45))
                .child(self.render_response_editor_surface(cx))
                .when(is_sending, |d| d.child(self.render_shimmer_loading_bar(cx))),
        };

        v_flex()
            .h_full()
            .w_full()
            .gap_2()
            .p_3()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .w_full()
                    .gap_2()
                    .child(self.render_response_tabs(window, cx))
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(self.render_response_status_summary(cx))
                            .child(format!("Time: {}", self.response_time))
                            .child(format!("Size: {}", self.response_size)),
                    ),
            )
            .child(response_container)
    }

    fn render_response_status_summary(&self, cx: &mut Context<Self>) -> AnyElement {
        let (status_code, status_text) =
            Self::response_status_code_and_text(&self.response_status, self.response_status_code);
        let status_color = Self::status_code_in_color(self.response_status_code, cx);
        let trigger = h_flex()
            .items_center()
            .gap_1()
            .child("Status:")
            .child(
                div()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(status_color)
                    .when(status_text.is_some(), |div| div.cursor_pointer())
                    .child(status_code),
            )
            .cursor_pointer();

        match status_text {
            Some(status_text) => HoverCard::new("response-status-summary")
                .anchor(gpui::Anchor::BottomRight)
                .appearance(false)
                .open_delay(Duration::from_millis(100))
                .close_delay(Duration::from_millis(150))
                .trigger(trigger)
                .child(
                    div()
                        .occlude()
                        .popover_style(cx)
                        .px_2()
                        .py_0()
                        .text_sm()
                        .child(status_text),
                )
                .into_any_element(),
            None => trigger.into_any_element(),
        }
    }

    fn response_status_code_and_text(
        status: &str,
        status_code: Option<u16>,
    ) -> (String, Option<String>) {
        let Some(status_code) = status_code else {
            return (status.to_string(), None);
        };

        let status_code_text = status_code.to_string();
        let status_text = status
            .strip_prefix(&status_code_text)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_string)
            .or_else(|| {
                reqwest::StatusCode::from_u16(status_code)
                    .ok()
                    .and_then(|status| status.canonical_reason())
                    .map(str::to_string)
            });

        (status_code_text, status_text)
    }
}
