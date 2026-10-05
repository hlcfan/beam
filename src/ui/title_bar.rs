use gpui_kit::component::{Selectable, button::Button};
use gpui_kit::{
    App, ClickEvent, InteractiveElement as _, IntoElement, MouseButton, MouseMoveEvent,
    ParentElement as _, Pixels, Point, RenderOnce, Styled as _, Window, canvas, div,
};

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

#[derive(Default)]
struct ClickGesture {
    start: Option<Point<Pixels>>,
    dragged: bool,
}

#[cfg(test)]
mod tests;
