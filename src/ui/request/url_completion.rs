//! Variable completion for the URL input, composed around a plain `Input`.
//!
//! The input knows nothing about completion. This module observes its edits,
//! owns the candidates and the selection, captures the navigation keys before
//! the input handles them, renders the popup, and applies the chosen edit with
//! the input's own `set_selected_range` and `replace`.

use std::ops::Range;
use std::sync::Arc;

use gpui_kit::Focusable;
use gpui_kit::component::input::{Escape, MoveDown, MoveUp, Rope};

use super::super::*;
use super::completion::{
    VariableCandidate, load_environment_variables, token_at_cursor, variable_candidates,
};
use crate::models::EnvironmentVariable;

/// Height at which the list starts to scroll.
const MAX_LIST_HEIGHT: Pixels = px(240.);

/// The variables available for the `{{` token under the cursor, and the ones
/// matching what has been typed of it.
///
/// The environment is read once, when the token is opened. Typing more of the
/// same token only filters `variables` again, so it does no file access.
pub(in crate::ui) struct UrlCompletion {
    /// Where the `{{` token starts. A different start is a different token.
    token_start: usize,
    variables: Arc<Vec<EnvironmentVariable>>,
    /// Cursor offset the candidates were computed for. The popup is shown only
    /// while the cursor is still there, so moving it away hides a stale list.
    cursor: usize,
    candidates: Vec<VariableCandidate>,
    selected: usize,
    scroll_handle: ScrollHandle,
}

impl UrlCompletion {
    fn new(token_start: usize, variables: Arc<Vec<EnvironmentVariable>>) -> Self {
        Self {
            token_start,
            variables,
            cursor: token_start,
            candidates: Vec::new(),
            selected: 0,
            scroll_handle: ScrollHandle::new(),
        }
    }

    /// Recomputes the candidates for `prefix` from the loaded variables.
    fn refilter(&mut self, cursor: usize, prefix: &str) {
        self.cursor = cursor;
        self.candidates = variable_candidates(prefix, &self.variables);
        self.selected = 0;
        self.scroll_handle = ScrollHandle::new();
    }
}

