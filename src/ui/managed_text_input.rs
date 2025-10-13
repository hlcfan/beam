use iced::widget::{text_input, mouse_area, stack, Space};
use iced::{Element, Length, Background, Border, Color, Theme};
use crate::types::{Message, Environment};
use regex::Regex;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub struct HistoryEntry {
    pub value: String,
    pub timestamp: u64, // milliseconds since epoch
}

impl HistoryEntry {
    pub fn new(value: String) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        
        Self { value, timestamp }
    }
}

#[derive(Debug, Clone)]
pub struct ManagedTextInput {
    pub value: String,
    pub placeholder: String,
    pub is_focused: bool,
    pub history: Vec<HistoryEntry>,
    pub history_index: usize,
    pub max_history: usize,
    pub grouping_threshold_ms: u64, // Time threshold for grouping changes (500ms)
}

impl ManagedTextInput {
    pub fn new(placeholder: String) -> Self {
        Self {
            value: String::new(),
            placeholder,
            is_focused: false,
            history: vec![HistoryEntry::new(String::new())],
            history_index: 0,
            max_history: 50, // Limit history to prevent memory issues
            grouping_threshold_ms: 200, // 500ms threshold for grouping changes
        }
    }

    pub fn with_value(mut self, value: String) -> Self {
        self.value = value.clone();
        self.history = vec![HistoryEntry::new(value)];
        self.history_index = 0;
        self
    }

    pub fn set_value(&mut self, value: String) {
        if self.value != value {
            self.value = value.clone();
            self.push_to_history(value);
        }
    }

    pub fn set_value_without_history(&mut self, value: String) {
        self.value = value;
    }

    pub fn push_to_history(&mut self, value: String) {
        // Remove any future history if we're not at the end
        if self.history_index < self.history.len() - 1 {
            self.history.truncate(self.history_index + 1);
        }

        let new_entry = HistoryEntry::new(value);
        
        // Check if we should group with the last entry (time-based grouping)
        if let Some(last_entry) = self.history.last() {
            let time_diff = new_entry.timestamp - last_entry.timestamp;
            
            // If the time difference is within the threshold, replace the last entry
            if time_diff <= self.grouping_threshold_ms && self.history_index == self.history.len() - 1 {
                self.history[self.history_index] = new_entry;
                return;
            }
        }

        // Add new entry to history (creates a new undo group)
        self.history.push(new_entry);
        self.history_index = self.history.len() - 1;

        // Limit history size
        if self.history.len() > self.max_history {
            self.history.remove(0);
            self.history_index = self.history.len() - 1;
        }
    }

    pub fn undo(&mut self) -> bool {
        if self.history_index > 0 {
            self.history_index -= 1;
            self.value = self.history[self.history_index].value.clone();
            true
        } else {
            false
        }
    }

    pub fn redo(&mut self) -> bool {
        if self.history_index < self.history.len() - 1 {
            self.history_index += 1;
            self.value = self.history[self.history_index].value.clone();
            true
        } else {
            false
        }
    }

    pub fn set_focused(&mut self, focused: bool) {
        self.is_focused = focused;
    }

    pub fn view<'a>(
        &'a self,
        environments: &'a [Environment],
        active_environment: Option<usize>,
    ) -> Element<'a, Message> {
        // Create the base text input
        let input = text_input(&self.placeholder, &self.value)
            .on_input(|value| Message::ManagedUrlChanged(value))
            .width(Length::Fill)
            .style(|theme: &Theme, _status: text_input::Status| text_input::Style {
                border: Border {
                    color: Color::TRANSPARENT,
                    width: 0.0,
                    radius: 0.0.into(),
                },
                background: Background::Color(theme.palette().background),
                icon: Color::TRANSPARENT,
                placeholder: theme.palette().text,
                value: theme.palette().text,
                selection: theme.palette().primary,
            });

        // Create a transparent overlay for tooltip detection
        let overlay = mouse_area(
            Space::new()
                .width(Length::Fill)
                .height(Length::Fill)
        )
        .on_move(move |_point| {
            // Detect all environment variables in the URL
            let re = Regex::new(r"\{\{([^}]+)\}\}").unwrap();

            let mut variables = Vec::new();
            for captures in re.captures_iter(&self.value) {
                let variable_name = captures.get(1).unwrap().as_str().to_string();
                let variable_value = if let Some(active_idx) = active_environment {
                    if let Some(env) = environments.get(active_idx) {
                        env.get_variable(&variable_name)
                            .map(|v| v.clone())
                            .unwrap_or_else(|| "undefined".to_string())
                    } else {
                        "undefined".to_string()
                    }
                } else {
                    "undefined".to_string()
                };
                variables.push((variable_name, variable_value));
            }

            if !variables.is_empty() {
                // Show all variables in the tooltip, one per line
                let all_vars = variables
                    .iter()
                    .map(|(name, value)| format!("{}: {}", name, value))
                    .collect::<Vec<_>>()
                    .join("\n");

                Message::ShowUrlTooltip(
                    "Variables".to_string(),
                    all_vars,
                    20.0,  // Fixed left padding
                    100.0, // Fixed position above URL input
                )
            } else {
                Message::HideUrlTooltip
            }
        })
        .on_exit(Message::HideUrlTooltip);

        // Stack the input and overlay
        stack![input, overlay].into()
    }
}