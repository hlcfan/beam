use gpui_kit::component::{ActiveTheme, WindowExt, v_flex};
use gpui_kit::*;

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
                    ),
            )
    });
}
