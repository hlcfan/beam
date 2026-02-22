use crate::constant;
use crate::history::{History, TextInputCommand, diff_to_command};
use crate::ui::editor_view::{Action as UndoableAction, EditorView};
use constant::URL_INPUT_ID;
use iced::widget::text_input;
use iced::{Background, Border, Color, Element, Length, Theme};

#[derive(Debug, Clone)]
pub enum Message {
    Changed(String),
    Undo,
    Redo,
    None,
}

#[derive(Debug, Clone)]
pub struct UndoableInput {
    value: String,
    history: History<TextInputCommand>,
    placeholder: String,
    size: f32,
    padding: f32,
}

impl UndoableInput {
    pub fn new(initial_value: String, placeholder: String) -> Self {
        Self {
            value: initial_value,
            history: History::new(),
            placeholder,
            size: 14.0,
            padding: 8.0,
        }
    }

    pub fn new_empty(placeholder: String) -> Self {
        Self {
            value: String::new(),
            history: History::new(),
            placeholder,
            size: 14.0,
            padding: 8.0,
        }
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    pub fn padding(mut self, padding: f32) -> Self {
        self.padding = padding;
        self
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    /// Sync the internal baseline value without recording a history entry.
    /// Call this whenever the URL is set externally (e.g. loading a saved request).
    /// This also clears the undo history (the new value is the new baseline).
    pub fn set_value(&mut self, value: String) {
        self.value = value;
        self.history = crate::history::History::new();
    }

    /// Returns true if the user has made edits (history stack is not empty).
    pub fn has_history(&self) -> bool {
        self.history.can_undo()
    }

    /// Update the component with a message.
    /// Returns Some(new_value) if the value changed (for parent notification).
    pub fn update(&mut self, message: Message) -> Option<String> {
        match message {
            Message::Changed(new_value) => {
                if let Some(cmd) = diff_to_command(&self.value, &new_value) {
                    self.history.push(cmd);
                    self.value = new_value.clone();
                    Some(new_value)
                } else {
                    None
                }
            }
            Message::Undo => {
                if self.history.undo(&mut self.value) {
                    Some(self.value.clone())
                } else {
                    None
                }
            }
            Message::Redo => {
                if self.history.redo(&mut self.value) {
                    Some(self.value.clone())
                } else {
                    None
                }
            }
            Message::None => None,
        }
    }

    pub fn view<'a>(&'a self, value: &'a str) -> Element<'a, Message> {
        let input = text_input(&self.placeholder, value)
            .id(URL_INPUT_ID)
            .on_input(Message::Changed)
            .size(self.size)
            .padding(self.padding)
            .width(Length::Fill)
            .style(move |theme: &Theme, _status| {
                let palette = theme.palette();

                text_input::Style {
                    background: Background::Color(palette.background),
                    border: Border {
                        color: Color::TRANSPARENT,
                        width: 0.0,
                        radius: 4.0.into(),
                    },
                    icon: palette.text,
                    placeholder: palette.text,
                    value: palette.text,
                    selection: palette.primary,
                }
            });

        EditorView::new(input, |action| match action {
            UndoableAction::Undo => Message::Undo,
            UndoableAction::Redo => Message::Redo,
            UndoableAction::Find => Message::None,
            UndoableAction::ScrollToMatch(_) => Message::None,
        })
        .into()
    }
}
