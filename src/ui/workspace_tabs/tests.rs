use super::{WorkspaceMenuDelete, WorkspaceMenuRename, init_workspace_tab_actions};
use crate::app_shell::{AppCommand, AppEvent, AppShellState, DataSyncRuntime};
use crate::models::{
    AuthConfig, BodyConfig, HttpMethod, RequestDefinition, RequestFile, RequestMeta, ScriptConfig,
    WorkspaceEntry,
};
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
    fixture_with_active_workspace(cx, count, 0)
}

fn fixture_with_active_workspace(
    cx: &mut TestAppContext,
    count: usize,
    active_index: usize,
) -> (Fixture, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        init_workspace_tab_actions(cx);
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
        .get(active_index)
        .map(|entry| entry.workspace_id);
    shell.workspace.workspace_name = format!("Workspace {active_index}");
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
    cx.update(|window, cx| window.simulate_next_frame(cx));
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

fn settle_dialog(cx: &mut VisualTestContext) {
    draw(cx);
    // Dialog entrance moves hitboxes using wall-clock time, not the test clock.
    std::thread::sleep(
        *gpui_kit::component::dialog::ANIMATION_DURATION + std::time::Duration::from_millis(10),
    );
    draw(cx);
}

fn snapshot(cx: &mut VisualTestContext, id: impl Into<SharedString>) -> ElementSnapshot {
    let id = gpui::ElementId::from(id.into());
    cx.update(|window, _| find(window, &[], &id).expect("rendered control"))
}

#[gpui_kit::test]
fn workspace_tabs_preserve_independent_tree_scroll_offsets(cx: &mut TestAppContext) {
    let (fixture, cx) = fixture(cx, 3);
    let workspaces = fixture
        .view
        .read_with(cx, |view, _| view.shell.workspace.all_workspaces.clone());
    let mut shells: Vec<_> = [100, 80, 3]
        .into_iter()
        .enumerate()
        .map(|(index, count)| tree_workspace(&workspaces, index, count))
        .collect();
    cx.update(|_, cx| {
        fixture.view.update(cx, |view, cx| {
            view.shell.workspace_tree = shells[0].workspace_tree.clone();
            view.shell.shared_store = shells[0].shared_store.clone();
            cx.notify();
        });
    });
    draw(cx);
    scroll_workspace_tree(&fixture, cx, -850.);
    let first_offset = tree_scroll_offset(&fixture, cx);
    assert!(first_offset.y < px(0.));
    let scroll_bounds = fixture
        .view
        .read_with(cx, |view, _| view.workspace_scroll_handle.bounds());
    let selected_row = shells[0]
        .workspace_tree
        .visible_rows()
        .into_iter()
        .find_map(|row| {
            let id = format!("tree-row-{}", row.id);
            cx.update(|window, _| find(window, &[], &id.into()))
                .filter(|snapshot| scroll_bounds.contains(&snapshot.bounds().center()))
                .map(|snapshot| (row.id, snapshot.bounds()))
        })
        .expect("visible request row");
    cx.simulate_click(selected_row.1.center(), Modifiers::default());
    draw(cx);
    assert_eq!(
        fixture.view.read_with(cx, |view, _| view
            .shell
            .workspace_tree
            .selected_request_id()),
        Some(selected_row.0)
    );
    shells[0]
        .workspace_tree
        .set_selected_request(Some(selected_row.0));

    confirm_workspace_switch(&fixture, cx, &shells[1]);
    assert_eq!(tree_scroll_offset(&fixture, cx), point(px(0.), px(0.)));
    scroll_workspace_tree(&fixture, cx, -430.);
    let second_offset = tree_scroll_offset(&fixture, cx);
    assert!(second_offset.y < px(0.));
    assert_ne!(first_offset, second_offset);

    // A short tree clamps to zero without overwriting either long tree's offset.
    confirm_workspace_switch(&fixture, cx, &shells[2]);
    assert_eq!(tree_scroll_offset(&fixture, cx), point(px(0.), px(0.)));
    confirm_workspace_switch(&fixture, cx, &shells[0]);
    assert_eq!(tree_scroll_offset(&fixture, cx), first_offset);
    assert_eq!(
        snapshot(cx, format!("tree-row-{}", selected_row.0)).bounds(),
        selected_row.1
    );
    confirm_workspace_switch(&fixture, cx, &shells[1]);
    assert_eq!(tree_scroll_offset(&fixture, cx), second_offset);
}

fn tree_workspace(workspaces: &[WorkspaceEntry], index: usize, count: usize) -> AppShellState {
    let mut shell = AppShellState::default();
    shell.workspace.all_workspaces = workspaces.to_vec();
    shell.workspace.workspace_id = Some(workspaces[index].workspace_id);
    shell.workspace.workspace_name = workspaces[index].name.clone();
    for number in 0..count {
        let now = Utc::now();
        shell.insert_request_at_root(
            None,
            &RequestFile {
                meta: RequestMeta {
                    request_id: Ulid::new(),
                    name: format!("Request {number}"),
                    description: None,
                    created_at: now,
                    updated_at: now,
                },
                request: RequestDefinition {
                    method: HttpMethod::Get,
                    url: "https://example.com".to_string(),
                    headers: vec![],
                    query_params: vec![],
                },
                auth: AuthConfig::None,
                body: BodyConfig::None,
                scripts: ScriptConfig::default(),
                file_path: None,
            },
        );
    }
    shell
}

fn scroll_workspace_tree(fixture: &Fixture, cx: &mut VisualTestContext, delta: f32) {
    let bounds = fixture
        .view
        .read_with(cx, |view, _| view.workspace_scroll_handle.bounds());
    cx.simulate_event(ScrollWheelEvent {
        position: bounds.center(),
        delta: ScrollDelta::Pixels(point(px(0.), px(delta))),
        ..Default::default()
    });
    draw(cx);
}

fn tree_scroll_offset(fixture: &Fixture, cx: &mut VisualTestContext) -> gpui::Point<gpui::Pixels> {
    fixture
        .view
        .read_with(cx, |view, _| view.workspace_scroll_handle.offset())
}

fn confirm_workspace_switch(fixture: &Fixture, cx: &mut VisualTestContext, shell: &AppShellState) {
    let workspace_id = shell.workspace.workspace_id.expect("workspace id");
    let tab = snapshot(cx, format!("workspace-tab-{workspace_id}"));
    cx.simulate_click(tab.bounds().center(), Modifiers::default());
    draw(cx);
    assert!(matches!(
        fixture.commands.try_recv(),
        Ok(AppCommand::SwitchWorkspace { workspace_id: target, .. }) if target == workspace_id
    ));
    fixture
        .events
        .send(AppEvent::WorkspaceSwitched {
            workspace_id,
            workspace_name: shell.workspace.workspace_name.clone(),
            all_workspaces: shell.workspace.all_workspaces.clone(),
            workspace_tree: shell.workspace_tree.clone(),
            shared_store: shell.shared_store.clone(),
            request_pane_data: shell.request_pane_data.clone(),
            environments: shell.environments.clone(),
            environment_selection: shell.environment_selection.clone(),
            command_id: "tree-scroll-test".to_string(),
        })
        .expect("workspace switch event");
    cx.background_executor
        .advance_clock(std::time::Duration::from_millis(25));
    draw(cx);
}

#[gpui_kit::test]
fn number_shortcuts_select_workspace_tabs_in_display_order(cx: &mut TestAppContext) {
    let (fixture, cx) = fixture_with_active_workspace(cx, 10, 9);
    let workspaces = fixture
        .view
        .read_with(cx, |view, _| view.shell.workspace.all_workspaces.clone());
    let modifier = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    // Shortcuts also work before any editor or tree row has focus.
    for number in 1..=9 {
        cx.simulate_keystrokes(&format!("{modifier}-{number}"));
        draw(cx);
        assert!(matches!(
            fixture.commands.try_recv(),
            Ok(AppCommand::SwitchWorkspace { workspace_id, .. })
                if workspace_id == workspaces[number - 1].workspace_id
        ));
        assert!(
            fixture.commands.try_recv().is_err(),
            "one switch per shortcut"
        );
    }
}

#[gpui_kit::test]
fn workspace_shortcuts_work_with_editor_and_tree_focus(cx: &mut TestAppContext) {
    let (fixture, cx) = fixture(cx, 2);
    let (second, input, tree_focus) = fixture.view.read_with(cx, |view, _| {
        (
            view.shell.workspace.all_workspaces[1].workspace_id,
            view.url_input.clone(),
            view.tree_focus_handle.clone(),
        )
    });
    let shortcut = if cfg!(target_os = "macos") {
        "cmd-2"
    } else {
        "ctrl-2"
    };
    for focus_editor in [true, false] {
        cx.update(|window, cx| {
            if focus_editor {
                input.update(cx, |input, cx| input.focus(window, cx));
            } else {
                tree_focus.focus(window, cx);
            }
        });
        draw(cx);
        let focus = cx.update(|window, cx| window.focused(cx));
        cx.simulate_keystrokes(shortcut);
        draw(cx);
        assert!(matches!(
            fixture.commands.try_recv(),
            Ok(AppCommand::SwitchWorkspace { workspace_id, .. }) if workspace_id == second
        ));
        cx.update(|window, cx| assert_eq!(window.focused(cx), focus));
    }
}

#[gpui_kit::test]
fn workspace_shortcuts_ignore_active_and_missing_tabs(cx: &mut TestAppContext) {
    let (fixture, cx) = fixture(cx, 2);
    let shortcuts = if cfg!(target_os = "macos") {
        "cmd-1 cmd-3 cmd-9"
    } else {
        "ctrl-1 ctrl-3 ctrl-9"
    };
    cx.simulate_keystrokes(shortcuts);
    draw(cx);
    assert!(fixture.commands.try_recv().is_err());
}

#[gpui_kit::test]
fn startup_reveals_the_active_workspace_tab(cx: &mut TestAppContext) {
    let (fixture, cx) = fixture_with_active_workspace(cx, 12, 11);
    let (active, scroll) = fixture.view.read_with(cx, |view, _| {
        (
            view.shell.workspace.workspace_id.expect("active workspace"),
            view.workspace_tabs_scroll_handle.clone(),
        )
    });
    let tab = snapshot(cx, format!("workspace-tab-{active}"));
    assert_eq!(tab.selected(), Some(true));
    assert!(
        scroll.offset().x < px(0.),
        "startup scrolls to the active tab"
    );
    assert!(tab.bounds().left() >= scroll.bounds().left());
    assert!(tab.bounds().right() <= scroll.bounds().right());
    assert!(fixture.commands.try_recv().is_err());
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
        let selected_background = cx.update(|window, cx| {
            let painted_tab = window
                .painted_quads()
                .into_iter()
                .find(|quad| quad.bounds == tab.scale(window.scale_factor()))
                .expect("painted tab background");
            if mode.is_dark() {
                assert_ne!(painted_tab.background, Theme::global(cx).background.into());
                assert!(!window.painted_quads().into_iter().any(|quad| {
                    quad.bounds == tab.scale(window.scale_factor()) && quad.border_color.a > 0.
                }));
            } else {
                assert_eq!(painted_tab.background, Theme::global(cx).background.into());
            }
            painted_tab.background
        });
        for button in [MouseButton::Left, MouseButton::Right] {
            cx.simulate_mouse_down(tab.center(), button, Modifiers::default());
            // Check while pressed; native menu tracking can also consume mouse-up.
            draw(cx);
            cx.update(|window, _| {
                let painted_tab = window
                    .painted_quads()
                    .into_iter()
                    .find(|quad| quad.bounds == tab.scale(window.scale_factor()))
                    .expect("painted tab background");
                assert_eq!(
                    painted_tab.background, selected_background,
                    "{button:?}-click preserves the selected background in {mode:?}"
                );
            });
            cx.simulate_mouse_up(tab.center(), button, Modifiers::default());
            draw(cx);
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
fn clicking_workspace_tabs_switches_without_taking_keyboard_focus(cx: &mut TestAppContext) {
    let (fixture, cx) = fixture(cx, 2);
    let (first, second, input) = fixture.view.read_with(cx, |view, _| {
        (
            view.shell.workspace.all_workspaces[0].workspace_id,
            view.shell.workspace.all_workspaces[1].workspace_id,
            view.url_input.clone(),
        )
    });
    cx.update(|window, cx| input.update(cx, |input, cx| input.focus(window, cx)));
    draw(cx);
    let focus = cx.update(|window, cx| window.focused(cx).expect("URL input focused"));
    for workspace_id in [first, second] {
        let tab = snapshot(cx, format!("workspace-tab-{workspace_id}")).bounds();
        cx.simulate_click(tab.center(), Modifiers::default());
        draw(cx);
        cx.update(|window, cx| assert_eq!(window.focused(cx), Some(focus.clone())));
        if workspace_id == first {
            assert!(
                fixture.commands.try_recv().is_err(),
                "active workspace is a no-op"
            );
        } else {
            assert!(matches!(
                fixture.commands.try_recv(),
                Ok(AppCommand::SwitchWorkspace { workspace_id, .. }) if workspace_id == second
            ));
        }
    }
    assert_eq!(
        snapshot(cx, format!("workspace-tab-{first}")).selected(),
        Some(true),
        "selection waits for worker confirmation"
    );
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
    // Linux's drawn native-menu fallback takes focus outside Beam's action scope.
    // Dismiss it before dispatching the menu action directly.
    cx.simulate_keystrokes("escape");
    draw(cx);
    cx.dispatch_action(WorkspaceMenuRename(target));
    draw(cx);
    snapshot(cx, "workspace-dialog-submit");
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

    cx.dispatch_action(WorkspaceMenuRename(target));
    settle_dialog(cx);
    let submit = snapshot(cx, "workspace-dialog-submit").bounds().center();
    cx.simulate_click(submit, Modifiers::default());
    draw(cx);
    let command = fixture.commands.try_recv();
    assert!(
        matches!(command, Ok(AppCommand::RenameWorkspace { workspace_id, .. }) if workspace_id == target),
        "command={command:?}"
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

    let tab = snapshot(cx, format!("workspace-tab-{target}"))
        .bounds()
        .center();
    cx.simulate_mouse_down(tab, MouseButton::Right, Modifiers::default());
    cx.simulate_mouse_up(tab, MouseButton::Right, Modifiers::default());
    draw(cx);
    cx.simulate_keystrokes("escape");
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
    settle_dialog(cx);
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

    cx.update(|_, cx| {
        fixture.view.update(cx, |view, cx| {
            view.shell
                .workspace
                .all_workspaces
                .retain(|entry| entry.workspace_id == active);
            cx.notify();
        })
    });
    draw(cx);
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
