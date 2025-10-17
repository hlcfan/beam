// Custom Text Input Demo
// 
// This is a conceptual example showing how the CustomTextInput widget would be used.
// To actually run this, you would need to integrate it into the main beam application
// or set up proper module imports.

use beam::ui::custom_text_input::CustomTextInput;
use iced::{Element, Length, Task, Event, keyboard, Color, Background, Border};
use iced::widget::{column, container, text, button, row, text_input, stack, mouse_area};

#[derive(Debug, Clone)]
pub enum Message {
    TextChanged(String),
    CustomTextInputChanged(String),
    TogglePassword,
    Clear,
    Undo,
    Redo,
    // Tooltip messages
    ShowTooltip(String, f32, f32),
    HideTooltip,
}

pub struct App {
    text_value: String,
    is_password: bool,
    custom_input: CustomTextInput,
    // History for undo/redo
    history: Vec<String>,
    history_index: usize,
    // Tooltip state
    tooltip_visible: bool,
    tooltip_text: String,
    tooltip_position: (f32, f32),
}

impl Default for App {
    fn default() -> Self {
        let initial_text = "https://{{host}}/api/{{version}}/users".to_string();
        Self {
            text_value: initial_text.clone(),
            is_password: false,
            custom_input: CustomTextInput::new("Enter text with variables like {{host}} or {{version}}".to_string())
                .with_syntax_highlighting(true)
                .with_variable_color(Color::from_rgb(0.2, 0.4, 0.8))
                .with_cursor_color(Color::from_rgb(0.1, 0.1, 0.1)) // Darker cursor color
                .width(Length::Fill),
            history: vec![initial_text],
            history_index: 0,
            tooltip_visible: false,
            tooltip_text: String::new(),
            tooltip_position: (0.0, 0.0),
        }
    }
}

impl App {
    fn create_syntax_highlighted_overlay(&self) -> Element<Message> {
        use regex::Regex;
        
        // Parse variables like {{variable_name}}
        let variable_regex = Regex::new(r"\{\{[^}]+\}\}").unwrap();
        let input_text = &self.text_value;
        
        if input_text.is_empty() {
            return container(text("")).into();
        }
        
        let mut elements: Vec<Element<Message>> = Vec::new();
        let mut last_end = 0;
        
        // Find all variable matches
        for mat in variable_regex.find_iter(input_text) {
            // Add normal text before the variable
            if mat.start() > last_end {
                let normal_text = &input_text[last_end..mat.start()];
                if !normal_text.is_empty() {
                    elements.push(
                        text(normal_text)
                            .size(16.0)
                            .color(Color::BLACK) // Make normal text black and visible
                            .into()
                    );
                }
            }
            
            // Add the variable with blue color and hover detection
            let variable_text = mat.as_str();
            let tooltip_text = format!("Variable: {}\nClick to edit this variable", variable_text);
            elements.push(
                mouse_area(
                    text(variable_text)
                        .size(16.0)
                        .color(Color::from_rgb(0.2, 0.4, 0.8)) // Blue color for variables
                )
                .on_enter(Message::ShowTooltip(tooltip_text, 0.0, 0.0))
                .on_exit(Message::HideTooltip)
                .into()
            );
            
            last_end = mat.end();
        }
        
        // Add remaining normal text
        if last_end < input_text.len() {
            let remaining_text = &input_text[last_end..];
            if !remaining_text.is_empty() {
                elements.push(
                    text(remaining_text)
                        .size(16.0)
                        .color(Color::BLACK) // Make normal text black and visible
                        .into()
                );
            }
        }
        
        // Create a row with all text segments
        if elements.is_empty() {
            container(text("")).into()
        } else {
            row(elements)
                .spacing(0)
                .into()
        }
    }

