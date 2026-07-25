use gpui::{Action, Hsla};
use gpui_component::{Icon, input, native_menu::NativeMenu};

/// Append a context-menu item to a [`NativeMenu`], preferring an icon variant.
///
/// `icon_path` is the asset-source relative path (e.g. `"icons/cut.svg"`).
pub(super) fn append_with_image_or_plain(
    menu: NativeMenu,
    label: &str,
    icon_path: &str,
    disabled: bool,
    action: Box<dyn Action>,
) -> NativeMenu {
    menu.menu_with_icon_disabled(label, Icon::default().path(icon_path), disabled, action)
}

pub(super) fn build_text_edit_context_menu(
    menu: NativeMenu,
    has_selection: bool,
    _muted_color: Hsla,
) -> NativeMenu {
    let menu = append_with_image_or_plain(
        menu,
        "Cut",
        "icons/cut.svg",
        !has_selection,
        Box::new(input::Cut),
    );
    let menu = append_with_image_or_plain(
        menu,
        "Copy",
        "icons/copy.svg",
        !has_selection,
        Box::new(input::Copy),
    );
    let menu = append_with_image_or_plain(
        menu,
        "Paste",
        "icons/clipboard-paste.svg",
        false,
        Box::new(input::Paste),
    );
    let menu = menu.separator();
    append_with_image_or_plain(
        menu,
        "Select All",
        "icons/square-dashed-text.svg",
        false,
        Box::new(input::SelectAll),
    )
}

pub(super) fn build_text_edit_context_menu_with_find(
    menu: NativeMenu,
    has_selection: bool,
    muted_color: Hsla,
) -> NativeMenu {
    let menu = append_with_image_or_plain(
        menu,
        "Find",
        "icons/search.svg",
        false,
        Box::new(input::Search),
    )
    .separator();
    build_text_edit_context_menu(menu, has_selection, muted_color)
}
