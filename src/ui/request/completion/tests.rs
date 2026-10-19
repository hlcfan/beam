use std::sync::mpsc::{channel, sync_channel};

use chrono::Utc;
use gpui_kit::component::Root;
use gpui_kit::component::input::InputEvent;
use gpui_kit::{AppContext as _, Entity, TestAppContext, VisualTestContext};
use ulid::Ulid;

use crate::app_shell::{AppEvent, AppShellState, DataSyncRuntime};
use crate::models::{EnvironmentFile, EnvironmentMeta, EnvironmentScope, EnvironmentVariable};
use crate::paths::BeamPaths;
use crate::ui::BeamView;

#[gpui_kit::test]
fn body_completion_reuses_variables_across_tokens(cx: &mut TestAppContext) {
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
    assert_eq!(completion_names(&view, cx), ["host"]);
    view.read_with(cx, |view, cx| {
        let menu = view.request_body_editor.read(cx).completion_menu_state();
        let Some(lsp_types::CompletionTextEdit::Edit(edit)) = &menu.items[0].text_edit else {
            panic!("completion text edit");
        };
        assert_eq!(edit.range.start, lsp_types::Position::new(0, 7));
        assert_eq!(edit.range.end, lsp_types::Position::new(0, 11));
    });
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
fn completion_shares_variables_across_editors_and_menu_dismissals(cx: &mut TestAppContext) {
    let (view, environment_path, _directory, cx) = fixture(cx);
    type_body(&view, "{{ho", cx);
    cx.run_until_parked();
    assert_eq!(completion_names(&view, cx), ["host"]);
    std::fs::remove_file(environment_path).expect("remove environment");

    type_url(&view, "{{ho", cx);
    cx.run_until_parked();
    view.update(cx, |view, cx| {
        assert!(view.url_completion_active(cx));
        view.dismiss_url_completion(cx);
        assert!(!view.url_completion_active(cx));
    });
    type_url(&view, "https://{{ho", cx);
    cx.run_until_parked();
    cx.update(|window, cx| {
        view.update(cx, |view, cx| view.accept_url_completion(None, window, cx));
    });
    view.read_with(cx, |view, cx| {
        assert_eq!(view.url_input.read(cx).value().as_ref(), "https://{{host}}");
    });
}

#[gpui_kit::test]
fn dismissing_body_completion_retains_a_pending_environment_load(cx: &mut TestAppContext) {
    let (view, environment_path, _directory, cx) = fixture(cx);
    type_body(&view, "{{ho", cx);
    view.update(cx, |view, cx| {
        assert!(view.completion_environment_task.is_some());
        view.dismiss_request_body_completion(cx);
        assert!(view.completion_environment_task.is_some());
    });
    cx.run_until_parked();
    assert!(completion_names(&view, cx).is_empty());
    std::fs::remove_file(environment_path).expect("remove environment");
    type_body(&view, "s", cx);
    cx.run_until_parked();
    assert_eq!(completion_names(&view, cx), ["host"]);
}

#[gpui_kit::test]
fn discarded_environment_load_clears_pending_presentations(cx: &mut TestAppContext) {
    let (view, _, _directory, cx) = fixture(cx);
    type_body(&view, "{{ho", cx);
    type_url(&view, "{{ho", cx);
    view.update(cx, |view, _| {
        assert!(view.request_body_completion_pending);
        assert!(view.url_completion_pending);
        // Bypass explicit invalidation to exercise the callback's stale-result guard.
        view.shell
            .environment_selection
            .active_global_environment_id = None;
    });
    cx.run_until_parked();
    view.read_with(cx, |view, _| {
        assert!(!view.request_body_completion_pending);
        assert!(!view.url_completion_pending);
        assert!(view.completion_environment_task.is_none());
        assert!(view.completion_environment_variables.is_none());
    });
}

#[gpui_kit::test]
fn replacing_the_body_editor_keeps_the_environment_snapshot(cx: &mut TestAppContext) {
    let (view, environment_path, _directory, cx) = fixture(cx);
    type_body(&view, "{{ho", cx);
    cx.run_until_parked();
    std::fs::remove_file(environment_path).expect("remove environment");
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.request_body_editor =
                cx.new(|cx| gpui_kit::component::input::EditorState::new(window, cx));
            view.resubscribe_request_body_editor(window, cx);
            view.request_body_editor
                .update(cx, |editor, cx| editor.focus(window, cx));
        });
    });
    type_body(&view, "{{ho", cx);
    cx.run_until_parked();
    assert_eq!(completion_names(&view, cx), ["host"]);
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
    view.update(cx, |view, cx| {
        view.set_selected_environment_for_view(other.environment_id, cx);
        view.shell.environments.push(other);
    });
    assert!(completion_names(&view, cx).is_empty());
    view.update(cx, |view, cx| {
        view.request_body_editor.update(cx, |editor, cx| {
            editor.set_selected_range(2..4, cx);
        });
    });
    type_body(&view, "po", cx);
    cx.run_until_parked();
    assert_eq!(completion_names(&view, cx), ["port"]);
}

