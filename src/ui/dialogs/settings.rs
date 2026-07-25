use super::super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SettingsSection {
    Theme,
    Editor,
}

pub(in crate::ui) struct SettingsDialogView {
    beam_view: Entity<BeamView>,
    selected_section: SettingsSection,
}

impl SettingsDialogView {
    pub(in crate::ui) fn new(
        beam_view: Entity<BeamView>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Self {
        Self {
            beam_view,
            selected_section: SettingsSection::Theme,
        }
    }
}

impl Render for SettingsDialogView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let active_theme_name = cx.theme().theme_name().clone();
        let active_font_size = AppFontSize::from_pixels_value(cx.theme().font_size.as_f32());
        let (auto_format_response, wrap_body_editor) = {
            let beam_view = self.beam_view.read(cx);
            (
                beam_view.shell.theme.auto_format_response,
                beam_view.shell.theme.wrap_body_editor,
            )
        };
        let theme_options: Vec<SharedString> = ThemeRegistry::global(cx)
            .sorted_themes()
            .into_iter()
            .map(|theme| theme.name.clone())
            .collect();
        let font_size_options = [AppFontSize::Small, AppFontSize::Medium, AppFontSize::Large];

        let mut right_panel = v_flex().w_full().h_full().gap_3();
        match self.selected_section {
            SettingsSection::Theme => {
                let beam_view = self.beam_view.clone();
                let font_size_beam_view = beam_view.clone();
                let active_theme_name_for_menu = active_theme_name.clone();
                let theme_options_for_menu = theme_options.clone();
                let font_size_options_for_menu = font_size_options;
                right_panel = right_panel
                    .child(div().text_sm().font_semibold().child("Theme"))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("Choose a theme. The selected theme is also available from the system menu."),
                    )
                    .child(
                        DropdownButton::new("settings-theme-dropdown")
                            .w(px(320.0))
                            .button(
                                Button::new("settings-theme-dropdown-button")
                                    .w(px(290.0))
                                    .justify_start()
                                    .label(active_theme_name.to_string()),
                            )
                            .dropdown_menu(move |menu, window, _| {
                                theme_options_for_menu.clone().into_iter().fold(
                                    menu.scrollable(true).max_h(px(220.0)),
                                    |menu, theme_name| {
                                        let selected_theme = theme_name.clone();
                                    let target_view = beam_view.clone();
                                    let checked = theme_name == active_theme_name_for_menu;
                                    menu.item(
                                            PopupMenuItem::element(move |_, _| {
                                                div()
                                                    .w_full()
                                                    .px_2()
                                                    .py_1()
                                                    .cursor_pointer()
                                                    .child(theme_name.clone())
                                            })
                                        .checked(checked)
                                        .on_click(window.listener_for(
                                            &target_view,
                                            move |_: &mut BeamView, _, _, cx| {
                                                BeamView::apply_named_theme(
                                                    selected_theme.clone(),
                                                    cx,
                                                );
                                                cx.notify();
                                            },
                                        )),
                                    )
                                })
                            }),
                    )
                    .child(div().mt_4().text_sm().font_semibold().child("Font size"))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("Choose the app font scale for the interface."),
                    )
                    .child(
                        DropdownButton::new("settings-font-size-dropdown")
                            .w(px(320.0))
                            .button(
                                Button::new("settings-font-size-dropdown-button")
                                    .w(px(290.0))
                                    .justify_start()
                                    .label(active_font_size.label()),
                            )
                            .dropdown_menu(move |menu, window, _| {
                                font_size_options_for_menu.into_iter().fold(
                                    menu.scrollable(true).max_h(px(220.0)),
                                    |menu, font_size| {
                                        let target_view = font_size_beam_view.clone();
                                        menu.item(
                                            PopupMenuItem::element(move |_, _| {
                                                div()
                                                    .w_full()
                                                    .px_2()
                                                    .py_1()
                                                    .cursor_pointer()
                                                    .child(font_size.label())
                                            })
                                            .checked(font_size == active_font_size)
                                            .on_click(window.listener_for(
                                                &target_view,
                                                move |this: &mut BeamView, _, window, cx| {
                                                    this.apply_font_size_setting(
                                                        font_size,
                                                        window,
                                                        cx,
                                                    );
                                                },
                                            )),
                                        )
                                    },
                                )
                            }),
                    );
            }
            SettingsSection::Editor => {
                let auto_format_beam_view = self.beam_view.clone();
                let wrap_body_editor_beam_view = self.beam_view.clone();
                right_panel = right_panel
                    .child(
                        h_flex()
                            .items_start()
                            .justify_between()
                            .gap_3()
                            .child(
                                v_flex()
                                    .flex_1()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_semibold()
                                            .child("Editor soft wrap"),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child("Wraps long lines in the editor."),
                                    ),
                            )
                            .child(
                                Switch::new("settings-wrap-body-editor")
                                    .cursor_pointer()
                                    .checked(wrap_body_editor)
                                    .on_click(cx.listener(move |_, checked: &bool, window, cx| {
                                        wrap_body_editor_beam_view.update(cx, |this, cx| {
                                            this.apply_wrap_body_editor_setting(*checked, window, cx);
                                        });
                                    })),
                            ),
                    )
                    .child(
                        h_flex()
                            .mt_4()
                            .items_start()
                            .justify_between()
                            .child(
                                v_flex()
                                    .flex_1()
                                    .gap_1()
                                    .child(div().text_sm().font_semibold().child("Auto format response"))
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child("Automatically formats the response body after a request completes."),
                                    ),
                            )
                            .child(
                                Switch::new("settings-auto-format-response")
                                    .cursor_pointer()
                                    .checked(auto_format_response)
                                    .on_click(cx.listener(move |_, checked: &bool, window, cx| {
                                        auto_format_beam_view.update(cx, |this, cx| {
                                            this.apply_auto_format_response_setting(
                                                *checked, window, cx,
                                            );
                                        });
                                    })),
                            ),
                    );
            }
        }

        v_flex()
            .w_full()
            .h(px(520.0))
            .p_3()
            .gap_3()
            .child(
                h_flex()
                    .w_full()
                    .h_full()
                    .gap_3()
                    .child(
                        v_flex()
                            .w(px(220.0))
                            .h_full()
                            .rounded(px(8.0))
                            .border_1()
                            .border_color(cx.theme().border)
                            .bg(cx.theme().background)
                            .p_2()
                            .gap_1()
                            .child(
                                ListItem::new("settings-section-theme")
                                    .w_full()
                                    .cursor_pointer()
                                    .rounded(px(8.0))
                                    .px_2()
                                    .py_1()
                                    .selected(self.selected_section == SettingsSection::Theme)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.selected_section = SettingsSection::Theme;
                                        cx.notify();
                                    }))
                                    .child("Appearance"),
                            )
                            .child(
                                ListItem::new("settings-section-editor")
                                    .w_full()
                                    .cursor_pointer()
                                    .rounded(px(8.0))
                                    .px_2()
                                    .py_1()
                                    .selected(self.selected_section == SettingsSection::Editor)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.selected_section = SettingsSection::Editor;
                                        cx.notify();
                                    }))
                                    .child("Editor"),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .h_full()
                            .rounded(px(8.0))
                            .border_1()
                            .border_color(cx.theme().border)
                            .bg(cx.theme().background)
                            .p_3()
                            .child(right_panel),
                    ),
            )
            .into_any_element()
    }
}
