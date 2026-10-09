use super::ResponseTab;
use crate::app_shell::{AppShellState, DataSyncRuntime};
use crate::paths::BeamPaths;
use crate::ui::BeamView;
use gpui_kit::component::{
    Root,
    input::{Position, RopeExt as _},
};
use gpui_kit::{
    AppContext as _, Entity, Pixels, Point, TestAppContext, VisualTestContext, point, px, size,
};
use std::sync::mpsc::{channel, sync_channel};
use tempfile::TempDir;
use ulid::Ulid;

struct Fixture {
    view: Entity<BeamView>,
    request_id: Ulid,
    _directory: TempDir,
}

#[gpui_kit::test]
fn wrapped_refresh_preserves_position_in_first_frame_with_highlighting(cx: &mut TestAppContext) {
    assert_first_refresh_frame_preserves_scroll(cx, "json");
}

#[gpui_kit::test]
fn wrapped_refresh_preserves_position_in_first_frame_without_highlighting(cx: &mut TestAppContext) {
    assert_first_refresh_frame_preserves_scroll(cx, "text");
}

#[gpui_kit::test]
fn html_refresh_preserves_first_frame_when_long_attributes_change(cx: &mut TestAppContext) {
    let (fixture, cx) = fixture(cx, true);
    let body = (0..6608)
        .map(|line| {
            format!(
                "    <div title=\"{}\">entry {line}</div>",
                "long attribute ".repeat(if line == 100 { 1000 } else { line % 12 + 1 })
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let changed = body.replace(&"long attribute ".repeat(1000), "short");
    cx.update(|window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.update_response_body_preserving_scroll(body, "html", window, cx);
        });
    });
    draw(cx);
    scroll_to_line(&fixture, 4085, px(3.25), cx);
    let top = line_top(&fixture, 4085, cx);
    assert_refresh_frame(&fixture, changed, "html", 4085, top, cx);
}

#[gpui_kit::test]
fn repeated_sends_preserve_middle_of_large_wrapped_response(cx: &mut TestAppContext) {
    let (fixture, cx) = fixture(cx, true);
    let bodies = [response_body(true), response_body(false)];
    assert_eq!(bodies[0].lines().count(), 6608);
    receive_response(&fixture, &bodies[0], cx);
    scroll_to_line(&fixture, 4085, px(35.25), cx);
    let top = line_top(&fixture, 4085, cx);
    let focus = cx.update(|window, cx| window.focused(cx));

    for index in 0..6 {
        fixture.view.update(cx, |view, cx| {
            view.begin_request_run_for(fixture.request_id);
            cx.notify();
        });
        draw(cx);
        receive_response(&fixture, &bodies[(index + 1) % 2], cx);
        assert_eq!(first_visible_line(&fixture, cx), 4085);
        assert!((line_top(&fixture, 4085, cx) - top).abs() < px(0.1));
        assert_eq!(cx.update(|window, cx| window.focused(cx)), focus);
        fixture.view.read_with(cx, |view, cx| {
            assert_eq!(view.response_body_editor.read(cx).selected_range(), 0..0);
            assert!(view.pending_response_scroll_restoration.is_none());
        });
    }
}

#[gpui_kit::test]
fn unchanged_response_keeps_layout_without_a_restoration_frame(cx: &mut TestAppContext) {
    let (fixture, cx) = fixture(cx, true);
    let body = response_body(true);
    receive_response(&fixture, &body, cx);
    scroll_to_line(&fixture, 4085, px(35.25), cx);
    fixture.view.update(cx, |view, cx| {
        view.response_body_editor.update(cx, |editor, cx| {
            editor.set_selected_range(0..0, cx);
            editor.set_scroll_offset(editor.scroll_offset(), cx);
        });
    });
    draw(cx);
    let offset = scroll_offset(&fixture, cx);
    cx.update(|window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.update_response_body_preserving_scroll(body, "json", window, cx);
            assert!(view.pending_response_scroll_restoration.is_none());
            assert_eq!(view.response_body_editor.read(cx).selected_range(), 0..0);
        });
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert_eq!(scroll_offset(&fixture, cx), offset);
}

