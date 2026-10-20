use super::{SettingsDialogView, SettingsSection};
use crate::app_shell::{AppShellState, DataSyncRuntime};
use crate::models::UpdateChannel;
use crate::paths::BeamPaths;
use crate::ui::BeamView;
use crate::ui::update::UpdatePhase;
use gpui_kit::base::test_support::find;
use gpui_kit::component::{Root, Theme};
use gpui_kit::{AppContext as _, Entity, TestAppContext, VisualTestContext, px, size};
use std::sync::mpsc::{channel, sync_channel};

fn fixture(
    cx: &mut TestAppContext,
) -> (
    Entity<SettingsDialogView>,
    tempfile::TempDir,
    &mut VisualTestContext,
) {
    cx.update(gpui_kit::init);
    let directory = tempfile::tempdir().expect("directory");
    let paths = BeamPaths::from_root(directory.path().to_path_buf());
    let (command_tx, _) = sync_channel(32);
    let (_, event_rx) = channel();
    let (root, cx) = cx.add_window_view(move |window, cx| {
        let shell = AppShellState {
            update_channel: UpdateChannel::Nightly,
            ..AppShellState::default()
        };
        let beam = cx.new(|cx| {
            BeamView::new(
                shell,
                vec![],
                DataSyncRuntime {
                    command_tx,
                    event_rx,
                },
                paths,
                window,
                cx,
            )
        });
        let settings = cx.new(|cx| SettingsDialogView::new(beam, window, cx));
        settings.update(cx, |settings, _| {
            settings.selected_section = SettingsSection::Update
        });
        Root::new(settings, window, cx)
    });
    let settings = root.read_with(cx, |root, _| {
        root.view()
            .clone()
            .downcast::<SettingsDialogView>()
            .expect("settings")
    });
    cx.simulate_resize(size(px(920.), px(650.)));
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.update(|window, cx| window.simulate_next_frame(cx));
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    (settings, directory, cx)
}

#[gpui_kit::test]
fn update_panel_aligns_channel_version_and_action_in_both_themes(cx: &mut TestAppContext) {
    let (settings, _directory, cx) = fixture(cx);
    settings.read_with(cx, |settings, cx| {
        assert_eq!(
            settings
                .update_channel_select
                .read(cx)
                .selected_value()
                .map(|value| value.as_ref()),
            Some("Nightly")
        );
    });
    for mode in [
        gpui_kit::component::ThemeMode::Light,
        gpui_kit::component::ThemeMode::Dark,
    ] {
        for font in [14., 16., 18.] {
            cx.update(|_, cx| {
                Theme::change(mode, None, cx);
                Theme::global_mut(cx).font_size = px(font);
                Theme::sync_base(cx);
            });
            cx.run_until_parked();
            cx.update(|window, cx| window.draw(cx).clear(cx));
            cx.update(|window, _| {
                let channel = find(window, &[], &"settings-update-channel".into())
                    .expect("channel")
                    .bounds();
                let version = find(window, &[], &"settings-update-version".into())
                    .expect("version")
                    .bounds();
                let update = find(window, &[], &"settings-install-update".into())
                    .expect("update")
                    .bounds();
                assert!(version.left() >= channel.right());
                assert!(update.top() >= channel.bottom());
                assert_eq!(update.left(), channel.left());
            });
        }
    }
}

#[gpui_kit::test]
fn reopening_settings_retains_installation_state(cx: &mut TestAppContext) {
    let (settings, _directory, cx) = fixture(cx);
    cx.update(|window, cx| {
        let beam = settings.read(cx).beam_view.clone();
        let updater = beam.read(cx).updater.clone();
        updater.update(cx, |updater, cx| {
            updater.phase = UpdatePhase::Installed;
            cx.notify();
        });
        let reopened = cx.new(|cx| SettingsDialogView::new(beam, window, cx));
        assert!(reopened.read(cx).updater == updater);
        assert!(reopened.read(cx).updater.read(cx).phase == UpdatePhase::Installed);
    });
}

#[gpui_kit::test]
fn failed_channel_save_restores_dropdown_without_overwriting_settings(cx: &mut TestAppContext) {
    let (settings, directory, cx) = fixture(cx);
    let paths = BeamPaths::from_root(directory.path().to_path_buf());
    std::fs::create_dir_all(paths.app_settings_file.parent().expect("parent")).expect("directory");
    std::fs::write(&paths.app_settings_file, "invalid = = toml").expect("corrupt settings");
    settings.update(cx, |settings, cx| {
        settings.updater.update(cx, |updater, cx| {
            updater.select_channel(UpdateChannel::Release, cx)
        });
    });
    cx.run_until_parked();
    settings.read_with(cx, |settings, cx| {
        let updater = settings.updater.read(cx);
        assert_eq!(updater.channel, UpdateChannel::Nightly);
        assert!(
            updater
                .error
                .as_ref()
                .is_some_and(|error| error.contains("save the update channel"))
        );
        assert_eq!(
            settings
                .update_channel_select
                .read(cx)
                .selected_value()
                .map(|value| value.as_ref()),
            Some("Nightly")
        );
    });
    assert_eq!(
        std::fs::read_to_string(paths.app_settings_file).expect("settings"),
        "invalid = = toml"
    );
}