    fn create_non_interactive_overlay(&self) -> Vec<Element<Message>> {
        use regex::Regex;
        
        // Parse variables like {{variable_name}}
        let variable_regex = Regex::new(r"\{\{[^}]+\}\}").unwrap();
        let input_text = &self.text_value;
        
        if input_text.is_empty() {
            return vec![text("").into()];
        }
        
        let mut elements: Vec<Element<Message>> = Vec::new();
        let mut last_end = 0;
        
        // Find all variable matches
        for mat in variable_regex.find_iter(input_text) {
            // Add normal text before the variable
            if mat.start() > last_end {
                let normal_text = &input_text[last_end..mat.start()];
                if !normal_text.is_empty() {
                    elements.push(
                        text(normal_text)
                            .size(16.0)
                            .color(Color::BLACK) // Make normal text black and visible
                            .into()
                    );
                }
            }
            
            // Add the variable with blue color (no mouse interaction)
            let variable_text = mat.as_str();
            elements.push(
                text(variable_text)
                    .size(16.0)
                    .color(Color::from_rgb(0.2, 0.4, 0.8)) // Blue color for variables
                    .into()
            );
            
            last_end = mat.end();
        }
        
        // Add remaining normal text
        if last_end < input_text.len() {
            let remaining_text = &input_text[last_end..];
            if !remaining_text.is_empty() {
                elements.push(
                    text(remaining_text)
                        .size(16.0)
                        .color(Color::BLACK) // Make normal text black and visible
                        .into()
                );
            }
        }
        
        elements
    }

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
                self.custom_input = self.custom_input.clone().with_value(value.clone());
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
                self.custom_input = self.custom_input.clone().with_value(String::new());
                Task::none()
            }
            Message::Undo => {
                self.undo();
                self.custom_input = self.custom_input.clone().with_value(self.text_value.clone());
                Task::none()
            }
            Message::Redo => {
                self.redo();
                self.custom_input = self.custom_input.clone().with_value(self.text_value.clone());
                Task::none()
            }
            Message::ShowTooltip(text, x, y) => {
                self.tooltip_visible = true;
                self.tooltip_text = text;
                self.tooltip_position = (x, y);
                Task::none()
            }
            Message::HideTooltip => {
                self.tooltip_visible = false;
                self.tooltip_text.clear();
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
            
            // Use text_input with syntax highlighting overlay
            container(
                stack![
                    // Base text input (transparent text)
                    text_input("Enter text with {{variables}}", &self.text_value)
                        .on_input(Message::CustomTextInputChanged)
                        .size(16.0)
                        .padding(10)
                        .style(|theme: &iced::Theme, status: text_input::Status| {
                            text_input::Style {
                                background: Background::Color(Color::WHITE),
                                border: Border {
                                color: Color::from_rgb(0.7, 0.7, 0.7),
                                width: 1.0,
                                radius: 4.0.into(),
                            },
                            icon: Color::from_rgb(0.1, 0.1, 0.1), // Darker cursor color
                            placeholder: Color::from_rgb(0.6, 0.6, 0.6),
                            value: Color::from_rgba(0.0, 0.0, 0.0, 0.1), // Make input text very light but visible for cursor
                            selection: Color::from_rgba(0.2, 0.4, 0.8, 0.3),
                        }
                    }),
                    
                    // Syntax highlighting overlay with hover detection
                    container(
                        self.create_syntax_highlighted_overlay()
                    )
                    .padding(10)
                    .width(Length::Fill)
                ]
            ),
            
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

        // Add tooltip overlay if visible
        if self.tooltip_visible && !self.tooltip_text.is_empty() {
            stack![
                main_content,
                container(
                    text(&self.tooltip_text)
                        .size(12)
                        .color(Color::WHITE)
                )
                .padding(8)
                .style(|theme: &iced::Theme| {
                    container::Style {
                        background: Some(Background::Color(Color::from_rgba(0.0, 0.0, 0.0, 0.8))),
                        border: Border {
                            color: Color::from_rgb(0.3, 0.3, 0.3),
                            width: 1.0,
                            radius: 4.0.into(),
                        },
                        ..Default::default()
                    }
                })
            ].into()
        } else {
            main_content.into()
        }
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