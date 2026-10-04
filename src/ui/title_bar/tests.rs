use super::on_click_without_drag;
use gpui_kit as gpui;
use gpui_kit::base::test_support::{find, snapshots};
use gpui_kit::component::{
    Root, Selectable, TitleBar,
    button::Button,
    h_flex,
    menu::{PopupMenu, PopupMenuItem},
    popover::Popover,
};
use gpui_kit::{
    App, AppContext as _, Context, DismissEvent, Entity, FocusHandle, Focusable as _,
    InteractiveElement as _, IntoElement, Modifiers, MouseButton, ParentElement as _, Render,
    Styled as _, TestAppContext, VisualTestContext, Window, point, px, size,
};

use std::rc::Rc;

struct Harness {
    activations: usize,
    focus: FocusHandle,
}

impl Render for Harness {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let workspace = click_dropdown_menu(
            Button::new("workspace-picker").label("Workspace"),
            |menu, _, _| menu.item(PopupMenuItem::new("New Workspace")),
            window,
            cx,
        );
        let environment = on_click_without_drag(
            Button::new("title-bar-environment-sheet").label("Environment variables"),
            cx.listener(|this, _, _, _| this.activations += 1),
            window,
            cx,
        );
        TitleBar::new().child(
            h_flex()
                .track_focus(&self.focus)
                .size_full()
                .justify_between()
                .child(workspace)
                .child(environment),
        )
    }
}

fn click_dropdown_menu(
    mut button: Button,
    builder: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
    window: &mut Window,
    cx: &mut App,
) -> Popover {
    let id = button
        .interactivity()
        .element_id
        .clone()
        .expect("title bar button id");
    let style = button.style().clone();
    let state = window.use_keyed_state((id.clone(), "click-menu"), cx, |_, _| {
        ClickMenuState::default()
    });
    let open = state.read(cx).open;
    let activation = state.clone();
    let button = on_click_without_drag(
        button.selected(open),
        move |_, window, cx| {
            activation.update(cx, |state, cx| {
                state.open = !state.open;
                if state.open {
                    state.menu = None;
                }
                cx.notify();
            });
            window.refresh();
        },
        window,
        cx,
    );
    let changes = state.clone();
    let builder = Rc::new(builder);
    Popover::new((id, "popover"))
        .appearance(false)
        .overlay_closable(false)
        .trigger_style(style)
        .trigger(button)
        .open(open)
        .on_open_change(move |open, _, cx| {
            changes.update(cx, |state, cx| {
                state.open = *open;
                if !open {
                    state.menu = None;
                }
                cx.notify();
            });
        })
        .content(move |_, window, cx| {
            if let Some(menu) = state.read(cx).menu.clone() {
                return menu;
            }
            let builder = builder.clone();
            let menu = PopupMenu::build(window, cx, move |menu, window, cx| {
                builder(menu, window, cx)
            });
            menu.focus_handle(cx).focus(window, cx);
            let popover = cx.entity();
            window
                .subscribe(&menu, cx, move |_, _: &DismissEvent, window, cx| {
                    // Restore focus before releasing the menu's focused entity.
                    popover.update(cx, |state, cx| state.dismiss(window, cx));
                    window.refresh();
                })
                .detach();
            state.update(cx, |state, _| state.menu = Some(menu.clone()));
            menu
        })
}

#[derive(Default)]
struct ClickMenuState {
    open: bool,
    menu: Option<Entity<PopupMenu>>,
}

