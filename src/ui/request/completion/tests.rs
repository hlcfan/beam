use std::sync::mpsc::{channel, sync_channel};

use chrono::Utc;
use gpui_kit::component::Root;
use gpui_kit::component::input::InputEvent;
use gpui_kit::{AppContext as _, Entity, TestAppContext, VisualTestContext};
use ulid::Ulid;

use crate::app_shell::{AppShellState, DataSyncRuntime};
use crate::models::{EnvironmentFile, EnvironmentMeta, EnvironmentScope, EnvironmentVariable};
use crate::paths::BeamPaths;
use crate::ui::BeamView;

#[gpui_kit::test]
fn body_completion_reuses_variables_until_the_token_ends(cx: &mut TestAppContext) {
    let (view, environment_path, _directory, cx) = fixture(cx);
    type_body(&view, "{{", cx);
    cx.run_until_parked();
    assert!(completion_names(&view, cx).contains(&"host".into()));

    // A second read would lose the environment variables.
    std::fs::remove_file(&environment_path).expect("remove environment");
    type_body(&view, "ho", cx);
    cx.run_until_parked();
    assert_eq!(completion_names(&view, cx), ["host"]);

    type_body(&view, "x", cx);
    cx.run_until_parked();
    assert!(completion_names(&view, cx).is_empty());
    view.update(cx, |view, cx| {
        view.request_body_editor.update(cx, |editor, cx| {
            let cursor = editor.cursor();
            editor.set_selected_range(cursor - 1..cursor, cx);
        });
    });
    type_body(&view, "", cx);
    cx.run_until_parked();
    assert_eq!(completion_names(&view, cx), ["host"]);

    type_body(&view, "}} {{ho", cx);
    cx.run_until_parked();
    assert!(completion_names(&view, cx).is_empty());
}

#[gpui_kit::test]
fn body_completion_uses_the_latest_prefix_when_the_load_finishes(cx: &mut TestAppContext) {
    let (view, _, _directory, cx) = fixture(cx);
    // Queue more edits before giving the background load time to finish.
    type_body(&view, "{{", cx);
    type_body(&view, "ho", cx);
    cx.run_until_parked();
    assert_eq!(completion_names(&view, cx), ["host"]);
    view.read_with(cx, |view, cx| {
        let menu = view.request_body_editor.read(cx).completion_menu_state();
        assert_eq!(menu.query, "ho");
        let Some(lsp_types::CompletionTextEdit::Edit(edit)) = &menu.items[0].text_edit else {
            panic!("completion text edit");
        };
        assert_eq!(edit.range.end, lsp_types::Position::new(0, 4));
    });
}

