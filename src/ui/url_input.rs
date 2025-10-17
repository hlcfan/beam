use iced::widget::{text, row, column, container, stack, mouse_area, Space, text_input, tooltip};
use iced::{Element, Length, Background, Border, Color, Theme, Alignment, Shadow, Vector, Point, Size, Rectangle, Padding, widget};
use iced::event::{Event, Status};
use iced::keyboard::{self, Key, Modifiers};
use iced::mouse::{self, Button, Cursor};
use std::time::{Duration, Instant};
use std::marker::PhantomData;
use regex::Regex;

#[derive(Debug, Clone)]
pub struct HistoryEntry {
    pub value: String,
    pub cursor_position: usize,
    pub selection: Option<(usize, usize)>,
    pub timestamp: u64,
}

impl HistoryEntry {
    pub fn new(value: String, cursor_position: usize, selection: Option<(usize, usize)>) -> Self {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        Self {
            value,
            cursor_position,
            selection,
            timestamp,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TextSegment {
    pub text: String,
    pub segment_type: SegmentType,
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SegmentType {
    Normal,
    Variable,
    String,
    Number,
    Keyword,
}

#[derive(Debug, Clone)]
pub struct SyntaxHighlighting {
    pub enabled: bool,
    pub variable_color: Color,
    pub string_color: Color,
    pub number_color: Color,
    pub keyword_color: Color,
}

impl Default for SyntaxHighlighting {
    fn default() -> Self {
        Self {
            enabled: true, // Enabled by default with improved overlay implementation
            variable_color: Color::from_rgb(0.2, 0.4, 0.8),
            string_color: Color::from_rgb(0.0, 0.6, 0.0),
            number_color: Color::from_rgb(0.8, 0.4, 0.0),
            keyword_color: Color::from_rgb(0.6, 0.0, 0.8),
        }
    }
}

/// A URL input widget that wraps iced's text_input with additional features
#[derive(Debug, Clone)]
pub struct UrlInput<Message>
where
    Message: Clone,
{
    value: String,
    placeholder: String,
    is_secure: bool,
    width: Length,
    height: Length,
    padding: Padding,
    size: f32,
    font: Option<iced::Font>,
    syntax_highlighting: SyntaxHighlighting,
    max_history: usize,
    grouping_threshold_ms: u64,
    on_input: Option<fn(String) -> Message>,
    on_submit: Option<Message>,
    on_paste: Option<fn(String) -> Message>,
    id: Option<widget::Id>,
    history: Vec<HistoryEntry>,
    history_index: usize,
    _phantom: PhantomData<Message>,
}

impl<Message> UrlInput<Message>
where
    Message: Clone,
{
    pub fn new(placeholder: &str, value: &str) -> Self {
        Self {
            value: value.to_string(),
            placeholder: placeholder.to_string(),
            is_secure: false,
            width: Length::Fill,
            height: Length::Shrink,
            padding: Padding::new(8.0),
            size: 14.0,
            font: None,
            syntax_highlighting: SyntaxHighlighting::default(),
            max_history: 50,
            grouping_threshold_ms: 1000,
            on_input: None,
            on_submit: None,
            on_paste: None,
            id: None,
            history: Vec::new(),
            history_index: 0,
            _phantom: PhantomData,
        }
    }
}

impl<Message> Default for UrlInput<Message>
where
    Message: Clone,
{
    fn default() -> Self {
        Self {
            value: String::new(),
            placeholder: String::new(),
            is_secure: false,
            width: Length::Fill,
            height: Length::Shrink,
            padding: Padding::new(8.0),
            size: 14.0,
            font: None,
            syntax_highlighting: SyntaxHighlighting::default(),
            max_history: 100,
            grouping_threshold_ms: 500,
            on_input: None,
            on_submit: None,
            on_paste: None,
            id: None,
            history: Vec::new(),
            history_index: 0,
            _phantom: PhantomData,
        }
    }
}

impl<Message> UrlInput<Message>
where
    Message: Clone,
{
    pub fn placeholder(mut self, placeholder: String) -> Self {
        self.placeholder = placeholder;
        self
    }

    pub fn value(mut self, value: String) -> Self {
        self.value = value;
        self
    }

    pub fn secure(mut self, secure: bool) -> Self {
        self.is_secure = secure;
        self
    }

    pub fn width(mut self, width: Length) -> Self {
        self.width = width;
        self
    }

    pub fn height(mut self, height: Length) -> Self {
        self.height = height;
        self
    }

    pub fn padding(mut self, padding: impl Into<Padding>) -> Self {
        self.padding = padding.into();
        self
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    pub fn font(mut self, font: iced::Font) -> Self {
        self.font = Some(font);
        self
    }

    pub fn on_input(mut self, callback: fn(String) -> Message) -> Self {
        self.on_input = Some(callback);
        self
    }

    pub fn on_submit(mut self, message: Message) -> Self {
        self.on_submit = Some(message);
        self
    }

    pub fn on_paste(mut self, callback: fn(String) -> Message) -> Self {
        self.on_paste = Some(callback);
        self
    }

    pub fn syntax_highlighting(mut self, highlighting: SyntaxHighlighting) -> Self {
        self.syntax_highlighting = highlighting;
        self
    }

    pub fn max_history(mut self, max: usize) -> Self {
        self.max_history = max;
        self
    }

    pub fn id(mut self, id: widget::Id) -> Self {
        self.id = Some(id);
        self
    }

    pub fn set_value(&mut self, value: String) {
        self.value = value;
    }

    fn parse_text_segments(&self) -> Vec<TextSegment> {
        let mut segments = Vec::new();
        let text = &self.value;

        if !self.syntax_highlighting.enabled {
            segments.push(TextSegment {
                text: text.clone(),
                segment_type: SegmentType::Normal,
                start: 0,
                end: text.len(),
            });
            return segments;
        }

        // Parse variables like {{variable_name}}
        let variable_regex = Regex::new(r"\{\{[^}]+\}\}").unwrap();
        let mut last_end = 0;

        for mat in variable_regex.find_iter(text) {
            // Add normal text before the variable
            if mat.start() > last_end {
                let normal_text = &text[last_end..mat.start()];
                segments.push(TextSegment {
                    text: normal_text.to_string(),
                    segment_type: SegmentType::Normal,
                    start: last_end,
                    end: mat.start(),
                });
            }

            // Add the variable
            segments.push(TextSegment {
                text: mat.as_str().to_string(),
                segment_type: SegmentType::Variable,
                start: mat.start(),
                end: mat.end(),
            });

            last_end = mat.end();
        }

        // Add remaining normal text
        if last_end < text.len() {
            let normal_text = &text[last_end..];
            segments.push(TextSegment {
                text: normal_text.to_string(),
                segment_type: SegmentType::Normal,
                start: last_end,
                end: text.len(),
            });
        }

        segments
    }

    fn create_syntax_overlay(&self) -> Element<Message> {
        let segments = self.parse_text_segments();

        if segments.is_empty() || !self.syntax_highlighting.enabled {
            return container(Space::new()).into();
        }

        let mut row_elements: Vec<Element<Message>> = Vec::new();

        for segment in segments {
            let color = match segment.segment_type {
                SegmentType::Variable => self.syntax_highlighting.variable_color,
                SegmentType::String => self.syntax_highlighting.string_color,
                SegmentType::Number => self.syntax_highlighting.number_color,
                SegmentType::Keyword => self.syntax_highlighting.keyword_color,
                SegmentType::Normal => Color::TRANSPARENT, // Let the underlying text show through
            };

            let segment_text = segment.text.clone(); // Clone to avoid lifetime issues
            let segment_type = segment.segment_type.clone();

            if segment.segment_type != SegmentType::Normal {
                let text_element = text(segment_text.clone())
                    .size(self.size)
                    .color(color);

                // Add tooltip for variable segments
                if segment.segment_type == SegmentType::Variable {
                    let tooltip_content = container(
                        text(format!("Variable: {}", segment_text.clone()))
                            .size(14)
                            .color(Color::WHITE)
                    )
                    .padding(8)
                    .style(|_theme: &Theme| container::Style {
                        background: Some(Background::Color(Color::from_rgb(0.2, 0.2, 0.2))),
                        border: Border {
                            color: Color::from_rgb(0.6, 0.6, 0.6),
                            width: 1.0,
                            radius: 4.0.into(),
                        },
                        shadow: Shadow {
                            color: Color::from_rgba(0.0, 0.0, 0.0, 0.3),
                            offset: Vector::new(0.0, 2.0),
                            blur_radius: 4.0,
                        },
                        text_color: Some(Color::WHITE),
                        snap: false,
                    });

                    row_elements.push(
                        tooltip(
                            text_element,
                            tooltip_content,
                            tooltip::Position::Top
                        )
                        .into()
                    );
                } else {
                    row_elements.push(text_element.into());
                }
            } else {
                // For normal text, add transparent space to maintain positioning
                row_elements.push(
                    text(segment_text)
                        .size(self.size)
                        .color(Color::TRANSPARENT)
                        .into()
                );
            }
        }

        if row_elements.is_empty() {
            container(Space::new()).into()
        } else {
            // Create the overlay wrapped in a mouse_area that ignores all events
            mouse_area(
                container(
                    row(row_elements)
                        .align_y(Alignment::Center)
                )
                .padding(self.padding)
            )
            .into()
        }
    }



    pub fn view(&self) -> Element<Message> {
        let mut input = if self.is_secure {
            text_input(&self.placeholder, &self.value)
                .secure(true)
        } else {
            text_input(&self.placeholder, &self.value)
        };

        input = input
            .width(self.width)
            .size(self.size)
            .padding(self.padding);

        if let Some(id) = &self.id {
            input = input.id(id.clone());
        }

        if let Some(font) = self.font {
            input = input.font(font);
        }

        if let Some(callback) = self.on_input {
            input = input.on_input(callback);
        }

        if let Some(ref message) = self.on_submit {
            input = input.on_submit(message.clone());
        }

        // Create the base input with custom styling
        let styled_input = input.style(|theme: &Theme, status| {
            let palette = theme.palette();

            text_input::Style {
                background: Background::Color(palette.background),
                border: Border {
                    color: match status {
                        text_input::Status::Active => palette.primary,
                        text_input::Status::Hovered => palette.primary,
                        text_input::Status::Focused { .. } => palette.primary,
                        text_input::Status::Disabled => palette.text,
                    },
                    width: 1.0,
                    radius: 4.0.into(),
                },
                icon: palette.text,
                placeholder: palette.text,
                value: palette.text,
                selection: palette.primary,
            }
        });

        // Create the final element with syntax highlighting if enabled
        if self.syntax_highlighting.enabled && !self.value.is_empty() {
            stack![
                styled_input,
                self.create_syntax_overlay()
            ]
            .into()
        } else {
            styled_input.into()
        }
    }
}

/// Wrapper for CustomTextInput with callback support
pub struct UrlInputWithCallback<Message, F>
where
    Message: Clone,
{
    input: UrlInput<Message>,
    _phantom: PhantomData<F>,
}

impl<Message, F> UrlInputWithCallback<Message, F>
where
    F: Fn(String) -> Message + 'static,
    Message: Clone,
{
    pub fn new(placeholder: &str, value: &str, callback: F) -> Self {
        // Convert the closure to a function pointer
        // This is a limitation - we can only use function pointers, not closures
        let input = UrlInput::new(placeholder, value);

        Self {
            input,
            _phantom: PhantomData,
        }
    }

    pub fn view(&self) -> Element<Message> {
        self.input.view()
    }
}