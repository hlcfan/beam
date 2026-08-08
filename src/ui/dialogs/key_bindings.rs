use super::super::*;

pub(in crate::ui) struct KeyBindingsDialogView;

impl KeyBindingsDialogView {
    fn key_bindings_list() -> Vec<(&'static str, &'static str)> {
        if cfg!(target_os = "macos") {
            vec![
                ("Send Request", "cmd-enter"),
                ("Send Request", "cmd-r"),
                ("New Request", "cmd-n"),
                ("Duplicate Selected Item", "cmd-d"),
                ("Rename Selected Item", "f2"),
                ("Delete Selected Item", "cmd-backspace"),
                ("Focus URL", "cmd-l"),
                ("Open Settings", "cmd-,"),
                ("Open Command Palette", "cmd-p"),
                ("Next Item in Tree", "cmd-alt-down"),
                ("Previous Item in Tree", "cmd-alt-up"),
                ("Next Item in Tree", "ctrl-j"),
                ("Previous Item in Tree", "ctrl-k"),
                ("Expand/Collapse Selected Folder", "space"),
                ("Next Request in History", "cmd-alt-right"),
                ("Previous Request in History", "cmd-alt-left"),
                ("Quit Beam", "cmd-q"),
            ]
        } else {
            vec![
                ("Send Request", "ctrl-enter"),
                ("Send Request", "ctrl-r"),
                ("New Request", "ctrl-n"),
                ("Duplicate Selected Item", "ctrl-d"),
                ("Rename Selected Item", "f2"),
                ("Delete Selected Item", "ctrl-delete"),
                ("Focus URL", "ctrl-l"),
                ("Open Settings", "ctrl-,"),
                ("Open Command Palette", "ctrl-p"),
                ("Next Item in Tree", "ctrl-j"),
                ("Previous Item in Tree", "ctrl-k"),
                ("Next Item in Tree", "ctrl-alt-down"),
                ("Previous Item in Tree", "ctrl-alt-up"),
                ("Expand/Collapse Selected Folder", "space"),
                ("Next Request in History", "ctrl-alt-right"),
                ("Previous Request in History", "ctrl-alt-left"),
                ("Quit Beam", "alt-f4"),
            ]
        }
    }
}

fn key_binding_display_tokens(binding: &str) -> Vec<String> {
    let is_mac = cfg!(target_os = "macos");
    binding
        .split('-')
        .map(|part| match part {
            "cmd" => if is_mac { "⌘" } else { "Cmd" }.to_string(),
            "ctrl" => "Ctrl".to_string(),
            "alt" => if is_mac { "⌥" } else { "Alt" }.to_string(),
            "shift" => if is_mac { "⇧" } else { "Shift" }.to_string(),
            "enter" => "Enter".to_string(),
            "down" => "↓".to_string(),
            "up" => "↑".to_string(),
            "left" => "←".to_string(),
            "right" => "→".to_string(),
            "backspace" => "Delete".to_string(),
            "delete" => "Delete".to_string(),
            "," => ",".to_string(),
            s => s.to_uppercase(),
        })
        .collect()
}

fn render_key_binding_chip(token: &str, cx: &App) -> Div {
    h_flex()
        .items_center()
        .justify_center()
        .px_1p5()
        .h(px(22.0))
        .min_w(px(22.0))
        .rounded(px(6.0))
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().secondary)
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(token.to_string())
}

impl Render for KeyBindingsDialogView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let bindings = Self::key_bindings_list();
        let mut list = v_flex().w_full().flex_none().gap_2();
        for (label, binding) in bindings {
            let tokens = key_binding_display_tokens(binding);
            let mut chips = h_flex().items_center().gap_1();
            for token in tokens {
                chips = chips.child(render_key_binding_chip(token.as_str(), cx));
            }
            list = list.child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .w_full()
                    .py_1()
                    .text_sm()
                    .child(
                        div()
                            .text_color(cx.theme().foreground)
                            .child(label.to_string()),
                    )
                    .child(chips),
            );
        }
        div()
            .w_full()
            .h(px(420.0))
            .overflow_y_scrollbar()
            .child(v_flex().w_full().p_3().child(list))
            .into_any_element()
    }
}
