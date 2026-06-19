use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use gpui::{AssetSource, Result, SharedString};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "assets"]
struct BeamAssets;

#[derive(RustEmbed)]
#[folder = "themes"]
#[include = "*.json"]
struct BeamThemes;

pub struct Assets;

/// Directory on disk where icons from `BeamAssets::icons/` are extracted.
///
/// The native context-menu backend on macOS/Windows requires real filesystem
/// paths (it loads `NSImage` / `HBITMAP` via `initWithContentsOfFile`), so the
/// SVG bytes embedded by `rust-embed` are unpacked here on first use and the
/// returned paths are reused for subsequent `menu_with_image` calls.
static ICONS_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Get the absolute filesystem path for an embedded icon (`icons/<name>.svg`).
///
/// The icon is extracted from the embedded `assets/` folder to a cache
/// directory the first time this is called; later calls reuse the same path.
/// Returns `None` if the icon cannot be located or written.
pub fn native_icon_path(icon: &str) -> Option<PathBuf> {
    let dir = icons_dir()?;
    let file_name = Path::new(icon).file_name()?;
    let target = dir.join(file_name);
    if target.exists() {
        return Some(target);
    }

    let file = BeamAssets::get(icon)?;
    if let Err(error) = std::fs::write(&target, file.data.as_ref()) {
        log::warn!("failed to write native menu icon {icon}: {error}");
        return None;
    }
    Some(target)
}

fn icons_dir() -> Option<&'static PathBuf> {
    ICONS_DIR.get_or_init(|| {
        let base = dirs::data_local_dir()
            .or_else(dirs::data_dir)
            .unwrap_or_else(std::env::temp_dir);
        let dir = base.join("beam").join("icons");
        if let Err(error) = std::fs::create_dir_all(&dir) {
            log::warn!(
                "failed to create native menu icon cache {}: {error}",
                dir.display()
            );
        }
        dir
    });
    ICONS_DIR.get()
}

pub(crate) fn embedded_theme_contents() -> Vec<(String, String)> {
    let mut themes: Vec<(String, String)> = BeamThemes::iter()
        .filter_map(|path| {
            let file = BeamThemes::get(path.as_ref())?;
            let content = std::str::from_utf8(file.data.as_ref()).ok()?.to_owned();
            Some((path.into_owned(), content))
        })
        .collect();

    themes.sort_by(|a, b| a.0.cmp(&b.0));
    themes
}

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path.is_empty() {
            return Ok(None);
        }

        if let Some(file) = BeamAssets::get(path) {
            return Ok(Some(file.data));
        }

        gpui_component_assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = gpui_component_assets::Assets.list(path)?;
        for local_path in BeamAssets::iter().filter(|candidate| candidate.starts_with(path)) {
            let local_path: SharedString = local_path.into();
            if !paths.contains(&local_path) {
                paths.push(local_path);
            }
        }
        Ok(paths)
    }
}
