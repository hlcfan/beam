use super::BeamView;
use crate::app_shell::{AppShellState, DataSyncRuntime};
use crate::models::UpdateChannel;
use crate::paths::BeamPaths;
use crate::ui::update::UpdatePhase;
use crate::updater::AvailableUpdate;
use gpui_kit::component::{Root, Theme, ThemeMode};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Entity, TestAppContext, VisualTestContext, px, size};
use std::sync::mpsc::{channel, sync_channel};

fn fixture(
    cx: &mut TestAppContext,
) -> (Entity<BeamView>, tempfile::TempDir, &mut VisualTestContext) {
    cx.update(gpui_kit::init);
    let directory = tempfile::tempdir().expect("directory");
    let paths = BeamPaths::from_root(directory.path().to_path_buf());
    let (command_tx, _) = sync_channel(32);
    let (_, event_rx) = channel();
    let (root, cx) = cx.add_window_view(move |window, cx| {
        let beam = cx.new(|cx| {
            BeamView::new(
                AppShellState::default(),
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
        Root::new(beam, window, cx)
    });
    let beam = root.read_with(cx, |root, _| {
        root.view()
            .clone()
            .downcast::<BeamView>()
            .expect("Beam view")
    });
    cx.simulate_resize(size(px(1100.), px(750.)));
    cx.update(|window, cx| {
        window.activate_window();
        window.render_frame(cx);
    });
    cx.run_until_parked();
    (beam, directory, cx)
}

#[gpui_kit::test]
fn update_button_tracks_available_versions_and_installation(cx: &mut TestAppContext) {
    let (beam, _directory, cx) = fixture(cx);
    let updater = beam.read_with(cx, |beam, _| beam.updater.clone());
    for (channel, phase, version, available, visible) in [
        (
            UpdateChannel::Release,
            UpdatePhase::Idle,
            None,
            false,
            false,
        ),
        (
            UpdateChannel::Release,
            UpdatePhase::Idle,
            Some("0.8.0"),
            false,
            false,
        ),
        (
            UpdateChannel::Release,
            UpdatePhase::Idle,
            Some("0.9.0"),
            true,
            true,
        ),
        (
            UpdateChannel::Nightly,
            UpdatePhase::ChangingChannel,
            None,
            false,
            false,
        ),
        (
            UpdateChannel::Nightly,
            UpdatePhase::Checking,
            None,
            false,
            false,
        ),
        (
            UpdateChannel::Nightly,
            UpdatePhase::Idle,
            Some("0.8.0-nightly+g0123456789ab"),
            true,
            true,
        ),
        (
            UpdateChannel::Nightly,
            UpdatePhase::Installing,
            Some("0.8.0-nightly+g0123456789ab"),
            true,
            true,
        ),
        (
            UpdateChannel::Nightly,
            UpdatePhase::Installed,
            Some("0.8.0-nightly+g0123456789ab"),
            true,
            false,
        ),
        (
            UpdateChannel::Nightly,
            UpdatePhase::Restarting,
            Some("0.8.0-nightly+g0123456789ab"),
            true,
            false,
        ),
    ] {
        updater.update(cx, |updater, cx| {
            updater.channel = channel;
            updater.phase = phase;
            updater.release =
                version.map(|version| AvailableUpdate::test_release(version, available));
            cx.notify();
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.render_frame(cx);
            let button = window.try_find("status-bar-update");
            assert_eq!(button.is_some(), visible);
            if let Some(button) = button {
                assert_eq!(
                    button.label(),
                    Some(format!("Update to {}", version.expect("version")).as_str())
                );
            }
        });
    }
}

#[gpui_kit::test]
fn update_button_precedes_key_bindings_and_opens_update_settings(cx: &mut TestAppContext) {
    let (beam, _directory, cx) = fixture(cx);
    let updater = beam.read_with(cx, |beam, _| beam.updater.clone());
    updater.update(cx, |updater, cx| {
        updater.channel = UpdateChannel::Nightly;
        updater.release = Some(AvailableUpdate::test_release(
            "0.8.0-nightly+g0123456789ab",
            true,
        ));
        cx.notify();
    });
    for mode in [ThemeMode::Light, ThemeMode::Dark] {
        for font in [14., 16., 18.] {
            cx.update(|window, cx| {
                Theme::change(mode, None, cx);
                Theme::global_mut(cx).font_size = px(font);
                Theme::sync_base(cx);
                window.render_frame(cx);
                let update = window.find("status-bar-update").bounds();
                let keys = window.find("status-bar-key-bindings-modal").bounds();
                assert!(update.right() < keys.left());
                assert!((update.center().y - keys.center().y).abs() <= px(1.));
                assert!(update.left() > window.find("status-bar-import-modal").bounds().right());
            });
        }
    }
    cx.update(|window, cx| window.click("status-bar-update", cx));
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("settings-update-channel").is_some());
        assert!(window.try_find("settings-update-version").is_some());
        assert!(updater.read(cx).phase == UpdatePhase::Idle);
    });
}
