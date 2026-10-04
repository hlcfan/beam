use super::{
    WorkspaceMenuDelete, WorkspaceMenuRename, WorkspaceTabNavigation, workspace_tab_index,
};
use crate::app_shell::{AppCommand, AppEvent, AppShellState, DataSyncRuntime};
use crate::models::WorkspaceEntry;
use crate::paths::BeamPaths;
use crate::ui::BeamView;
use chrono::Utc;
use gpui_kit as gpui;
use gpui_kit::base::test_support::{ElementSnapshot, find};
use gpui_kit::component::{Root, Theme, ThemeMode};
use gpui_kit::{
    AppContext as _, Entity, Modifiers, MouseButton, ScrollDelta, ScrollWheelEvent, SharedString,
    TestAppContext, TouchPhase, VisualTestContext, point, px, size,
};
use std::sync::mpsc::{Receiver, Sender, channel, sync_channel};
use tempfile::TempDir;
use ulid::Ulid;

struct Fixture {
    view: Entity<BeamView>,
    commands: Receiver<AppCommand>,
    events: Sender<AppEvent>,
    _directory: TempDir,
}

fn fixture(cx: &mut TestAppContext, count: usize) -> (Fixture, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        super::init(cx);
    });
    let directory = tempfile::tempdir().expect("fixture directory");
    let paths = BeamPaths::from_root(directory.path().to_path_buf());
    let mut shell = AppShellState::default();
    shell.workspace.all_workspaces = (0..count)
        .map(|index| WorkspaceEntry {
            workspace_id: Ulid::new(),
            name: format!("Workspace {index}"),
            path: format!("workspace-{index}"),
            created_at: Utc::now(),
        })
        .collect();
    shell.workspace.workspace_id = shell
        .workspace
        .all_workspaces
        .first()
        .map(|entry| entry.workspace_id);
    shell.workspace.workspace_name = "Workspace 0".to_string();
    let (command_tx, commands) = sync_channel(32);
    let (events, event_rx) = channel();
    let (root, cx) = cx.add_window_view(move |window, cx| {
        let view = cx.new(|cx| {
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
        Root::new(view, window, cx)
    });
    let view = root.read_with(cx, |root, _| {
        root.view()
            .clone()
            .downcast::<BeamView>()
            .expect("Beam view")
    });
    cx.simulate_resize(size(px(800.), px(600.)));
    cx.update(|window, _| window.activate_window());
    draw(cx);
    (
        Fixture {
            view,
            commands,
            events,
            _directory: directory,
        },
        cx,
    )
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

fn snapshot(cx: &mut VisualTestContext, id: impl Into<SharedString>) -> ElementSnapshot {
    let id = gpui::ElementId::from(id.into());
    cx.update(|window, _| find(window, &[], &id).expect("rendered control"))
}

#[test]
fn workspace_tab_navigation_wraps_and_handles_empty_strips() {
    use WorkspaceTabNavigation::{First, Last, Next, Previous};
    assert_eq!(workspace_tab_index(0, 0, Next), None);
    assert_eq!(workspace_tab_index(1, 0, Previous), Some(0));
    assert_eq!(workspace_tab_index(3, 2, Next), Some(0));
    assert_eq!(workspace_tab_index(3, 0, Previous), Some(2));
    assert_eq!(workspace_tab_index(3, 2, First), Some(0));
    assert_eq!(workspace_tab_index(3, 0, Last), Some(2));
}

#[gpui_kit::test]
fn clicking_keeps_the_selected_workspace_tab_background(cx: &mut TestAppContext) {
    let (fixture, cx) = fixture(cx, 2);
    let active = fixture.view.read_with(cx, |view, _| {
        view.shell.workspace.workspace_id.expect("active workspace")
    });

    for mode in [ThemeMode::Light, ThemeMode::Dark] {
        cx.update(|_, cx| Theme::change(mode, None, cx));
        draw(cx);
        let tab = snapshot(cx, format!("workspace-tab-{active}")).bounds();
        for button in [MouseButton::Left, MouseButton::Right] {
            cx.simulate_mouse_down(tab.center(), button, Modifiers::default());
            // Check while pressed; native menu tracking can also consume mouse-up.
            draw(cx);
            cx.update(|window, cx| {
                let painted_tab = window
                    .painted_quads()
                    .into_iter()
                    .find(|quad| quad.bounds == tab.scale(window.scale_factor()))
                    .expect("painted tab background");
                assert_eq!(
                    painted_tab.background,
                    Theme::global(cx).background.into(),
                    "{button:?}-click preserves the selected background in {mode:?}"
                );
            });
            cx.simulate_mouse_up(tab.center(), button, Modifiers::default());
            draw(cx);
            cx.update(|window, cx| {
                assert_eq!(
                    fixture.view.read(cx).focused_workspace_tab(window),
                    Some(active)
                );
            });
            assert!(
                fixture.commands.try_recv().is_err(),
                "clicking the active tab does not switch"
            );
        }
    }
}

#[gpui_kit::test]
fn add_button_follows_short_tab_strips(cx: &mut TestAppContext) {
    let (fixture, cx) = fixture(cx, 3);
    let last = fixture.view.read_with(cx, |view, _| {
        view.shell.workspace.all_workspaces[2].workspace_id
    });
    let first = fixture.view.read_with(cx, |view, _| {
        view.shell.workspace.all_workspaces[0].workspace_id
    });
    for mode in [ThemeMode::Light, ThemeMode::Dark] {
        cx.update(|_, cx| Theme::change(mode, None, cx));
        for font_size in [14., 16., 18.] {
            cx.update(|_, cx| {
                Theme::global_mut(cx).font_size = px(font_size);
                Theme::sync_base(cx);
                cx.refresh_windows();
            });
            for width in [800., 1280.] {
                cx.simulate_resize(size(px(width), px(600.)));
                draw(cx);
                let last_tab = snapshot(cx, format!("workspace-tab-{last}")).bounds();
                let active_tab = snapshot(cx, format!("workspace-tab-{first}")).bounds();
                let title_bar = snapshot(cx, "beam-title-bar").bounds();
                assert_eq!(active_tab.top(), last_tab.top());
                assert_eq!(active_tab.size.height, last_tab.size.height);
                let minimum_inset = px(font_size / 4. - 1.);
                assert!(active_tab.top() - title_bar.top() >= minimum_inset);
                assert!(title_bar.bottom() - active_tab.bottom() >= minimum_inset);
                assert!(title_bar.size.height - active_tab.size.height <= px(font_size));
                let add = snapshot(cx, "add-workspace").bounds();
                assert_eq!(add.top(), active_tab.top());
                assert_eq!(add.size.height, active_tab.size.height);
                assert_eq!(add.size.width, add.size.height);
                let environment = snapshot(cx, "title-bar-environment-sheet").bounds();
                assert_eq!(environment.top(), active_tab.top());
                assert_eq!(environment.size.height, active_tab.size.height);
                let gap = add.left() - last_tab.right();
                assert!(gap >= px(0.) && gap <= px(font_size / 2.), "gap={gap:?}");
                assert!(environment.left() - add.right() > px(font_size));
                assert!(environment.right() <= px(width));
            }
        }
    }
}

#[gpui_kit::test]
fn holding_title_bar_controls_does_not_start_a_window_drag(cx: &mut TestAppContext) {
    let (fixture, cx) = fixture(cx, 3);
    let workspace = fixture.view.read_with(cx, |view, _| {
        view.shell.workspace.all_workspaces[1].workspace_id
    });
    let add = snapshot(cx, "add-workspace").bounds();
    let environment = snapshot(cx, "title-bar-environment-sheet").bounds();
    let blank_title_bar = point((add.right() + environment.left()) / 2., add.center().y);
    let title_bar = snapshot(cx, "beam-title-bar").bounds();

    for control in [
        format!("workspace-tab-{workspace}"),
        "add-workspace".to_string(),
        "title-bar-environment-sheet".to_string(),
    ] {
        let start = snapshot(cx, control).bounds().center();
        cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_move(start, MouseButton::Left, Modifiers::default());
        // Moving off the control into the draggable background must still be safe.
        // TestWindow::start_window_move panics if the title bar starts a native drag.
        cx.simulate_mouse_move(blank_title_bar, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(blank_title_bar, MouseButton::Left, Modifiers::default());
        draw(cx);
        assert_eq!(snapshot(cx, "beam-title-bar").bounds(), title_bar);
        assert!(
            fixture.commands.try_recv().is_err(),
            "drag must not activate the control"
        );
    }
}

#[gpui_kit::test]
fn scrolling_keeps_title_bar_buttons_fixed(cx: &mut TestAppContext) {
    let (fixture, cx) = fixture(cx, 12);
    let add = snapshot(cx, "add-workspace").bounds();
    let environment = snapshot(cx, "title-bar-environment-sheet").bounds();
    let scroll = fixture
        .view
        .read_with(cx, |view, _| view.workspace_tabs_scroll_handle.clone());
    assert_eq!(
        cx.update(|window, _| window.viewport_size().width),
        px(800.)
    );
    assert!(scroll.bounds().right() <= add.left());
    assert!(add.right() <= environment.left());
    assert!(
        environment.right() <= px(800.),
        "viewport={:?}, add={add:?}, environment={environment:?}, beam={:?}, title={:?}",
        scroll.bounds(),
        snapshot(cx, "beam-view").bounds(),
        snapshot(cx, "beam-title-bar").bounds()
    );

    cx.simulate_event(ScrollWheelEvent {
        position: scroll.bounds().center(),
        delta: ScrollDelta::Lines(point(0., -3.)),
        ..Default::default()
    });
    draw(cx);
    assert!(
        scroll.offset().x < px(0.),
        "ordinary mouse wheel scrolls horizontally"
    );
    let mouse_offset = scroll.offset().x;
    cx.simulate_event(ScrollWheelEvent {
        position: scroll.bounds().center(),
        delta: ScrollDelta::Pixels(point(px(-80.), px(0.))),
        touch_phase: TouchPhase::Started,
        ..Default::default()
    });
    draw(cx);
    assert!(
        scroll.offset().x < mouse_offset,
        "horizontal trackpad gesture scrolls"
    );
    assert_eq!(snapshot(cx, "add-workspace").bounds(), add);
    assert_eq!(
        snapshot(cx, "title-bar-environment-sheet").bounds(),
        environment
    );
    let offset = scroll.offset();
    fixture.view.update(cx, |_, cx| cx.notify());
    draw(cx);
    assert_eq!(scroll.offset(), offset, "redraw preserves manual scrolling");

    for mode in [ThemeMode::Light, ThemeMode::Dark] {
        cx.update(|_, cx| Theme::change(mode, None, cx));
        for font_size in [14., 16., 18.] {
            cx.update(|_, cx| {
                Theme::global_mut(cx).font_size = px(font_size);
                Theme::sync_base(cx);
                cx.refresh_windows();
            });
            for width in [600., 1000.] {
                cx.simulate_resize(size(px(width), px(600.)));
                draw(cx);
                let add = snapshot(cx, "add-workspace").bounds();
                let environment = snapshot(cx, "title-bar-environment-sheet").bounds();
                assert!(scroll.bounds().right() <= add.left());
                assert!(add.right() <= environment.left());
                assert!(environment.right() <= px(width));
            }
        }
    }
}

#[gpui_kit::test]
fn keyboard_focus_reveals_tabs_and_activation_requests_a_switch(cx: &mut TestAppContext) {
    let (fixture, cx) = fixture(cx, 12);
    let first = fixture.view.read_with(cx, |view, _| {
        view.shell.workspace.all_workspaces[0].workspace_id
    });
    let second = fixture.view.read_with(cx, |view, _| {
        view.shell.workspace.all_workspaces[1].workspace_id
    });
    let last = fixture.view.read_with(cx, |view, _| {
        view.shell.workspace.all_workspaces[11].workspace_id
    });
    cx.update(|window, cx| {
        fixture
            .view
            .update(cx, |view, cx| view.focus_workspace_tab(first, window, cx))
    });
    draw(cx);
    cx.simulate_keystrokes("end");
    draw(cx);
    cx.update(|window, cx| {
        assert_eq!(
            fixture.view.read(cx).focused_workspace_tab(window),
            Some(last)
        )
    });
    let last_tab = snapshot(cx, format!("workspace-tab-{last}"));
    let viewport = fixture
        .view
        .read_with(cx, |view, _| view.workspace_tabs_scroll_handle.bounds());
    assert!(last_tab.bounds().right() <= viewport.right());
    assert!(
        fixture.commands.try_recv().is_err(),
        "moving focus does not switch"
    );
    cx.simulate_keystrokes("home right enter");
    draw(cx);
    assert!(
        matches!(fixture.commands.try_recv(), Ok(AppCommand::SwitchWorkspace { workspace_id, .. }) if workspace_id == second)
    );
    assert_eq!(
        snapshot(cx, format!("workspace-tab-{first}")).selected(),
        Some(true),
        "selection waits for worker confirmation"
    );
    cx.simulate_keystrokes("home space");
    assert!(
        fixture.commands.try_recv().is_err(),
        "active workspace is a no-op"
    );
    cx.simulate_keystrokes("tab");
    draw(cx);
    assert_eq!(snapshot(cx, "add-workspace").focused(), Some(true));
}

#[gpui_kit::test]
fn rename_dialog_submissions_target_the_inactive_workspace(cx: &mut TestAppContext) {
    let (fixture, cx) = fixture(cx, 2);
    let active = fixture
        .view
        .read_with(cx, |view, _| view.shell.workspace.workspace_id);
    let target = fixture.view.read_with(cx, |view, _| {
        view.shell.workspace.all_workspaces[1].workspace_id
    });
    let tab = snapshot(cx, format!("workspace-tab-{target}"))
        .bounds()
        .center();
    cx.simulate_mouse_down(tab, MouseButton::Right, Modifiers::default());
    cx.simulate_mouse_up(tab, MouseButton::Right, Modifiers::default());
    draw(cx);
    assert!(
        fixture.commands.try_recv().is_err(),
        "right-click does not switch workspaces"
    );
    cx.simulate_keystrokes("shift-f10");
    assert!(
        fixture.commands.try_recv().is_err(),
        "keyboard menu does not switch workspaces"
    );
    cx.dispatch_action(WorkspaceMenuRename(target));
    draw(cx);
    cx.simulate_keystrokes("enter");
    draw(cx);
    let command = fixture.commands.try_recv();
    assert!(
        matches!(command, Ok(AppCommand::RenameWorkspace { workspace_id, .. }) if workspace_id == target),
        "command={command:?}"
    );
    assert_eq!(
        fixture
            .view
            .read_with(cx, |view, _| view.shell.workspace.workspace_id),
        active
    );
    cx.update(|window, cx| {
        assert_eq!(
            fixture.view.read(cx).focused_workspace_tab(window),
            Some(target)
        )
    });

    cx.dispatch_action(WorkspaceMenuRename(target));
    draw(cx);
    let submit = snapshot(cx, "workspace-dialog-submit").bounds().center();
    cx.simulate_click(submit, Modifiers::default());
    draw(cx);
    assert!(
        matches!(fixture.commands.try_recv(), Ok(AppCommand::RenameWorkspace { workspace_id, .. }) if workspace_id == target)
    );
    let mut entries = fixture
        .view
        .read_with(cx, |view, _| view.shell.workspace.all_workspaces.clone());
    entries[1].name = "長いワークスペースの名前".repeat(20);
    fixture
        .events
        .send(AppEvent::WorkspaceRenamed {
            workspace: entries[1].clone(),
            all_workspaces: entries.clone(),
            command_id: "rename-test".to_string(),
        })
        .expect("rename event");
    cx.background_executor
        .advance_clock(std::time::Duration::from_millis(25));
    draw(cx);
    let renamed = snapshot(cx, format!("workspace-tab-{target}"));
    assert_eq!(renamed.label(), Some(entries[1].name.as_str()));
    assert!(renamed.bounds().size.width <= px(192.));
    cx.update(|window, cx| {
        assert_eq!(
            fixture.view.read(cx).focused_workspace_tab(window),
            Some(target)
        )
    });
}

#[gpui_kit::test]
fn workspace_actions_support_creation_cancellation_and_final_workspace_protection(
    cx: &mut TestAppContext,
) {
    let (fixture, cx) = fixture(cx, 2);
    let active = fixture.view.read_with(cx, |view, _| {
        view.shell.workspace.workspace_id.expect("active workspace")
    });
    let target = fixture.view.read_with(cx, |view, _| {
        view.shell.workspace.all_workspaces[1].workspace_id
    });
    let add = snapshot(cx, "add-workspace").bounds().center();
    cx.simulate_click(add, Modifiers::default());
    draw(cx);
    cx.simulate_input("Added workspace");
    cx.simulate_keystrokes("enter");
    draw(cx);
    assert!(
        matches!(fixture.commands.try_recv(), Ok(AppCommand::CreateWorkspace { name, .. }) if name == "Added workspace")
    );

    cx.update(|window, cx| {
        fixture
            .view
            .update(cx, |view, cx| view.focus_workspace_tab(target, window, cx))
    });
    draw(cx);
    cx.dispatch_action(WorkspaceMenuDelete(target));
    draw(cx);
    cx.simulate_keystrokes("escape");
    draw(cx);
    assert!(
        fixture.commands.try_recv().is_err(),
        "cancel does not delete"
    );
    cx.dispatch_action(WorkspaceMenuDelete(target));
    draw(cx);
    let delete = snapshot(cx, "delete-workspace-submit").bounds().center();
    cx.simulate_click(delete, Modifiers::default());
    draw(cx);
    assert!(
        matches!(fixture.commands.try_recv(), Ok(AppCommand::DeleteWorkspace { workspace_id, .. }) if workspace_id == target)
    );
    assert_eq!(
        fixture
            .view
            .read_with(cx, |view, _| view.shell.workspace.workspace_id),
        Some(active)
    );

    cx.update(|window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.shell
                .workspace
                .all_workspaces
                .retain(|entry| entry.workspace_id == active);
            view.sync_workspace_tab_focus(window, cx);
            cx.notify();
        })
    });
    draw(cx);
    cx.update(|window, cx| {
        assert_eq!(
            fixture.view.read(cx).focused_workspace_tab(window),
            Some(active)
        )
    });
    cx.dispatch_action(WorkspaceMenuDelete(active));
    draw(cx);
    assert!(fixture.commands.try_recv().is_err());
    cx.update(|window, _| {
        assert!(
            find(window, &[], &"delete-workspace-submit".into()).is_none(),
            "final workspace cannot be deleted"
        )
    });
}
