use std::time::Duration;

use gpui_kit::component::{ActiveTheme, WindowExt, v_flex};
use gpui_kit::*;

const NYAN_CAT_FRAMES: [&str; 6] = [
    "about/nyan-cat-0.svg",
    "about/nyan-cat-1.svg",
    "about/nyan-cat-2.svg",
    "about/nyan-cat-3.svg",
    "about/nyan-cat-4.svg",
    "about/nyan-cat-5.svg",
];

pub(in crate::ui) fn open_about_dialog(window: &mut Window, cx: &mut App) {
    window.open_dialog(cx, |dialog, _, cx| {
        dialog
            .title("About")
            .w_80()
            .keyboard(true)
            .overlay_closable(true)
            .child(
                v_flex()
                    .items_center()
                    .gap_2()
                    .py_4()
                    .child(img("icon.iconset/icon_128x128@2x.png").size_16())
                    .child(
                        div()
                            .text_xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Beam"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(concat!("Version ", env!("CARGO_PKG_VERSION"))),
                    )
                    .child(render_nyan_cat(cx)),
            )
    });
}

fn render_nyan_cat(cx: &App) -> impl IntoElement {
    div()
        .w_full()
        .h_24()
        .mt_2()
        .overflow_hidden()
        .rounded(cx.theme().radius)
        .bg(cx.theme().muted)
        // The SVGs contain the pixel artwork's palette; the surface uses the theme.
        // GPUI keeps repeating animations static under reduced motion and stops
        // requesting frames once this element leaves the dialog tree.
        .with_animation(
            "about-nyan-cat",
            Animation::new(Duration::from_millis(600))
                .repeat()
                .with_max_fps(10.0),
            |scene, phase| {
                let frame = (phase * NYAN_CAT_FRAMES.len() as f32) as usize;
                scene.child(img(NYAN_CAT_FRAMES[frame % NYAN_CAT_FRAMES.len()]).size_full())
            },
        )
}
