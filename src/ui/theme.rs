use gpui::*;
use gpui_component::{ActiveTheme, Theme, ThemeMode, ThemeRegistry, WindowExt as _};

use super::BeamView;
#[cfg(target_os = "macos")]
use super::actions::build_macos_system_menus;
use crate::assets::embedded_theme_contents;
use crate::models::AppFontSize;
use crate::paths::BeamPaths;
use crate::storage::fs_backend::FileSystemStorage;
use crate::storage::workspace_repo::WorkspaceRepository;

#[cfg(not(target_family = "wasm"))]
pub(super) fn init_theme_registry(
    preferred_theme_name: Option<SharedString>,
    preferred_font_size: AppFontSize,
    cx: &mut App,
) {
    let registry = ThemeRegistry::global_mut(cx);
    for (theme_path, content) in embedded_theme_contents() {
        if let Err(error) = registry.load_themes_from_str(&content) {
            log::error!("Failed to preload theme file {theme_path}: {error}");
        }
    }

    if let Some(theme_name) = preferred_theme_name.as_ref() {
        let _ = BeamView::apply_named_theme_by_name(theme_name.as_ref(), cx, false);
    }
    BeamView::apply_font_size(preferred_font_size, cx);
}

impl BeamView {
    pub(super) fn apply_theme_mode(mode: ThemeMode, cx: &mut App) {
        let active_font_size = AppFontSize::from_pixels_value(cx.theme().font_size.as_f32());
        Theme::change(mode, None, cx);
        Self::apply_font_size(active_font_size, cx);
        #[cfg(target_os = "macos")]
        cx.set_menus(build_macos_system_menus(cx));
        if let Err(error) = Self::persist_theme_state_from_app(cx) {
            log::error!("{error}");
        }
    }

    pub(super) fn apply_named_theme(theme_name: SharedString, cx: &mut App) {
        if Self::apply_named_theme_by_name(theme_name.as_ref(), cx, true) {
            return;
        }
    }

    fn apply_font_size(font_size: AppFontSize, cx: &mut App) {
        let theme = Theme::global_mut(cx);
        theme.font_size = px(font_size.pixels());
        theme.mono_font_size = px(font_size.mono_pixels());
        cx.refresh_windows();
    }

    pub(super) fn apply_font_size_setting(
        &mut self,
        font_size: AppFontSize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        Self::apply_font_size(font_size, cx);
        self.shell.theme.font_size = font_size;
        if let Err(error) = self.persist_font_size_state(font_size) {
            window.push_notification(error, cx);
        }
        cx.notify();
    }

    pub(super) fn apply_auto_format_response_setting(
        &mut self,
        auto_format_response: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.shell.theme.auto_format_response = auto_format_response;
        if let Err(error) = self.persist_auto_format_response_state(auto_format_response) {
            window.push_notification(error, cx);
        }
        cx.notify();
    }

    pub(super) fn apply_wrap_body_editor_setting(
        &mut self,
        wrap_body_editor: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.shell.theme.wrap_body_editor = wrap_body_editor;
        self.request_body_editor.update(cx, |input, cx| {
            input.set_soft_wrap(wrap_body_editor, window, cx);
        });
        self.response_body_editor.update(cx, |input, cx| {
            input.set_soft_wrap(wrap_body_editor, window, cx);
        });
        if let Err(error) = self.persist_wrap_body_editor_state(wrap_body_editor) {
            window.push_notification(error, cx);
        }
        cx.notify();
    }

    fn apply_named_theme_by_name(theme_name: &str, cx: &mut App, persist: bool) -> bool {
        let stored_theme_name: SharedString = theme_name.to_string().into();
        let theme_config = ThemeRegistry::global(cx)
            .themes()
            .get(&stored_theme_name)
            .cloned();
        if let Some(theme_config) = theme_config {
            Theme::global_mut(cx).apply_config(&theme_config);
            #[cfg(target_os = "macos")]
            cx.set_menus(build_macos_system_menus(cx));
            if persist {
                if let Err(error) = Self::persist_theme_state_from_app(cx) {
                    log::error!("{error}");
                }
            }
            return true;
        }
        false
    }

    fn persist_font_size_state(&self, font_size: AppFontSize) -> Result<(), String> {
        let backend = FileSystemStorage::new(self.current_workspace_paths.clone());
        let storage = WorkspaceRepository::new(backend)
            .map_err(|error| format!("Failed to load workspace: {error}"))?;
        storage
            .persist_font_size_state(font_size)
            .map_err(|error| format!("Failed to save local state: {error}"))
    }

    fn persist_auto_format_response_state(&self, auto_format_response: bool) -> Result<(), String> {
        let backend = FileSystemStorage::new(self.current_workspace_paths.clone());
        let storage = WorkspaceRepository::new(backend)
            .map_err(|error| format!("Failed to load workspace: {error}"))?;
        storage
            .persist_auto_format_response_state(auto_format_response)
            .map_err(|error| format!("Failed to save local state: {error}"))
    }

    fn persist_wrap_body_editor_state(&self, wrap_body_editor: bool) -> Result<(), String> {
        let backend = FileSystemStorage::new(self.current_workspace_paths.clone());
        let storage = WorkspaceRepository::new(backend)
            .map_err(|error| format!("Failed to load workspace: {error}"))?;
        storage
            .persist_wrap_body_editor_state(wrap_body_editor)
            .map_err(|error| format!("Failed to save local state: {error}"))
    }

    fn persist_theme_state_from_app(cx: &App) -> Result<(), String> {
        let paths = BeamPaths::default_user_config();
        let backend = FileSystemStorage::new(paths);
        let storage = WorkspaceRepository::new(backend)
            .map_err(|error| format!("Failed to load workspace: {error}"))?;
        let active_theme_name = cx.theme().theme_name().to_string();
        storage
            .persist_theme_state(active_theme_name.as_str())
            .map_err(|error| format!("Failed to save local state: {error}"))
    }
}
