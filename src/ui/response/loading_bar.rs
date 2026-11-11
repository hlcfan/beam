use super::super::*;

/// Shimmer bar shown over the response while a request is sending.
///
/// gpui notifies the view being drawn on every animation frame. Keeping the bar in a view of its
/// own means those frames render this view and not `BeamView`.
pub(in crate::ui) struct LoadingBar {
    /// Counts renders so tests can check that the animation keeps advancing.
    #[cfg(test)]
    pub(in crate::ui) render_count: usize,
}

impl LoadingBar {
    pub(in crate::ui) fn new() -> Self {
        Self {
            #[cfg(test)]
            render_count: 0,
        }
    }
}

impl Render for LoadingBar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(test)]
        {
            self.render_count += 1;
        }
        let color = cx.theme().progress_bar;
        div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .h(px(2.0))
            .overflow_hidden()
            .bg(color.opacity(0.18))
            .with_animation(
                "shimmer-loading",
                Animation::new(Duration::from_millis(2800)).repeat(),
                move |this, delta| {
                    this.child(
                        canvas(
                            |_, _, _| {},
                            move |bounds, _, window, _| {
                                let width = bounds.size.width;
                                let height = bounds.size.height;
                                let origin = bounds.origin;

                                // Ease back and forth without restarting at the
                                // left edge. Each half fades toward its thin tip.
                                let travel = (1.0 - (delta * std::f32::consts::TAU).cos()) / 2.0;
                                let half_length = width * 0.14;
                                let center_x = half_length + (width - half_length * 2.0) * travel;
                                let center_y = height * 0.5;
                                for direction in [-1.0, 1.0] {
                                    let tip_x = center_x + half_length * direction;
                                    let control_x = center_x + half_length * direction * 0.45;
                                    let mut highlight = PathBuilder::fill();
                                    highlight.move_to(origin + point(tip_x, center_y));
                                    highlight.curve_to(
                                        origin + point(center_x, px(0.0)),
                                        origin + point(control_x, px(0.0)),
                                    );
                                    highlight.line_to(origin + point(center_x, height));
                                    highlight.curve_to(
                                        origin + point(tip_x, center_y),
                                        origin + point(control_x, height),
                                    );
                                    highlight.close();
                                    let dim = color.opacity(0.0);
                                    let bright = color.opacity(0.95);
                                    let (from, to) = if direction < 0.0 {
                                        (dim, bright)
                                    } else {
                                        (bright, dim)
                                    };
                                    if let Ok(highlight) = highlight.build() {
                                        window.paint_path(
                                            highlight,
                                            linear_gradient(
                                                90.0,
                                                linear_color_stop(from, 0.0),
                                                linear_color_stop(to, 1.0),
                                            ),
                                        );
                                    }
                                }
                            },
                        )
                        .size_full(),
                    )
                },
            )
    }
}
