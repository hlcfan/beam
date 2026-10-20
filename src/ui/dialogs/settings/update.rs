use super::*;
use gpui_kit::base::TestSupportExt as _;

impl SettingsDialogView {
    pub(super) fn render_update_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let updater = self.updater.read(cx);
        let phase = updater.phase;
        let checking = matches!(phase, UpdatePhase::ChangingChannel | UpdatePhase::Checking);
        let installed = matches!(phase, UpdatePhase::Installed | UpdatePhase::Restarting);
        let available = updater
            .release
            .as_ref()
            .is_some_and(|release| release.is_available());
        let verification_error = crate::updater::installation_error();
        let version = if checking {
            "Checking…".to_string()
        } else {
            updater
                .release
                .as_ref()
                .map(|release| {
                    if phase == UpdatePhase::Idle && !available {
                        format!("{} (up to date)", release.version())
                    } else {
                        release.version().to_string()
                    }
                })
                .unwrap_or_else(|| "Unavailable".into())
        };
        let status = match phase {
            UpdatePhase::ChangingChannel => Some("Saving channel…"),
            UpdatePhase::Checking => Some("Checking for updates…"),
            UpdatePhase::Installing => Some("Downloading and installing the update…"),
            UpdatePhase::Installed => Some("Update installed. Restart to use the new version."),
            UpdatePhase::Restarting => Some("Saving changes before restarting…"),
            UpdatePhase::Idle if available => Some("An update is available."),
            UpdatePhase::Idle if updater.release.is_some() => None,
            UpdatePhase::Idle => Some("Check for updates to try again."),
        };
        let error = updater.error.clone();
        v_flex()
            .w_full()
            .min_w_0()
            .gap_4()
            .child(div().text_sm().font_semibold().child("Update"))
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!(
                        "Installed version: {}",
                        crate::updater::CURRENT_VERSION
                    )),
            )
            .child(
                v_flex()
                    .gap_2()
                    .child(div().text_sm().child("Release channel"))
                    .child(
                        h_flex()
                            .w_full()
                            .items_center()
                            .gap_4()
                            .child(
                                div()
                                    .id("settings-update-channel")
                                    .w_48()
                                    .flex_shrink_0()
                                    .input_h(Size::Medium)
                                    .child(
                                        Select::new(&self.update_channel_select)
                                            .w_full()
                                            .cursor_pointer()
                                            .disabled(updater.channel_locked())
                                            .accessibility_label("Release channel"),
                                    )
                                    .test_support(),
                            )
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child("Available version"),
                                    )
                                    .child(
                                        div()
                                            .id("settings-update-version")
                                            .text_sm()
                                            .child(version)
                                            .test_support(),
                                    ),
                            ),
                    ),
            )
            .when(updater.channel == UpdateChannel::Nightly, |panel| {
                panel.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("Nightly builds contain the latest changes and may be unstable."),
                )
            })
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("settings-install-update")
                            .cursor_pointer()
                            .loading(phase == UpdatePhase::Installing)
                            .label(if phase == UpdatePhase::Installing {
                                "Updating…"
                            } else {
                                "Update"
                            })
                            .disabled(
                                !available
                                    || checking
                                    || updater.channel_locked()
                                    || verification_error.is_some(),
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.updater.update(cx, |updater, cx| updater.install(cx));
                            })),
                    )
                    .child(
                        Button::new("settings-check-update")
                            .cursor_pointer()
                            .loading(checking)
                            .label("Check for updates")
                            .disabled(checking || updater.channel_locked())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.updater.update(cx, |updater, cx| updater.check(cx));
                            })),
                    )
                    .when(installed, |row| {
                        row.child(
                            Button::new("settings-restart-update")
                                .cursor_pointer()
                                .label("Restart")
                                .disabled(phase == UpdatePhase::Restarting)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.beam_view
                                        .update(cx, |beam, cx| beam.restart_after_update(cx));
                                })),
                        )
                    }),
            )
            .when_some(status, |panel, status| {
                panel.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(status),
                )
            })
            .when_some(error, |panel, error| {
                panel.child(div().text_sm().text_color(cx.theme().danger).child(error))
            })
            .when_some(verification_error, |panel, error| {
                panel.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(error),
                )
            })
            .into_any_element()
    }
}