impl BeamView {
    /// Recomputes the candidates after the URL text or cursor changed.
    pub(in crate::ui) fn update_url_completion(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = self.url_input.read(cx);
        if !input.focus_handle(cx).is_focused(window) {
            self.dismiss_url_completion(cx);
            return;
        }
        let cursor = input.cursor();
        let Some(token) = token_at_cursor(input.text(), cursor) else {
            self.dismiss_url_completion(cx);
            return;
        };

        // The same token as the open popup: filter what is already loaded.
        if let Some(completion) = self.url_completion.as_mut()
            && completion.token_start == token.start
        {
            completion.refilter(cursor, &token.prefix);
            cx.notify();
            return;
        }
        // The same token is still loading; its result is built from the text
        // as it is when the load finishes.
        if self
            .url_completion_task
            .as_ref()
            .is_some_and(|(start, _)| *start == token.start)
        {
            return;
        }

        // A new token: read the environment once.
        self.dismiss_url_completion(cx);
        let start = token.start;
        let path = self
            .selected_environment_id_for_view()
            .and_then(|id| self.environment_file_path_from_shell(id));
        let variables =
            cx.background_spawn(async move { Arc::new(load_environment_variables(path)) });
        let task = cx.spawn_in(window, async move |this, cx| {
            let variables = variables.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.url_completion_task = None;
                let input = this.url_input.read(cx);
                if !input.focus_handle(cx).is_focused(window) {
                    return;
                }
                let cursor = input.cursor();
                let Some(token) = token_at_cursor(input.text(), cursor) else {
                    return;
                };
                if token.start != start {
                    return;
                }
                let mut completion = UrlCompletion::new(start, variables);
                completion.refilter(cursor, &token.prefix);
                this.url_completion = Some(completion);
                cx.notify();
            });
        });
        self.url_completion_task = Some((start, task));
    }

    pub(in crate::ui) fn dismiss_url_completion(&mut self, cx: &mut Context<Self>) {
        self.url_completion_task = None;
        if self.url_completion.take().is_some() {
            cx.notify();
        }
    }

    /// Whether the popup is showing, and so owns Up, Down, Escape and Enter.
    pub(in crate::ui) fn url_completion_active(&self, cx: &App) -> bool {
        self.url_completion.as_ref().is_some_and(|completion| {
            !completion.candidates.is_empty()
                && completion.cursor == self.url_input.read(cx).cursor()
        })
    }

    /// Replaces the typed token with the selected (or clicked) candidate.
    pub(in crate::ui) fn accept_url_completion(
        &mut self,
        index: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(completion) = self.url_completion.take() else {
            return;
        };
        self.url_completion_task = None;
        let Some(candidate) = completion
            .candidates
            .get(index.unwrap_or(completion.selected))
        else {
            return;
        };
        let insertion = candidate.insertion();
        self.url_input.update(cx, |input, cx| {
            // The text may have changed since the candidates appeared, so the
            // range is worked out again from the current text and cursor.
            let range = replacement_range(input.text(), input.cursor());
            input.set_selected_range(range, cx);
            input.replace(insertion, window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    fn move_url_completion(&mut self, step: isize, cx: &mut Context<Self>) {
        let Some(completion) = self.url_completion.as_mut() else {
            return;
        };
        let count = completion.candidates.len() as isize;
        if count == 0 {
            return;
        }
        completion.selected = (completion.selected as isize + step).rem_euclid(count) as usize;
        completion.scroll_handle.scroll_to_item(completion.selected);
        cx.notify();
    }

    /// Takes the navigation keys ahead of the input while the popup is showing.
    /// Otherwise the actions fall through and the input handles them as usual.
    pub(in crate::ui) fn capture_url_completion_keys(
        &self,
        container: Stateful<Div>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        container
            .capture_action(cx.listener(|this, _: &MoveUp, _, cx| {
                if this.url_completion_active(cx) {
                    this.move_url_completion(-1, cx);
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &MoveDown, _, cx| {
                if this.url_completion_active(cx) {
                    this.move_url_completion(1, cx);
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &Escape, _, cx| {
                if this.url_completion_active(cx) {
                    this.dismiss_url_completion(cx);
                    cx.stop_propagation();
                }
            }))
    }

    /// The popup, with its left edge under the caret, when there is something to show.
    pub(in crate::ui) fn render_url_completion(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.url_completion_active(cx) {
            return None;
        }
        let completion = self.url_completion.as_ref()?;
        let selected = completion.selected;
        let scroll_handle = completion.scroll_handle.clone();
        let rows: Vec<(usize, String, &'static str)> = completion
            .candidates
            .iter()
            .enumerate()
            .map(|(index, candidate)| (index, candidate.name.clone(), candidate.detail))
            .collect();
        let view = cx.entity();

        // The content is built in prepaint, once the caret position of this
        // frame is known; see `CaretAnchored`.
        Some(
            deferred(CaretAnchored::new(
                self.url_input.downgrade(),
                move |window, cx| {
                    let rows = rows.iter().map(|(index, name, detail)| {
                        let index = *index;
                        h_flex()
                            .id(("url-completion-row", index))
                            .px_2()
                            .py_1()
                            .gap_3()
                            .justify_between()
                            .text_xs()
                            .rounded(cx.theme().radius)
                            .cursor_pointer()
                            .when(index == selected, |row| {
                                row.bg(cx.theme().accent)
                                    .text_color(cx.theme().accent_foreground)
                            })
                            .when(index != selected, |row| {
                                row.hover(|row| row.bg(cx.theme().accent.opacity(0.5)))
                            })
                            .child(name.clone())
                            .child(
                                div()
                                    .italic()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(*detail),
                            )
                            // Mouse down, not click: it lands before the input could lose focus.
                            .on_mouse_down(
                                MouseButton::Left,
                                window.listener_for(&view, move |this, _, window, cx| {
                                    this.accept_url_completion(Some(index), window, cx);
                                    cx.stop_propagation();
                                }),
                            )
                    });
                    v_flex()
                        .occlude()
                        .relative()
                        .popover_style(cx)
                        .min_w(px(240.))
                        .p_1()
                        .child(
                            v_flex()
                                .id("url-completion-list")
                                .items_stretch()
                                .max_h(MAX_LIST_HEIGHT)
                                .overflow_y_scroll()
                                .track_scroll(&scroll_handle)
                                .children(rows),
                        )
                        .vertical_scrollbar(&scroll_handle)
                        .into_any_element()
                },
            ))
            .with_priority(2)
            .into_any_element(),
        )
    }
}

/// A popup placed under the caret from this frame's layout.
///
/// The input records where its caret is while it prepaints. Reading that in
/// `render` would use the previous frame's position, so a menu that opens as
/// the user types would appear one keystroke behind and then jump. Inside
/// `deferred`, this element's prepaint runs after the input's, so the position
/// is current, and the content is built and laid out there.
type BuildContent = Box<dyn Fn(&mut Window, &mut App) -> AnyElement>;

struct CaretAnchored {
    input: WeakEntity<InputState>,
    build: BuildContent,
    content: Option<AnyElement>,
}

impl CaretAnchored {
    fn new(
        input: WeakEntity<InputState>,
        build: impl Fn(&mut Window, &mut App) -> AnyElement + 'static,
    ) -> Self {
        Self {
            input,
            build: Box::new(build),
            content: None,
        }
    }
}

impl Element for CaretAnchored {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        (window.request_layout(Style::default(), [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(input) = self.input.upgrade() else {
            return;
        };
        let Some(caret) = input.read(cx).prepaint_caret_geometry() else {
            return;
        };
        let bounds = caret.cursor_bounds();
        let viewport = window.viewport_size();
        let mut content = (self.build)(window, cx);
        let size = content.layout_as_root(
            Size {
                width: AvailableSpace::Definite(viewport.width),
                height: AvailableSpace::Definite(viewport.height),
            },
            window,
            cx,
        );
        // Left edge under the caret, one line below it, kept inside the window.
        let margin = px(8.);
        let x = bounds
            .origin
            .x
            .min(viewport.width - size.width - margin)
            .max(margin);
        let y = bounds.origin.y + caret.scroll_offset().y + caret.line_height() + px(4.);
        content.prepaint_at(point(x, y), window, cx);
        self.content = Some(content);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(content) = self.content.as_mut() {
            content.paint(window, cx);
        }
    }
}

impl IntoElement for CaretAnchored {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// The span an accepted candidate replaces: the variable token the cursor is
/// in, or nothing at the cursor if the text no longer has one.
fn replacement_range(text: &Rope, cursor: usize) -> Range<usize> {
    token_at_cursor(text, cursor)
        .map(|token| token.start..token.end)
        .unwrap_or(cursor..cursor)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gpui_kit::component::input::Rope;

    use super::{UrlCompletion, replacement_range};
    use crate::models::EnvironmentVariable;
    use crate::ui::request::completion::VariableCandidate;

    fn apply(text: &str, cursor: usize, name: &str) -> String {
        let candidate = VariableCandidate {
            name: name.into(),
            detail: "Environment variable",
        };
        let mut result = text.to_string();
        result.replace_range(
            replacement_range(&Rope::from(text), cursor),
            &candidate.insertion(),
        );
        result
    }

    #[test]
    fn replaces_the_typed_prefix_and_an_existing_close() {
        assert_eq!(apply("{{ho", 4, "host"), "{{host}}");
        assert_eq!(apply("{{ho}}", 4, "host"), "{{host}}");
        assert_eq!(
            apply("https://{{ho}}/v1/users", 12, "host"),
            "https://{{host}}/v1/users"
        );
    }

    #[test]
    fn typing_more_of_a_token_filters_the_loaded_variables() {
        let variables = Arc::new(vec![
            EnvironmentVariable {
                name: "host".into(),
                value: "example.com".into(),
                enabled: true,
                description: None,
            },
            EnvironmentVariable {
                name: "port".into(),
                value: "8080".into(),
                enabled: true,
                description: None,
            },
        ]);
        let mut completion = UrlCompletion::new(8, variables);
        let names = |completion: &UrlCompletion| -> Vec<String> {
            completion
                .candidates
                .iter()
                .map(|candidate| candidate.name.clone())
                .collect()
        };

        completion.refilter(10, "ho");
        assert_eq!(names(&completion), ["host"]);
        completion.refilter(11, "hox");
        assert!(names(&completion).is_empty());
        // Backspacing restores matches without anything being reloaded.
        completion.refilter(10, "ho");
        assert_eq!(names(&completion), ["host"]);
        assert_eq!(completion.cursor, 10);
    }

    #[test]
    fn inserts_at_the_cursor_when_the_token_is_gone() {
        assert_eq!(apply("https://api", 11, "host"), "https://api{{host}}");
    }
}
