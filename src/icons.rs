use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "assets/icons/"]
pub struct Assets;

impl Assets {
    pub fn get_svg_handle(path: &str) -> Option<iced::widget::svg::Handle> {
        Self::get(path).map(|data| iced::widget::svg::Handle::from_memory(data.data))
    }
}