#[gpui_kit::test]
fn unwrapped_response_refresh_preserves_both_scroll_axes(cx: &mut TestAppContext) {
    let (fixture, cx) = fixture(cx, false);
    let bodies = [response_body(true), response_body(false)];
    receive_response(&fixture, &bodies[0], cx);
    scroll_to_line(&fixture, 4085, px(3.25), cx);
    fixture.view.update(cx, |view, cx| {
        view.response_body_editor.update(cx, |editor, cx| {
            let mut offset = editor.scroll_offset();
            offset.x = px(-80.);
            editor.set_scroll_offset(offset, cx);
        });
    });
    draw(cx);
    let offset = scroll_offset(&fixture, cx);
    assert!(offset.x < px(0.));
    receive_response(&fixture, &bodies[1], cx);
    assert_eq!(scroll_offset(&fixture, cx), offset);
}

#[gpui_kit::test]
fn wrapped_response_restoration_survives_hidden_body_and_cancels_on_clear(cx: &mut TestAppContext) {
    let (fixture, cx) = fixture(cx, true);
    let bodies = [response_body(true), response_body(false)];
    receive_response(&fixture, &bodies[0], cx);
    scroll_to_line(&fixture, 4085, px(3.25), cx);
    let top = line_top(&fixture, 4085, cx);
    fixture.view.update(cx, |view, cx| {
        view.active_response_tab = ResponseTab::Headers;
        cx.notify();
    });
    draw(cx);
    for body in [&bodies[1], &bodies[0], &bodies[1]] {
        receive_response(&fixture, body, cx);
    }
    fixture.view.update(cx, |view, cx| {
        view.active_response_tab = ResponseTab::Body;
        cx.notify();
    });
    draw(cx);
    assert_eq!(first_visible_line(&fixture, cx), 4085);
    assert!((line_top(&fixture, 4085, cx) - top).abs() < px(0.1));

    cx.update(|window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.update_response_body_preserving_scroll(bodies[0].clone(), "json", window, cx);
            assert!(view.pending_response_scroll_restoration.is_some());
            view.shell
                .workspace_tree
                .set_selected_request(Some(Ulid::new()));
            view.clear_response_pane(window, cx);
            cx.notify();
        })
    });
    draw(cx);
    fixture.view.read_with(cx, |view, cx| {
        assert!(view.response_body_editor.read(cx).value().is_empty());
        assert!(view.pending_response_scroll_restoration.is_none());
        assert_eq!(view.response_body_editor.read(cx).selected_range(), 0..0);
    });
}

#[gpui_kit::test]
fn shorter_response_clamps_scroll_without_a_pending_anchor(cx: &mut TestAppContext) {
    let (fixture, cx) = fixture(cx, true);
    receive_response(&fixture, &response_body(true), cx);
    scroll_to_line(&fixture, 4085, px(3.25), cx);
    receive_response(&fixture, "{\n  \"value\": 1\n}", cx);
    fixture.view.read_with(cx, |view, cx| {
        assert!(view.pending_response_scroll_restoration.is_none());
        assert_eq!(view.response_body_editor.read(cx).scroll_offset().y, px(0.));
    });
    for body in ["", " \n "] {
        receive_response(&fixture, body, cx);
        fixture.view.read_with(cx, |view, cx| {
            assert!(view.pending_response_scroll_restoration.is_none());
            assert_eq!(view.response_body_editor.read(cx).selected_range(), 0..0);
        });
    }
}

