use std::borrow::Cow;

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