#[gpui_kit::test]
fn clearing_the_environment_dismisses_body_completion_without_blurring(cx: &mut TestAppContext) {
    let (view, _, _directory, cx) = fixture(cx);
    type_body(&view, "{{", cx);
    cx.run_until_parked();
    assert!(completion_names(&view, cx).contains(&"host".into()));
    view.update(cx, |view, cx| {
        view.clear_selected_environment_for_view(cx);
    });
    assert!(completion_names(&view, cx).is_empty());

    type_body(&view, "$g", cx);
    cx.run_until_parked();
    assert_eq!(completion_names(&view, cx), ["$guid"]);
}

#[gpui_kit::test]
fn clearing_the_environment_dismisses_url_completion_and_clears_hover_cache(
    cx: &mut TestAppContext,
) {
    let (view, _, _directory, cx) = fixture(cx);
    cx.update(|window, cx| {
        let input = view.read(cx).url_input.clone();
        input.update(cx, |input, cx| {
            input.focus(window, cx);
            input.replace_all("{{ho", window, cx);
        });
    });
    cx.run_until_parked();
    view.update(cx, |view, cx| {
        assert!(view.url_completion_active(cx));
        view.env_var_resolved_cache = Some((
            view.selected_environment_id_for_view(),
            [("host".into(), "example.com".into())].into(),
        ));
        view.clear_selected_environment_for_view(cx);
        assert!(view.env_var_resolved_cache.is_none());
        assert!(!view.url_completion_active(cx));
    });
}

#[gpui_kit::test]
fn clearing_the_environment_cancels_pending_body_completion(cx: &mut TestAppContext) {
    let (view, _, _directory, cx) = fixture(cx);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.request_body_editor.update(cx, |editor, cx| {
                editor.replace("{{ho", window, cx);
            });
            view.update_request_body_completion(window, cx);
            assert!(view.completion_environment_task.is_some());
            view.clear_selected_environment_for_view(cx);
            assert!(view.completion_environment_task.is_none());
        });
    });
    cx.run_until_parked();
    assert!(completion_names(&view, cx).is_empty());
}

#[gpui_kit::test]
fn reselecting_the_environment_keeps_loaded_variables(cx: &mut TestAppContext) {
    let (view, environment_path, _directory, cx) = fixture(cx);
    type_body(&view, "{{", cx);
    cx.run_until_parked();
    std::fs::remove_file(environment_path).expect("remove environment");
    view.update(cx, |view, cx| {
        let id = view
            .selected_environment_id_for_view()
            .expect("selected environment");
        view.set_selected_environment_for_view(id, cx);
    });
    assert!(completion_names(&view, cx).contains(&"host".into()));
    type_body(&view, "ho", cx);
    cx.run_until_parked();
    assert_eq!(completion_names(&view, cx), ["host"]);
}

#[gpui_kit::test]
fn saving_the_environment_invalidates_cache_without_closing_body_completion(
    cx: &mut TestAppContext,
) {
    let (view, environment_path, _directory, cx) = fixture(cx);
    type_body(&view, "{{", cx);
    cx.run_until_parked();
    let names = completion_names(&view, cx);
    assert!(names.contains(&"host".into()));

    let content = std::fs::read_to_string(&environment_path).expect("read environment");
    let mut file: EnvironmentFile = toml::from_str(&content).expect("environment TOML");
    file.variables[0].name = "port".into();
    std::fs::write(&environment_path, toml::to_string(&file).expect("TOML"))
        .expect("save environment");
    apply_environment_event(
        &view,
        AppEvent::EnvironmentUpserted {
            environment: file.environment,
            command_id: "completion-cache-test".into(),
        },
        cx,
    );
    assert_eq!(completion_names(&view, cx), names);
    view.read_with(cx, |view, _| {
        assert!(view.completion_environment_variables.is_none())
    });

    type_body(&view, "po", cx);
    cx.run_until_parked();
    assert_eq!(completion_names(&view, cx), ["port"]);
}

