use iced::{
    advanced::{
        layout::{Limits, Node},
        renderer::{self, Quad},
        text::{self, Text},
        widget::{Tree, Widget},
        Layout,
    },
    event::Event,
    mouse::Cursor,
    widget::text_input,
    Background, Border, Color, Element, Length, Padding, Rectangle, Shadow, Size, Theme, Vector,
};
use std::marker::PhantomData;

pub struct SegmentedTextInput<'a, Message> {
    value: String,
    placeholder: String,
    on_input: Option<Box<dyn Fn(String) -> Message + 'a>>,
    width: Length,
    padding: Padding,
    _phantom: PhantomData<Message>,
}

impl<'a, Message> SegmentedTextInput<'a, Message> {
    pub fn new(value: &str) -> Self {
        Self {
            value: value.to_string(),
            placeholder: String::new(),
            on_input: None,
            width: Length::Fill,
            padding: Padding::new(8.0),
            _phantom: PhantomData,
        }
    }

    pub fn placeholder(mut self, placeholder: &str) -> Self {
        self.placeholder = placeholder.to_string();
        self
    }

    pub fn on_input<F>(mut self, callback: F) -> Self
    where
        F: Fn(String) -> Message + 'a,
    {
        self.on_input = Some(Box::new(callback));
        self
    }

    pub fn width(mut self, width: Length) -> Self {
        self.width = width;
        self
    }

    pub fn padding(mut self, padding: Padding) -> Self {
        self.padding = padding;
        self
    }
}

impl<'a, Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for SegmentedTextInput<'a, Message>
where
    Renderer: iced::advanced::Renderer + iced::advanced::text::Renderer,
    Theme: text_input::Catalog,
{
    fn size(&self) -> Size<Length> {
        Size::new(self.width, Length::Shrink)
    }

    fn layout(
        &mut self,
        _tree: &mut Tree,
        _renderer: &Renderer,
        limits: &Limits,
    ) -> Node {
        let size = limits.resolve(self.width, Length::Shrink, Size::ZERO);
        Node::new(size)
    }

    fn draw(
        &self,
        _tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: Cursor,
        _viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        let style = text_input::Style {
            background: Background::Color(Color::WHITE),
            border: Border::default(),
            icon: Color::BLACK,
            placeholder: Color::from_rgb(0.5, 0.5, 0.5),
            value: Color::BLACK,
            selection: Color::from_rgb(0.0, 0.5, 1.0),
        };

        // Draw background using correct Quad structure
        renderer.fill_quad(
            Quad {
                bounds,
                border: Border::default(),
                shadow: Shadow::default(),
                snap: true,
            },
            style.background,
        );

        // Draw text
        let text_color = style.value;
        let content = if !self.value.is_empty() {
            &self.value
        } else {
            &self.placeholder
        };

        if !content.is_empty() {
            let text_bounds = Rectangle::new(
                bounds.position() + Vector::new(self.padding.left, self.padding.top),
                bounds.size(),
            );

            let text_bounds = Rectangle::new(
                bounds.position() + Vector::new(self.padding.left, self.padding.top),
                bounds.size(),
            );

            renderer.fill_paragraph(
                &text::Paragraph::with_text(
                    Text {
                        content,
                        bounds: text_bounds.size(),
                        size: iced::Pixels(14.0),
                        line_height: text::LineHeight::default(),
                        font: renderer.default_font(),
                        align_x: text::Alignment::Left,
                        align_y: iced::alignment::Vertical::Center,
                        shaping: text::Shaping::Advanced,
                        wrapping: text::Wrapping::default(),
                    }
                ),
                text_bounds.position(),
                text_color,
                text_bounds,
            );
        }
    }
}

impl<'a, Message, Theme, Renderer> From<SegmentedTextInput<'a, Message>> for Element<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: iced::widget::text_input::Catalog + 'a,
    Renderer: iced::advanced::text::Renderer + 'a,
{
    fn from(widget: SegmentedTextInput<'a, Message>) -> Self {
        Self::new(widget)
    }
}

pub fn segmented_text_input<'a, Message>(value: &str) -> SegmentedTextInput<'a, Message> {
    SegmentedTextInput::new(value)
}