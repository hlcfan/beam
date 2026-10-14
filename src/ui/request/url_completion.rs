//! Variable completion for the URL input, composed around a plain `Input`.
//!
//! The input knows nothing about completion. This module observes its edits,
//! owns the candidates and the selection, captures the navigation keys before
//! the input handles them, renders the popup, and applies the chosen edit with
//! the input's own `set_selected_range` and `replace`.

use std::ops::Range;

use gpui_kit::Focusable;
use gpui_kit::component::input::{Escape, MoveDown, MoveUp, Rope};

use super::super::*;
use super::completion::{
    VariableCandidate, load_environment_variables, token_at_cursor, variable_candidates,
};

const VISIBLE_ROWS: usize = 8;

/// The candidates offered for the token under the cursor.
pub(in crate::ui) struct UrlCompletion {
    /// Cursor offset the candidates were computed for. The popup is shown only
    /// while the cursor is still there, so moving it away hides a stale list.
    cursor: usize,
    candidates: Vec<VariableCandidate>,
    selected: usize,
    first_visible: usize,
}

impl BeamView {
    /// Recomputes the candidates after the URL text or cursor changed.
    pub(in crate::ui) fn update_url_completion(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.url_completion_task = None;
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
        let path = self
            .selected_environment_id_for_view()
            .and_then(|id| self.environment_file_path_from_shell(id));
        let prefix = token.prefix;
        let candidates = cx.background_spawn(async move {
            let variables = load_environment_variables(path);
            variable_candidates(&prefix, &variables)
        });
        self.url_completion_task = Some(cx.spawn_in(window, async move |this, cx| {
            let candidates = candidates.await;
            let _ = this.update_in(cx, |this, window, cx| {
                let input = this.url_input.read(cx);
                if input.cursor() != cursor || !input.focus_handle(cx).is_focused(window) {
                    return;
                }
                this.url_completion = (!candidates.is_empty()).then_some(UrlCompletion {
                    cursor,
                    candidates,
                    selected: 0,
                    first_visible: 0,
                });
                cx.notify();
            });
        }));
    }

    pub(in crate::ui) fn dismiss_url_completion(&mut self, cx: &mut Context<Self>) {
        self.url_completion_task = None;
        if self.url_completion.take().is_some() {
            cx.notify();
        }
    }

    /// Whether the popup is showing, and so owns Up, Down, Escape and Enter.
    pub(in crate::ui) fn url_completion_active(&self, cx: &App) -> bool {
        self.url_completion
            .as_ref()
            .is_some_and(|completion| completion.cursor == self.url_input.read(cx).cursor())
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
        completion.selected = (completion.selected as isize + step).rem_euclid(count) as usize;
        if completion.selected < completion.first_visible {
            completion.first_visible = completion.selected;
        } else if completion.selected >= completion.first_visible + VISIBLE_ROWS {
            completion.first_visible = completion.selected + 1 - VISIBLE_ROWS;
        }
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

    /// The popup, anchored below the URL field, when there is something to show.
    pub(in crate::ui) fn render_url_completion(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.url_completion_active(cx) {
            return None;
        }
        let completion = self.url_completion.as_ref()?;
        let rows = completion
            .candidates
            .iter()
            .enumerate()
            .skip(completion.first_visible)
            .take(VISIBLE_ROWS)
            .map(|(index, candidate)| {
                let selected = index == completion.selected;
                h_flex()
                    .id(("url-completion-row", index))
                    .w_full()
                    .px_2()
                    .py_1()
                    .gap_3()
                    .justify_between()
                    .text_xs()
                    .rounded(cx.theme().radius)
                    .cursor_pointer()
                    .when(selected, |row| {
                        row.bg(cx.theme().accent)
                            .text_color(cx.theme().accent_foreground)
                    })
                    .when(!selected, |row| {
                        row.hover(|row| row.bg(cx.theme().accent.opacity(0.5)))
                    })
                    .child(candidate.name.clone())
                    .child(
                        div()
                            .text_color(cx.theme().muted_foreground)
                            .child(candidate.detail),
                    )
                    // Mouse down, not click: it lands before the input could lose focus.
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            this.accept_url_completion(Some(index), window, cx);
                            cx.stop_propagation();
                        }),
                    )
            });

        Some(
            deferred(
                div().absolute().top(relative(1.)).left_0().pt_1().child(
                    v_flex()
                        .occlude()
                        .popover_style(cx)
                        .min_w(px(240.))
                        .p_1()
                        .children(rows),
                ),
            )
            .with_priority(2)
            .into_any_element(),
        )
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
    use gpui_kit::component::input::Rope;

    use super::replacement_range;
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
    fn inserts_at_the_cursor_when_the_token_is_gone() {
        assert_eq!(apply("https://api", 11, "host"), "https://api{{host}}");
    }
}