#[gpui_kit::test]
fn saving_the_environment_keeps_url_completion_open(cx: &mut TestAppContext) {
    let (view, _, _directory, cx) = fixture(cx);
    type_url(&view, "{{ho", cx);
    cx.run_until_parked();
    let environment = view.read_with(cx, |view, cx| {
        assert!(view.url_completion_active(cx));
        view.shell.environments[0].clone()
    });
    apply_environment_event(
        &view,
        AppEvent::EnvironmentUpserted {
            environment,
            command_id: "url-completion-save-test".into(),
        },
        cx,
    );
    view.read_with(cx, |view, cx| {
        assert!(view.url_completion_active(cx));
        assert!(view.completion_environment_variables.is_none());
    });
}

#[gpui_kit::test]
fn saving_another_environment_preserves_the_selected_environment_cache(cx: &mut TestAppContext) {
    let (view, environment_path, _directory, cx) = fixture(cx);
    type_body(&view, "{{ho", cx);
    cx.run_until_parked();
    let mut other = view.read_with(cx, |view, _| view.shell.environments[0].clone());
    other.environment_id = Ulid::new();
    std::fs::remove_file(environment_path).expect("remove environment");
    apply_environment_event(
        &view,
        AppEvent::EnvironmentUpserted {
            environment: other,
            command_id: "other-environment-test".into(),
        },
        cx,
    );
    type_body(&view, "s", cx);
    cx.run_until_parked();
    assert_eq!(completion_names(&view, cx), ["host"]);
}

#[gpui_kit::test]
fn deleting_the_selected_environment_invalidates_cached_variables(cx: &mut TestAppContext) {
    let (view, _, _directory, cx) = fixture(cx);
    type_body(&view, "{{", cx);
    cx.run_until_parked();
    let id = view.read_with(cx, |view, _| {
        view.selected_environment_id_for_view()
            .expect("selected environment")
    });
    apply_environment_event(
        &view,
        AppEvent::EnvironmentDeleted {
            environment_id: id,
            command_id: "delete-environment-test".into(),
        },
        cx,
    );
    assert!(completion_names(&view, cx).is_empty());
    view.read_with(cx, |view, _| {
        assert!(view.completion_environment_variables.is_none())
    });
    type_body(&view, "$g", cx);
    cx.run_until_parked();
    assert_eq!(completion_names(&view, cx), ["$guid"]);
}

#[gpui_kit::test]
fn clearing_the_environment_cancels_pending_url_completion(cx: &mut TestAppContext) {
    let (view, _, _directory, cx) = fixture(cx);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.url_input.update(cx, |input, cx| {
                input.focus(window, cx);
                input.replace_all("{{ho", window, cx);
            });
            view.update_url_completion(window, cx);
            assert!(view.completion_environment_task.is_some());
            view.clear_selected_environment_for_view(cx);
            assert!(view.completion_environment_task.is_none());
        });
    });
    cx.run_until_parked();
    view.read_with(cx, |view, cx| assert!(!view.url_completion_active(cx)));
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
fn body_completion_keeps_loaded_variables_after_blur_event(cx: &mut TestAppContext) {
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
    assert_eq!(completion_names(&view, cx), ["host"]);
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

fn type_url(view: &Entity<BeamView>, text: &str, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        let input = view.read(cx).url_input.clone();
        input.update(cx, |input, cx| {
            input.focus(window, cx);
            input.replace_all(text.to_string(), window, cx);
        });
    });
}

fn apply_environment_event(view: &Entity<BeamView>, event: AppEvent, cx: &mut VisualTestContext) {
    let (events, event_rx) = channel();
    view.update(cx, |view, _| view.app_event_rx = event_rx);
    events.send(event).expect("environment event");
    cx.background_executor
        .advance_clock(std::time::Duration::from_millis(25));
    cx.run_until_parked();
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
