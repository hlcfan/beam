use super::{AppShellState, BeamPaths, BeamView, DataSyncRuntime, TreeNodeKind, TreeRowViewModel};
use gpui_kit::base::test_support::{ElementSnapshot, find};
use gpui_kit::component::{ActiveTheme as _, Root};
use gpui_kit::{
    AppContext as _, Context, ElementId, Entity, IntoElement, Modifiers, MouseButton,
    ParentElement, Render, SharedString, Styled, Subscription, TestAppContext, VisualTestContext,
    Window, div, point, px, size,
};
use std::sync::mpsc::{channel, sync_channel};
use tokio::sync::oneshot;
use ulid::Ulid;

struct RowHost {
    view: Entity<BeamView>,
    request_id: Ulid,
    _observation: Subscription,
}

impl Render for RowHost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let row = TreeRowViewModel {
            id: self.request_id,
            kind: TreeNodeKind::Request,
            depth: 0,
            selected: false,
        };
        div().w_64().child(
            self.view
                .update(cx, |view, cx| view.render_tree_row(&row, window, cx)),
        )
    }
}

#[gpui_kit::test]
fn request_row_action_dismisses_tooltip_on_click(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let directory = tempfile::tempdir().expect("fixture directory");
    let paths = BeamPaths::from_root(directory.path().to_path_buf());
    let request_id = Ulid::new();
    let other_request_id = Ulid::new();
    let (cancel_tx, mut cancel_rx) = oneshot::channel();
    let (other_cancel_tx, mut other_cancel_rx) = oneshot::channel();
    let (command_tx, _commands) = sync_channel(32);
    let (_events, event_rx) = channel();
    let (root, cx) = cx.add_window_view(move |window, cx| {
        let view = cx.new(|cx| {
            let mut view = BeamView::new(
                AppShellState::default(),
                vec![],
                DataSyncRuntime {
                    command_tx,
                    event_rx,
                },
                paths,
                window,
                cx,
            );
            view.shell
                .workspace_tree
                .set_selected_request(Some(other_request_id));
            for (id, cancel_tx) in [(request_id, cancel_tx), (other_request_id, other_cancel_tx)] {
                view.begin_request_run_for(id);
                view.request_execution_states
                    .get_mut(&id)
                    .expect("request run")
                    .cancel_tx = Some(cancel_tx);
            }
            view.response_status = "200 OK".into();
            view.response_status_code = Some(200);
            view
        });
        let host = cx.new(|cx| RowHost {
            _observation: cx.observe(&view, |_, _, cx| cx.notify()),
            view,
            request_id,
        });
        Root::new(host, window, cx)
    });
    let view = root.read_with(cx, |root, cx| {
        root.view()
            .clone()
            .downcast::<RowHost>()
            .expect("row host")
            .read(cx)
            .view
            .clone()
    });
    cx.simulate_resize(size(px(400.), px(200.)));
    cx.update(|window, _| window.activate_window());
    // Keep animations static so the test executor can park.
    cx.update(|_, cx| cx.set_reduce_motion(true));
    draw(cx);

    let button_id = format!("tree-row-send-{request_id}");
    let spinner_id = format!("tree-row-spinner-{request_id}");
    let stop_id = format!("tree-row-stop-{request_id}");
    let button = snapshot(cx, &button_id).bounds();
    assert_eq!(button.size.width, button.size.height);
    assert!(snapshot(cx, &spinner_id).visible());
    assert!(!snapshot(cx, &stop_id).visible());

    let row = snapshot(cx, &format!("tree-row-{request_id}")).bounds();
    cx.simulate_mouse_move(row.origin, MouseButton::Left, Modifiers::default());
    draw(cx);
    assert!(snapshot(cx, &spinner_id).visible());
    assert!(!snapshot(cx, &stop_id).visible());

    cx.simulate_mouse_move(button.center(), MouseButton::Left, Modifiers::default());
    draw(cx);
    assert!(!snapshot(cx, &spinner_id).visible());
    assert!(snapshot(cx, &stop_id).visible());
    assert_eq!(snapshot(cx, &button_id).bounds(), button);
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    draw(cx);
    assert!(tooltip_visible(cx, &button_id));

    // Click while the tooltip is visible and keep the pointer stationary.
    cx.simulate_click(button.center(), Modifiers::default());
    draw(cx);
    assert!(!tooltip_visible(cx, &button_id));
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    draw(cx);
    assert!(!tooltip_visible(cx, &button_id));
    assert_eq!(cancel_rx.try_recv(), Ok(()));
    assert_eq!(
        other_cancel_rx.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    );
    view.read_with(cx, |view, _| {
        assert!(!view.is_request_sending(request_id));
        assert!(view.is_request_sending(other_request_id));
        assert_eq!(
            view.shell.workspace_tree.selected_request_id(),
            Some(other_request_id)
        );
        assert_eq!(view.response_status, "200 OK");
        assert_eq!(view.response_status_code, Some(200));
    });
    assert_eq!(
        snapshot(cx, &button_id).label(),
        Some("Send request Unknown")
    );

    let (cancel_tx, mut cancel_rx) = oneshot::channel();
    view.update(cx, |view, cx| {
        view.shell
            .workspace_tree
            .set_selected_request(Some(request_id));
        view.begin_request_run_for(request_id);
        view.request_execution_states
            .get_mut(&request_id)
            .expect("request run")
            .cancel_tx = Some(cancel_tx);
        cx.notify();
    });
    draw(cx);
    assert!(!tooltip_visible(cx, &button_id));
    cx.simulate_click(button.center(), Modifiers::default());
    draw(cx);
    assert_eq!(cancel_rx.try_recv(), Ok(()));
    view.read_with(cx, |view, _| {
        assert!(!view.is_request_sending(request_id));
        assert!(view.is_request_sending(other_request_id));
        assert_eq!(view.response_status, "Canceled");
        assert_eq!(view.response_status_code, None);
    });

    view.update(cx, |view, cx| {
        view.begin_request_run_for(request_id);
        cx.notify();
    });
    draw(cx);
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    draw(cx);
    assert!(!tooltip_visible(cx, &button_id));

    // Completion must not reopen the tooltip beneath a stationary pointer.
    view.update(cx, |view, cx| {
        let run_id = view.request_execution_states[&request_id].run_id;
        crate::ui::request::execution::apply_request_run_completion_status(
            &mut view.request_execution_states,
            request_id,
            run_id,
            true,
        );
        cx.notify();
    });
    draw(cx);
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    draw(cx);
    assert!(!tooltip_visible(cx, &button_id));
    assert_eq!(
        snapshot(cx, &button_id).label(),
        Some("Send request Unknown")
    );

    cx.simulate_mouse_move(
        point(px(350.), px(150.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    draw(cx);
    let idle_button = snapshot(cx, &button_id).bounds();
    cx.simulate_mouse_move(
        idle_button.center(),
        MouseButton::Left,
        Modifiers::default(),
    );
    draw(cx);
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    draw(cx);
    assert!(tooltip_visible(cx, &button_id));

    // The idle Send action dismisses its tooltip too, even if URL validation fails.
    cx.simulate_click(idle_button.center(), Modifiers::default());
    draw(cx);
    assert!(!tooltip_visible(cx, &button_id));
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    draw(cx);
    assert!(!tooltip_visible(cx, &button_id));
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

fn snapshot(cx: &mut VisualTestContext, id: &str) -> ElementSnapshot {
    let id = ElementId::from(SharedString::from(id.to_owned()));
    cx.update(|window, _| find(window, &[], &id).expect("rendered control"))
}

fn tooltip_visible(cx: &mut VisualTestContext, button_id: &str) -> bool {
    // This fixture has no other popovers; a painted popover is the action tooltip.
    let button = snapshot(cx, button_id).bounds();
    cx.update(|window, cx| {
        let button = button.scale(window.scale_factor());
        window.painted_quads().iter().any(|quad| {
            quad.background == cx.theme().tokens.popover.into()
                && quad.bounds.top() >= button.bottom()
        })
    })
}
