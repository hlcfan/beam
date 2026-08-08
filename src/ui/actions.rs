use gpui::*;
use gpui_component::{ActiveTheme, ThemeMode, ThemeRegistry};
use ulid::Ulid;

actions!(
    beam,
    [
        QuitApp,
        SendActiveRequest,
        CreateRequestBelowActive,
        FocusUrlInput,
        OpenSettings,
        OpenCommandPalette,
        SelectNextRequestInTree,
        SelectPrevRequestInTree,
        ToggleSelectedFolder,
        SelectNextRequestInViewHistory,
        SelectPrevRequestInViewHistory,
        FormatRequestBody,
        FormatResponseBody,
        DuplicateActiveRequest,
        RenameActiveRequest,
        DeleteSelectedTreeNode
    ]
);

#[derive(Action, Clone, PartialEq)]
#[action(namespace = beam, no_json)]
pub(super) struct SwitchTheme(pub SharedString);

#[derive(Action, Clone, PartialEq)]
#[action(namespace = beam, no_json)]
pub(super) struct SwitchThemeMode(pub ThemeMode);

actions!(beam, [TreeMenuAddRequestAtRoot, TreeMenuAddFolderAtRoot]);

#[derive(Action, Clone, PartialEq)]
#[action(namespace = beam, no_json)]
pub(super) struct TreeMenuSendRequest(pub Ulid);

#[derive(Action, Clone, PartialEq)]
#[action(namespace = beam, no_json)]
pub(super) struct TreeMenuCopyAsCurl(pub Ulid);

#[derive(Action, Clone, PartialEq)]
#[action(namespace = beam, no_json)]
pub(super) struct TreeMenuAddRequestInFolder(pub Ulid);

#[derive(Action, Clone, PartialEq)]
#[action(namespace = beam, no_json)]
pub(super) struct TreeMenuAddFolderInFolder(pub Ulid);

#[derive(Action, Clone, PartialEq)]
#[action(namespace = beam, no_json)]
pub(super) struct TreeMenuRename(pub Ulid);

#[derive(Action, Clone, PartialEq)]
#[action(namespace = beam, no_json)]
pub(super) struct TreeMenuDelete(pub Ulid);

#[derive(Action, Clone, PartialEq)]
#[action(namespace = beam, no_json)]
pub(super) struct TreeMenuDuplicateRequest(pub Ulid);

#[derive(Action, Clone, PartialEq)]
#[action(namespace = beam, no_json)]
pub(super) struct TreeMenuDuplicateFolder(pub Ulid);

#[cfg(target_os = "macos")]
fn build_macos_theme_menu(cx: &App) -> MenuItem {
    let themes = ThemeRegistry::global(cx).sorted_themes();
    let active_theme_name = cx.theme().theme_name().clone();
    MenuItem::Submenu(Menu {
        name: "Theme".into(),
        items: themes
            .iter()
            .map(|theme| {
                MenuItem::action(theme.name.clone(), SwitchTheme(theme.name.clone()))
                    .checked(theme.name == active_theme_name)
            })
            .collect(),
        disabled: false,
    })
}

#[cfg(target_os = "macos")]
pub(super) fn build_macos_system_menus(cx: &App) -> Vec<Menu> {
    vec![
        Menu {
            name: "Beam".into(),
            items: vec![
                MenuItem::action("Settings", OpenSettings),
                MenuItem::separator(),
                MenuItem::Submenu(Menu {
                    name: "Appearance".into(),
                    items: vec![
                        MenuItem::action("Light", SwitchThemeMode(ThemeMode::Light))
                            .checked(!cx.theme().mode.is_dark()),
                        MenuItem::action("Dark", SwitchThemeMode(ThemeMode::Dark))
                            .checked(cx.theme().mode.is_dark()),
                    ],
                    disabled: false,
                }),
                build_macos_theme_menu(cx),
                MenuItem::separator(),
                MenuItem::action("Quit Beam", QuitApp),
            ],
            disabled: false,
        },
        Menu {
            name: "File".into(),
            items: vec![
                MenuItem::action("New Request", CreateRequestBelowActive),
                MenuItem::separator(),
                MenuItem::action("Duplicate Request", DuplicateActiveRequest),
                MenuItem::action("Rename Request", RenameActiveRequest),
                MenuItem::separator(),
                MenuItem::action("Focus URL", FocusUrlInput),
            ],
            disabled: false,
        },
        Menu {
            name: "Edit".into(),
            items: vec![
                MenuItem::action("Undo", gpui_component::input::Undo),
                MenuItem::action("Redo", gpui_component::input::Redo),
                MenuItem::separator(),
                MenuItem::action("Cut", gpui_component::input::Cut),
                MenuItem::action("Copy", gpui_component::input::Copy),
                MenuItem::action("Paste", gpui_component::input::Paste),
                MenuItem::separator(),
                MenuItem::action("Select All", gpui_component::input::SelectAll),
            ],
            disabled: false,
        },
        Menu {
            name: "View".into(),
            items: vec![MenuItem::action("Focus URL", FocusUrlInput)],
            disabled: false,
        },
    ]
}