#[gpui_kit::test]
fn body_completion_reloads_variables_when_the_environment_changes(cx: &mut TestAppContext) {
    let (view, _, directory, cx) = fixture(cx);
    type_body(&view, "{{ho", cx);
    cx.run_until_parked();
    assert_eq!(completion_names(&view, cx), ["host"]);

    let other = EnvironmentMeta {
        environment_id: Ulid::new(),
        scope: EnvironmentScope::Global,
        name: "Other".into(),
        file_name: "other.env.toml".into(),
        description: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    let file = EnvironmentFile {
        schema_version: crate::schema::SCHEMA_VERSION_V1,
        environment: other.clone(),
        variables: vec![EnvironmentVariable {
            name: "port".into(),
            value: "80".into(),
            enabled: true,
            description: None,
        }],
        file_path: None,
    };
    std::fs::write(
        directory.path().join("environments").join(&other.file_name),
        toml::to_string(&file).expect("TOML"),
    )
    .expect("environment file");
    view.update(cx, |view, _| {
        view.shell
            .environment_selection
            .active_global_environment_id = Some(other.environment_id);
        view.shell.environments.push(other);
    });
    type_body(&view, "{{po", cx);
    cx.run_until_parked();
    assert_eq!(completion_names(&view, cx), ["port"]);
}

#[gpui_kit::test]
fn body_completion_retries_after_a_failed_load(cx: &mut TestAppContext) {
    let (view, environment_path, _directory, cx) = fixture(cx);
    let content = std::fs::read_to_string(&environment_path).expect("read environment");
    std::fs::remove_file(&environment_path).expect("remove environment");
    type_body(&view, "{{ho", cx);
    cx.run_until_parked();
    assert!(completion_names(&view, cx).is_empty());

    std::fs::write(&environment_path, content).expect("restore environment");
    type_body(&view, "{{h", cx);
    cx.run_until_parked();
    assert_eq!(completion_names(&view, cx), ["host"]);
}

#[gpui_kit::test]
fn body_completion_clears_loaded_variables_on_blur_event(cx: &mut TestAppContext) {
    let (view, environment_path, _directory, cx) = fixture(cx);
    type_body(&view, "{{", cx);
    cx.run_until_parked();
    assert!(completion_names(&view, cx).contains(&"host".into()));
    std::fs::remove_file(environment_path).expect("remove environment");

    view.update(cx, |view, cx| {
        view.request_body_editor.update(cx, |_, cx| {
            cx.emit(InputEvent::Blur);
        });
    });
    cx.run_until_parked();
    assert!(completion_names(&view, cx).is_empty());
    type_body(&view, "ho", cx);
    cx.run_until_parked();
    assert!(completion_names(&view, cx).is_empty());
}

#[gpui_kit::test]
fn body_completion_discards_a_pending_load_when_the_token_ends(cx: &mut TestAppContext) {
    let (view, _, _directory, cx) = fixture(cx);
    type_body(&view, "{{", cx);
    type_body(&view, "}}", cx);
    cx.run_until_parked();
    assert!(completion_names(&view, cx).is_empty());
}

fn fixture(
    cx: &mut TestAppContext,
) -> (
    Entity<BeamView>,
    std::path::PathBuf,
    tempfile::TempDir,
    &mut VisualTestContext,
) {
    cx.update(gpui_kit::init);
    let directory = tempfile::tempdir().expect("fixture directory");
    let paths = BeamPaths::from_root(directory.path().to_path_buf());
    std::fs::create_dir_all(&paths.environments_dir).expect("environment directory");
    let environment = EnvironmentMeta {
        environment_id: Ulid::new(),
        scope: EnvironmentScope::Global,
        name: "Test".into(),
        file_name: "test.env.toml".into(),
        description: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    let environment_path = paths.environments_dir.join(&environment.file_name);
    let file = EnvironmentFile {
        schema_version: crate::schema::SCHEMA_VERSION_V1,
        environment: environment.clone(),
        variables: vec![EnvironmentVariable {
            name: "host".into(),
            value: "example.com".into(),
            enabled: true,
            description: None,
        }],
        file_path: None,
    };
    std::fs::write(&environment_path, toml::to_string(&file).expect("TOML"))
        .expect("environment file");
    let mut shell = AppShellState::default();
    shell.environment_selection.active_global_environment_id = Some(environment.environment_id);
    shell.environments.push(environment);
    let (command_tx, _commands) = sync_channel(32);
    let (_events, event_rx) = channel();
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
    cx.run_until_parked();
    cx.update(|window, cx| {
        let editor = view.read(cx).request_body_editor.clone();
        editor.update(cx, |editor, cx| {
            editor.focus(window, cx);
        });
    });
    (view, environment_path, directory, cx)
}

fn type_body(view: &Entity<BeamView>, text: &str, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        let editor = view.read(cx).request_body_editor.clone();
        editor.update(cx, |editor, cx| {
            editor.replace(text, window, cx);
        });
    });
}

fn completion_names(view: &Entity<BeamView>, cx: &VisualTestContext) -> Vec<String> {
    view.read_with(cx, |view, cx| {
        let menu = view.request_body_editor.read(cx).completion_menu_state();
        if !menu.open {
            return vec![];
        }
        menu.items.iter().map(|item| item.label.clone()).collect()
    })
}
