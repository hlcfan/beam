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
                Editor::new(&self.response_body_editor)
                    .h_full()
                    .p_0()
                    .border_0()
                    .bordered(false)
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
            .bottom_0()
            .left_0()
            .right_0()
            .with_animation(
                "shimmer-loading",
                Animation::new(Duration::from_millis(1400)).repeat(),
                move |this, delta| {
                    this.child(
                        canvas(
                            |_, _, _| {},
                            move |bounds, _, window, _| {
                                let Some(contour) = ShimmerTopContour::new(bounds) else {
                                    return;
                                };

                                let highlight_length = contour.length() * 0.28;
                                let highlight_end = (contour.length() + highlight_length) * delta;
                                let highlight_start = highlight_end - highlight_length;
                                let taper_length = 2.0_f32.min(contour.arc_length);
                                if let Some(highlight) = contour.path(
                                    highlight_start,
                                    highlight_end.min(taper_length),
                                    1.0,
                                ) {
                                    window.paint_path(highlight, color.opacity(0.85));
                                }
                                if let Some(highlight) = contour.path(
                                    highlight_start.max(taper_length),
                                    highlight_end.min(contour.length() - taper_length),
                                    2.0,
                                ) {
                                    window.paint_path(highlight, color.opacity(0.85));
                                }
                                if let Some(highlight) = contour.path(
                                    highlight_start.max(contour.length() - taper_length),
                                    highlight_end,
                                    1.0,
                                ) {
                                    window.paint_path(highlight, color.opacity(0.85));
                                }
                            },
                        )
                        .absolute()
                        .size_full(),
                    )
                },
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
                .child(
                    div()
                        .w_full()
                        .h_full()
                        .when(is_sending, |d| d.opacity(0.45))
                        .child(self.render_response_editor_surface(cx)),
                )
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

struct ShimmerTopContour {
    left_center: Point<Pixels>,
    right_center: Point<Pixels>,
    radius: f32,
    arc_length: f32,
    line_length: f32,
}

impl ShimmerTopContour {
    fn new(bounds: Bounds<Pixels>) -> Option<Self> {
        const PANE_RADIUS: f32 = 8.0;
        const STROKE_WIDTH: f32 = 2.0;
        const CORNER_SWEEP: f32 = 7.0 * std::f32::consts::PI / 18.0;

        let width = f32::from(bounds.size.width);
        let height = f32::from(bounds.size.height);
        let half_stroke = STROKE_WIDTH / 2.0;
        let outer_radius = PANE_RADIUS.min(width / 2.0).min(height / 2.0);
        let radius = outer_radius - half_stroke;
        if width <= STROKE_WIDTH || height <= STROKE_WIDTH || radius <= 0.0 {
            return None;
        }

        let origin_x = f32::from(bounds.origin.x);
        let origin_y = f32::from(bounds.origin.y);
        let center_y = origin_y + outer_radius;
        let left_center = point(px(origin_x + outer_radius), px(center_y));
        let right_center = point(px(origin_x + width - outer_radius), px(center_y));

        Some(Self {
            left_center,
            right_center,
            radius,
            arc_length: CORNER_SWEEP * radius,
            line_length: (width - 2.0 * outer_radius).max(0.0),
        })
    }

    fn length(&self) -> f32 {
        self.arc_length * 2.0 + self.line_length
    }

    fn path(&self, start: f32, end: f32, stroke_width: f32) -> Option<Path<Pixels>> {
        let start = start.clamp(0.0, self.length());
        let end = end.clamp(0.0, self.length());
        if end <= start {
            return None;
        }

        let mut builder = PathBuilder::stroke(px(stroke_width));
        builder.move_to(self.point_at(start));
        let mut cursor = start;

        if cursor < self.arc_length {
            let arc_end = end.min(self.arc_length);
            builder.arc_to(
                point(px(self.radius), px(self.radius)),
                px(0.0),
                false,
                true,
                self.point_at(arc_end),
            );
            cursor = arc_end;
        }

        let line_end_distance = self.arc_length + self.line_length;
        if cursor < end && cursor < line_end_distance {
            let line_end = end.min(line_end_distance);
            builder.line_to(self.point_at(line_end));
            cursor = line_end;
        }

        if cursor < end {
            builder.arc_to(
                point(px(self.radius), px(self.radius)),
                px(0.0),
                false,
                true,
                self.point_at(end),
            );
        }

        builder.build().ok()
    }

    fn point_at(&self, distance: f32) -> Point<Pixels> {
        let distance = distance.clamp(0.0, self.length());
        if distance <= self.arc_length {
            let angle = 10.0 * std::f32::consts::PI / 9.0 + distance / self.radius;
            return point(
                self.left_center.x + px(self.radius * angle.cos()),
                self.left_center.y + px(self.radius * angle.sin()),
            );
        }

        let line_end_distance = self.arc_length + self.line_length;
        if distance <= line_end_distance {
            return point(
                self.left_center.x + px(distance - self.arc_length),
                self.left_center.y - px(self.radius),
            );
        }

        let angle = -std::f32::consts::FRAC_PI_2 + (distance - line_end_distance) / self.radius;
        point(
            self.right_center.x + px(self.radius * angle.cos()),
            self.right_center.y + px(self.radius * angle.sin()),
        )
    }
}

#[cfg(test)]
mod tests {
    use gpui::{Bounds, Pixels, Point, point, px, size};

    use super::ShimmerTopContour;

    #[test]
    fn shimmer_contour_tracks_the_rounded_top_edge() {
        let contour = ShimmerTopContour::new(Bounds {
            origin: point(px(0.0), px(0.0)),
            size: size(px(100.0), px(50.0)),
        })
        .expect("the response panel is large enough for the shimmer contour");

        assert_point_close(contour.point_at(0.0), 1.422_152, 5.605_859);
        assert_point_close(contour.point_at(contour.arc_length), 8.0, 1.0);
        assert_point_close(
            contour.point_at(contour.arc_length + contour.line_length),
            92.0,
            1.0,
        );
        assert_point_close(contour.point_at(contour.length()), 98.577_85, 5.605_859);
        assert!(contour.path(0.0, contour.length(), 2.0).is_some());
    }

    fn assert_point_close(actual: Point<Pixels>, expected_x: f32, expected_y: f32) {
        assert!((f32::from(actual.x) - expected_x).abs() < 0.001);
        assert!((f32::from(actual.y) - expected_y).abs() < 0.001);
    }
}
