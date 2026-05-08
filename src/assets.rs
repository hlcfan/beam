use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "assets"]
#[include = "icons/**/*.svg"]
struct BeamAssets;

pub struct Assets;

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
