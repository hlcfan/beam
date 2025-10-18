// Custom Text Input Demo
//
// This is a conceptual example showing how the CustomTextInput widget would be used.
// To actually run this, you would need to integrate it into the main beam application
// or set up proper module imports.

use beam::ui::url_input::UrlInput;
use iced::{Element, Length, Task, Event, keyboard, Color, Background, Border, widget, Padding};
use iced::widget::{column, container, text, button, row, text_input};

const INPUT_ID: widget::Id = widget::Id::new("custom_input");

#[derive(Debug, Clone)]
pub enum Message {
    TextChanged(String),
    CustomTextInputChanged(String),
    TogglePassword,
    Clear,
    Undo,
    Redo,
}

pub struct App {
    text_value: String,
    is_password: bool,
    custom_input: UrlInput<Message>,
    // History for undo/redo
    history: Vec<String>,
    history_index: usize,
}

impl Default for App {
    fn default() -> Self {
        let initial_text = "https://{{host}}/api/{{version1}}/users/{{version2}}/users/{{version3}}/users/{{version4}}/users/{{version5}}/users/{{version6}}/users/{{version7}}/users/{{version8}}/users/{{version9}}/users/{{version10}}/users/{{version11}}/users/{{version12}}/users".to_string();
        let syntax_highlighting = beam::ui::url_input::SyntaxHighlighting {
            enabled: true,
            variable_color: Color::from_rgb(0.2, 0.4, 0.8),
            string_color: Color::from_rgb(0.0, 0.6, 0.0),
            number_color: Color::from_rgb(0.8, 0.4, 0.0),
            keyword_color: Color::from_rgb(0.6, 0.0, 0.8),
        };

        Self {
            text_value: initial_text.clone(),
            is_password: false,
            custom_input: UrlInput::new("Enter text with variables like {{host}} or {{version}}", &initial_text)
                .syntax_highlighting(syntax_highlighting)
                .width(Length::Fill)
                .id(INPUT_ID)
                .on_input(Message::CustomTextInputChanged),
            history: vec![initial_text],
            history_index: 0,
        }
    }
}

impl App {


    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::TextChanged(value) => {
                if value != self.text_value {
                    self.push_to_history(value.clone());
                    self.text_value = value;
                }
                Task::none()
            }
            Message::CustomTextInputChanged(value) => {
                // Update the custom input and sync with text_value
                self.custom_input = self.custom_input.clone().value(value.clone());
                // Add to history if the value is different
                if self.text_value != value {
                    self.push_to_history(value.clone());
                    self.text_value = value;
                }
                Task::none()
            }
            Message::TogglePassword => {
                self.is_password = !self.is_password;
                Task::none()
            }
            Message::Clear => {
                self.push_to_history(String::new());
                self.text_value.clear();
                self.custom_input = self.custom_input.clone().value(String::new());
                Task::none()
            }
            Message::Undo => {
                self.undo();
                self.custom_input = self.custom_input.clone().value(self.text_value.clone());
                Task::none()
            }
            Message::Redo => {
                self.redo();
                self.custom_input = self.custom_input.clone().value(self.text_value.clone());
                Task::none()
            }

        }
    }

    fn push_to_history(&mut self, value: String) {
        // Remove any future history if we're not at the end
        if self.history_index < self.history.len() - 1 {
            self.history.truncate(self.history_index + 1);
        }

        // Add new entry
        self.history.push(value);
        self.history_index = self.history.len() - 1;

        // Limit history size
        if self.history.len() > 50 {
            self.history.remove(0);
            self.history_index = self.history.len() - 1;
        }
    }

    fn undo(&mut self) {
        if self.history_index > 0 {
            self.history_index -= 1;
            self.text_value = self.history[self.history_index].clone();
        }
    }

    fn redo(&mut self) {
        if self.history_index < self.history.len() - 1 {
            self.history_index += 1;
            self.text_value = self.history[self.history_index].clone();
        }
    }

    fn can_undo(&self) -> bool {
        self.history_index > 0
    }

    fn can_redo(&self) -> bool {
        self.history_index < self.history.len() - 1
    }

    pub fn view(&self) -> Element<Message> {
        // Main input section with syntax highlighting
        let input_section = column![
            text("Text Input with Syntax Highlighting").size(20),
            text("Variables like {{host}}, {{port}}, {{version}} will be highlighted in blue").size(14).color(Color::from_rgb(0.6, 0.6, 0.6)),

            // Use our custom text input widget
            self.custom_input.view(),

            // Show current value info
            if !self.text_value.is_empty() {
                container(
                    text(format!("Current value: {}", self.text_value))
                        .size(14)
                        .color(Color::from_rgb(0.4, 0.4, 0.4))
                )
                .padding(10)
                .style(|theme: &iced::Theme| {
                    container::Style {
                        background: Some(Background::Color(Color::from_rgb(0.95, 0.95, 0.95))),
                        border: Border {
                            color: Color::from_rgb(0.8, 0.8, 0.8),
                            width: 1.0,
                            radius: 4.0.into(),
                        },
                        ..Default::default()
                    }
                })
            } else {
                container(
                    text("Type something with {{variables}} to see syntax highlighting inside the input")
                        .size(14)
                        .color(Color::from_rgb(0.6, 0.6, 0.6))
                )
                .padding(10)
            }
        ]
        .spacing(10);

        let main_content = container(
            column![
                text("Custom Text Input Demo with Syntax Highlighting").size(24),
                input_section,
                row![
                    button(if self.is_password { "Show Text" } else { "Show Password" })
                        .on_press(Message::TogglePassword),
                    button("Clear")
                        .on_press(Message::Clear),
                ].spacing(10),
                row![
                    button(text(format!("Undo (Cmd+Z){}", if self.can_undo() { "" } else { " - disabled" })))
                        .on_press_maybe(if self.can_undo() { Some(Message::Undo) } else { None }),
                    button(text(format!("Redo (Cmd+Shift+Z){}", if self.can_redo() { "" } else { " - disabled" })))
                        .on_press_maybe(if self.can_redo() { Some(Message::Redo) } else { None }),
                ].spacing(10),
                text("Undo/Redo functionality:").size(16),
                text("• Tracks text changes with timestamps").size(12),
                text("• Groups rapid changes together").size(12),
                text("• Preserves cursor position and selection").size(12),
                text("• Supports keyboard shortcuts (Cmd+Z, Cmd+Shift+Z)").size(12),
                text("• Hover over variables to see tooltips").size(12).color(Color::from_rgb(0.2, 0.4, 0.8)),
            ]
            .spacing(20)
            .padding(20)
        )
        .width(Length::Fill)
        .height(Length::Fill);

        main_content.into()
    }
}

impl App {
    fn subscription(&self) -> iced::Subscription<Message> {
        iced::event::listen_with(|event, _status, _id| {
            match event {
                Event::Keyboard(keyboard::Event::KeyPressed {
                    key,
                    modifiers,
                    ..
                }) => {
                    match key {
                        keyboard::Key::Character(ref c) if c == "z" => {
                            if modifiers.command() && modifiers.shift() {
                                Some(Message::Redo)
                            } else if modifiers.command() {
                                Some(Message::Undo)
                            } else {
                                None
                            }
                        }
                        _ => None
                    }
                }
                _ => None
            }
        })
    }
}

pub fn main() -> iced::Result {
    iced::application(
        || (App::default(), Task::none()),
        App::update,
        App::view,
    )
    .subscription(App::subscription)
    .title("Custom Text Input Demo")
    .run()
}