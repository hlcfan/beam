use gpui_kit::component::{Selectable, button::Button, menu::PopupMenu, popover::Popover};
use gpui_kit::{
    App, ClickEvent, Context, DismissEvent, Entity, Focusable as _, InteractiveElement as _,
    IntoElement, MouseButton, MouseMoveEvent, ParentElement as _, Pixels, Point, RenderOnce,
    Styled as _, Window, canvas, div,
};
use std::rc::Rc;

// Match GPUI's drag threshold, while keeping small pointer jitter clickable.
const DRAG_THRESHOLD: f64 = 2.;

pub(super) fn on_click_without_drag(
    mut button: Button,
    handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) -> TitleBarButton {
    let id = button
        .interactivity()
        .element_id
        .clone()
        .expect("title bar button id");
    let gesture = window.use_keyed_state((id, "click-gesture"), cx, |_, _| ClickGesture::default());
    let press = gesture.clone();
    let movement = gesture.clone();
    let button = button
        .capture_any_mouse_down(move |event, _, cx| {
            press.update(cx, |gesture, _| {
                gesture.start = (event.button == MouseButton::Left).then_some(event.position);
                gesture.dragged = false;
            });
        })
        .on_click(move |event, window, cx| {
            let accepted = match event {
                ClickEvent::Keyboard(_) | ClickEvent::Touch(_) => true,
                ClickEvent::Mouse(event) => {
                    !gesture.read(cx).dragged
                        && (event.up.position - event.down.position).magnitude() <= DRAG_THRESHOLD
                }
            };
            if accepted {
                handler(event, window, cx);
            }
        })
        .child(
            canvas(
                |_, _, _| (),
                move |_, _, window, _| {
                    // Observe the whole gesture, including movement outside the button
                    // and a return to the original press position before release.
                    window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                        if phase.capture() {
                            movement.update(cx, |gesture, _| {
                                if event.pressed_button == Some(MouseButton::Left) {
                                    if let Some(start) = gesture.start {
                                        gesture.dragged |=
                                            (event.position - start).magnitude() > DRAG_THRESHOLD;
                                    }
                                } else {
                                    gesture.start = None;
                                }
                            });
                        }
                    });
                },
            )
            .absolute()
            .size_full(),
        );
    TitleBarButton { button }
}

#[derive(IntoElement)]
pub(super) struct TitleBarButton {
    button: Button,
}

impl Selectable for TitleBarButton {
    fn selected(mut self, selected: bool) -> Self {
        self.button = self.button.selected(selected);
        self
    }

    fn is_selected(&self) -> bool {
        self.button.is_selected()
    }
}

impl RenderOnce for TitleBarButton {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        // Stop the press after Button records it, before Popover or TitleBar sees it.
        div()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(self.button)
    }
}

pub(super) fn click_dropdown_menu(
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
struct ClickGesture {
    start: Option<Point<Pixels>>,
    dragged: bool,
}

#[derive(Default)]
struct ClickMenuState {
    open: bool,
    menu: Option<Entity<PopupMenu>>,
}

#[cfg(test)]
mod tests;