fn fixture(cx: &mut TestAppContext) -> (&mut VisualTestContext, Entity<Harness>) {
    cx.update(gpui_kit::init);
    let (root, cx) = cx.add_window_view(move |window, cx| {
        let view = cx.new(|cx| Harness {
            activations: 0,
            focus: cx.focus_handle(),
        });
        view.read(cx).focus.clone().focus(window, cx);
        Root::new(view, window, cx)
    });
    let harness = root.read_with(cx, |root, _| {
        root.view().clone().downcast::<Harness>().expect("harness")
    });
    cx.simulate_resize(size(px(800.), px(600.)));
    cx.update(|window, _| window.activate_window());
    draw(cx);
    (cx, harness)
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

fn bounds(cx: &mut VisualTestContext, id: &str) -> gpui::Bounds<gpui::Pixels> {
    cx.update(|window, _| {
        find(window, &[], &gpui::ElementId::from(id.to_owned()))
            .expect("rendered control")
            .bounds()
    })
}

fn has_workspace_menu(cx: &mut VisualTestContext) -> bool {
    cx.update(|window, _| {
        snapshots(window)
            .iter()
            .any(|element| element.role() == Some(gpui::Role::Menu))
    })
}

fn press_key(cx: &mut VisualTestContext, key: &str) {
    let keystroke = gpui::Keystroke::parse(key).expect("test key");
    cx.simulate_event(gpui::KeyDownEvent {
        keystroke: keystroke.clone(),
        is_held: false,
        prefer_character_input: false,
    });
    cx.simulate_event(gpui::KeyUpEvent { keystroke });
}

#[gpui_kit::test]
fn title_bar_controls_wait_for_release(cx: &mut TestAppContext) {
    let (cx, harness) = fixture(cx);
    for id in ["workspace-picker", "title-bar-environment-sheet"] {
        let start = bounds(cx, id).center();
        cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
        draw(cx);
        assert!(!has_workspace_menu(cx), "{id} opened a menu on mouse down");
        assert_eq!(harness.read_with(cx, |view, _| view.activations), 0);
        cx.simulate_mouse_up(start, MouseButton::Left, Modifiers::default());
        draw(cx);
        if id == "workspace-picker" {
            assert!(has_workspace_menu(cx), "ordinary click must open the menu");
            cx.simulate_keystrokes("escape");
            draw(cx);
            assert!(!has_workspace_menu(cx), "Escape must dismiss the menu");
        } else {
            assert_eq!(
                harness.read_with(cx, |view, _| view.activations),
                1,
                "ordinary click must activate the button"
            );
        }
        draw(cx);
    }
}

#[gpui_kit::test]
fn title_bar_drag_cancellation_resets_for_the_next_click(cx: &mut TestAppContext) {
    let (cx, harness) = fixture(cx);
    let start = bounds(cx, "title-bar-environment-sheet").center();
    let end = point(start.x + px(6.), start.y);
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(end, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::default());
    draw(cx);
    assert_eq!(harness.read_with(cx, |view, _| view.activations), 0);
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
    let jitter = point(start.x + px(1.), start.y);
    cx.simulate_mouse_move(jitter, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(jitter, MouseButton::Left, Modifiers::default());
    draw(cx);
    assert_eq!(harness.read_with(cx, |view, _| view.activations), 1);
}

#[gpui_kit::test]
fn title_bar_controls_support_keyboard_activation(cx: &mut TestAppContext) {
    let (cx, harness) = fixture(cx);
    cx.simulate_keystrokes("tab");
    draw(cx);
    press_key(cx, "enter");
    draw(cx);
    assert!(has_workspace_menu(cx), "Enter must open the workspace menu");
    cx.simulate_keystrokes("escape");
    draw(cx);
    assert!(!has_workspace_menu(cx));
    cx.simulate_keystrokes("tab");
    draw(cx);
    press_key(cx, "space");
    draw(cx);
    assert_eq!(
        harness.read_with(cx, |view, _| view.activations),
        1,
        "Space must activate the environment button once"
    );
}

#[gpui_kit::test]
fn title_bar_drags_cancel_activation_even_after_returning(cx: &mut TestAppContext) {
    let (cx, harness) = fixture(cx);
    for id in ["workspace-picker", "title-bar-environment-sheet"] {
        let control = bounds(cx, id);
        let start = control.center();
        for end in [
            point(start.x + px(6.), start.y),
            point(start.x, control.bottom() + px(30.)),
        ] {
            cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
            cx.simulate_mouse_move(end, MouseButton::Left, Modifiers::default());
            draw(cx);
            cx.simulate_mouse_move(start, MouseButton::Left, Modifiers::default());
            cx.simulate_mouse_up(start, MouseButton::Left, Modifiers::default());
            draw(cx);
            assert!(!has_workspace_menu(cx), "drag on {id} opened a menu");
            assert_eq!(
                harness.read_with(cx, |view, _| view.activations),
                0,
                "drag on {id} activated the button"
            );
        }
    }
}