fn fixture(cx: &mut TestAppContext, wrap: bool) -> (Fixture, &mut VisualTestContext) {
    cx.update(gpui_kit::init);
    let directory = tempfile::tempdir().expect("fixture directory");
    let paths = BeamPaths::from_root(directory.path().to_path_buf());
    let (command_tx, _commands) = sync_channel(32);
    let (_events, event_rx) = channel();
    let (root, cx) = cx.add_window_view(move |window, cx| {
        let mut shell = AppShellState::default();
        shell.theme.wrap_body_editor = wrap;
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
    cx.simulate_resize(size(px(1200.), px(800.)));
    let request_id = Ulid::new();
    view.update(cx, |view, cx| {
        cx.set_reduce_motion(true);
        view.shell
            .workspace_tree
            .set_selected_request(Some(request_id));
        view.sync_selected_request_pane_data();
        cx.notify();
    });
    draw(cx);
    (
        Fixture {
            view,
            request_id,
            _directory: directory,
        },
        cx,
    )
}

fn response_body(long_name: bool) -> String {
    let values = (0..1101).map(|index| serde_json::json!({
        "id": index,
        "name": if index == 100 && long_name { "wrapped value ".repeat(30) } else { "entry".to_string() },
        "status": "complete",
        "value": "A long value to keep horizontal scrolling available. ".repeat(8),
    })).collect::<Vec<_>>();
    serde_json::to_string_pretty(&values).expect("JSON response")
}

fn assert_first_refresh_frame_preserves_scroll(cx: &mut TestAppContext, language: &'static str) {
    let (fixture, cx) = fixture(cx, true);
    let bodies = [response_body(true), response_body(false)];
    cx.update(|window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.update_response_body_preserving_scroll(bodies[0].clone(), language, window, cx);
        });
    });
    draw(cx);
    scroll_to_line(&fixture, 4085, px(35.25), cx);
    let top = line_top(&fixture, 4085, cx);
    assert_refresh_frame(&fixture, bodies[1].clone(), language, 4085, top, cx);
}

fn assert_refresh_frame(
    fixture: &Fixture,
    body: String,
    language: &'static str,
    line: usize,
    top: Pixels,
    cx: &mut VisualTestContext,
) {
    cx.update(|window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.update_response_body_preserving_scroll(body, language, window, cx);
        });
        window.refresh();
        window.draw(cx).clear(cx);
        let editor = fixture.view.read(cx).response_body_editor.read(cx);
        let offset = editor.text().line_start_offset(line);
        let current_top = editor
            .range_to_bounds(&(offset..offset))
            .expect("anchor bounds")
            .top()
            - editor.input_bounds().top();
        assert_eq!(
            editor.visible_row_range().expect("visible lines").start,
            line
        );
        assert!(
            (current_top - top).abs() < px(0.1),
            "first-frame anchor moved from {top:?} to {current_top:?}"
        );
    });
    draw(cx);
    assert!((line_top(fixture, line, cx) - top).abs() < px(0.1));
}

fn receive_response(fixture: &Fixture, body: &str, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.clear_request_execution_state(fixture.request_id);
            view.update_response_body_preserving_scroll(body.to_string(), "json", window, cx);
        })
    });
    draw(cx);
}

fn scroll_to_line(fixture: &Fixture, line: usize, inset: Pixels, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        fixture
            .view
            .read(cx)
            .response_body_editor
            .clone()
            .update(cx, |editor, cx| {
                editor.set_cursor_position(Position::new(line as u32, 0), window, cx);
            })
    });
    draw(cx);
    cx.update(|_, cx| {
        fixture
            .view
            .read(cx)
            .response_body_editor
            .clone()
            .update(cx, |editor, cx| {
                let offset = editor.text().line_start_offset(line);
                let bounds = editor
                    .range_to_bounds(&(offset..offset))
                    .expect("target line bounds");
                let delta = bounds.top() - editor.input_bounds().top() + inset;
                editor.set_scroll_offset(editor.scroll_offset() - point(px(0.), delta), cx);
            })
    });
    draw(cx);
    assert_eq!(first_visible_line(fixture, cx), line);
}

fn first_visible_line(fixture: &Fixture, cx: &mut VisualTestContext) -> usize {
    fixture.view.read_with(cx, |view, cx| {
        view.response_body_editor
            .read(cx)
            .visible_row_range()
            .expect("visible lines")
            .start
    })
}

fn line_top(fixture: &Fixture, line: usize, cx: &mut VisualTestContext) -> Pixels {
    fixture.view.read_with(cx, |view, cx| {
        let editor = view.response_body_editor.read(cx);
        let offset = editor.text().line_start_offset(line);
        editor
            .range_to_bounds(&(offset..offset))
            .expect("line bounds")
            .top()
            - editor.input_bounds().top()
    })
}

fn scroll_offset(fixture: &Fixture, cx: &mut VisualTestContext) -> Point<Pixels> {
    fixture.view.read_with(cx, |view, cx| {
        view.response_body_editor.read(cx).scroll_offset()
    })
}

fn draw(cx: &mut VisualTestContext) {
    for _ in 0..2 {
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }
}
