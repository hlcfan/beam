use crate::icons;
use iced::widget::svg;
use iced::{Element, Length, Color};

/// Available icon names that can be used with the icon component
#[derive(Debug, Clone, Copy)]
pub enum IconName {
    Cancel,
    Send,
    ChevronDown,
    ChevronRight,
}

impl IconName {
    /// Get the filename for the icon
    fn filename(&self) -> &'static str {
        match self {
            IconName::Cancel => "cancel.svg",
            IconName::Send => "send.svg",
            IconName::ChevronDown => "chevron-down.svg",
            IconName::ChevronRight => "chevron-right.svg",
        }
    }
}

/// Icon component for rendering SVG icons with consistent styling
pub fn icon<'a, Message>(name: IconName) -> Icon<'a, Message> {
    Icon::new(name)
}

/// Icon builder struct for configuring SVG icons
pub struct Icon<'a, Message> {
    name: IconName,
    width: Length,
    height: Length,
    color: Option<Color>,
    _phantom: std::marker::PhantomData<&'a Message>,
}

impl<'a, Message> Icon<'a, Message> {
    /// Create a new icon with the specified name
    pub fn new(name: IconName) -> Self {
        Self {
            name,
            width: Length::Fixed(16.0),
            height: Length::Fixed(16.0),
            color: None,
            _phantom: std::marker::PhantomData,
        }
    }

    /// Set the width of the icon
    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }

    /// Set the height of the icon
    pub fn height(mut self, height: impl Into<Length>) -> Self {
        self.height = height.into();
        self
    }

    /// Set both width and height to the same value (square icon)
    pub fn size(mut self, size: impl Into<Length>) -> Self {
        let size = size.into();
        self.width = size;
        self.height = size;
        self
    }

    /// Set the color of the icon
    pub fn color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }
}

impl<'a, Message: 'a> From<Icon<'a, Message>> for Element<'a, Message> {
    fn from(icon: Icon<'a, Message>) -> Self {
        let handle = icons::Assets::get_svg_handle(icon.name.filename())
            .expect("Failed to load SVG icon");

        let mut svg_widget = svg(handle)
            .width(icon.width)
            .height(icon.height);

        if let Some(color) = icon.color {
            svg_widget = svg_widget.style(move |_theme, _status| svg::Style {
                color: Some(color),
            });
        }

        svg_widget.into()
    }
}
