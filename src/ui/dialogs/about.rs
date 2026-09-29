use std::time::Duration;

use gpui_kit::component::{ActiveTheme, WindowExt, v_flex};
use gpui_kit::*;

// These fixed colors are pixel-art content; the dialog surface uses theme tokens.
const RAINBOW: [u32; 6] = [0xff667f, 0xffb35c, 0xffe477, 0x8bdd8d, 0x7eb6ff, 0xb99bff];

const CAT_HEAD: [&str; 10] = [
    ".##.....##.",
    "#gg#...#gg#",
    "#ggg###ggg#",
    "#ggggggggg#",
    "#gw#gggw#g#",
    "#g##ggg##g#",
    "#pggg#gggp#",
    "#gg#ggg#gg#",
    ".#gg###gg#.",
    "..#######..",
];

const OUTLINE: u32 = 0x292934;
const FUR: u32 = 0xa4a4b2;
const CRUST: u32 = 0xf5d49a;
const ICING: u32 = 0xffa3d5;
const SPRINKLE: u32 = 0xe963a8;
const STAR: u32 = 0x9b9bab;

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
        // GPUI keeps repeating animations static under reduced motion and stops
        // requesting frames once this element leaves the dialog tree.
        .with_animation(
            "about-nyan-cat",
            Animation::new(Duration::from_millis(600))
                .repeat()
                .with_max_fps(20.0),
            |scene, phase| {
                let frame = (phase * 6.0) as usize;
                scene.child(
                    canvas(
                        |_, _, _| {},
                        move |bounds, _, window, _| paint_nyan_cat(bounds, frame % 6, window),
                    )
                    .size_full(),
                )
            },
        )
}

fn paint_nyan_cat(bounds: Bounds<Pixels>, frame: usize, window: &mut Window) {
    let pixel = (bounds.size.width.as_f32() / 96.0).min(bounds.size.height.as_f32() / 32.0);
    if pixel < 1.0 {
        return;
    }
    let scene_width = (bounds.size.width.as_f32() / pixel).ceil() as i32;
    let cat_offset = (scene_width - 96).max(0);
    let origin = point(
        bounds.origin.x,
        bounds.origin.y + px((bounds.size.height.as_f32() - 32.0 * pixel) / 2.0),
    );
    // Snap each shared pixel boundary to the display grid so scaling the art to
    // the dialog width does not leave seams between adjacent rectangles.
    let scale = window.scale_factor();
    let snap = |value: Pixels| px((value.as_f32() * scale).round() / scale);
    let mut draw = |x, y, width, height, color| {
        let top_left = point(
            snap(origin.x + px(x as f32 * pixel)),
            snap(origin.y + px(y as f32 * pixel)),
        );
        let bottom_right = point(
            snap(origin.x + px((x + width) as f32 * pixel)),
            snap(origin.y + px((y + height) as f32 * pixel)),
        );
        window.paint_quad(fill(
            Bounds::from_corners(top_left, bottom_right),
            rgb(color),
        ));
    };

    for (lane, y) in [(0, 5), (11, 26)] {
        for column in -1..=(scene_width / 24 + 1) {
            let x = column * 24 + lane - frame as i32 * 4;
            draw(x, y, 1, 1, STAR);
            for (dx, dy) in [(-2, 0), (2, 0), (0, -2), (0, 2)] {
                draw(x + dx, y + dy, 1, 1, STAR);
            }
        }
    }

    let rainbow_end = 53 + cat_offset;
    for x in (0..rainbow_end).step_by(4) {
        let wave = (x as usize / 4 + frame / 2) % 2;
        for (stripe, color) in RAINBOW.into_iter().enumerate() {
            draw(
                x,
                (10 + stripe * 2 + wave) as i32,
                (rainbow_end - x).min(4),
                2,
                color,
            );
        }
    }

    let bob = [0, 0, 1, 1, 1, 0][frame];
    let paw = [0, 1, 2, 2, 1, 0][frame];
    draw(47 + cat_offset, 15 + bob - paw / 2, 8, 3, OUTLINE);
    draw(45 + cat_offset, 13 + bob - paw / 2, 3, 3, OUTLINE);
    draw(46 + cat_offset, 14 + bob - paw / 2, 2, 2, FUR);
    draw(48 + cat_offset, 16 + bob - paw / 2, 6, 1, FUR);
    for (x, offset) in [(55, paw), (60, 2 - paw), (68, paw), (73, 2 - paw)] {
        draw(x + cat_offset + offset, 23 + bob, 4, 3, OUTLINE);
        draw(x + cat_offset + offset + 1, 23 + bob, 2, 2, FUR);
    }

    draw(53 + cat_offset, 7 + bob, 22, 17, OUTLINE);
    draw(52 + cat_offset, 8 + bob, 24, 15, OUTLINE);
    draw(54 + cat_offset, 8 + bob, 20, 15, CRUST);
    draw(53 + cat_offset, 9 + bob, 22, 13, CRUST);
    draw(55 + cat_offset, 9 + bob, 18, 13, ICING);
    draw(54 + cat_offset, 10 + bob, 20, 11, ICING);
    for (x, y) in [
        (57, 11),
        (63, 10),
        (69, 12),
        (59, 15),
        (66, 14),
        (56, 19),
        (62, 20),
        (69, 18),
    ] {
        draw(x + cat_offset, y + bob, 1, 1, SPRINKLE);
    }

    for (y, row) in CAT_HEAD.into_iter().enumerate() {
        for (x, color) in row.bytes().enumerate() {
            let color = match color {
                b'#' => OUTLINE,
                b'g' => FUR,
                b'w' => 0xffffff,
                b'p' => ICING,
                _ => continue,
            };
            draw(70 + x as i32 + cat_offset, 13 + y as i32 + bob, 1, 1, color);
        }
    }
}
