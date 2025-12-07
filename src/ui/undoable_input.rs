use crate::ui::undoable::{Action as UndoableAction, Undoable};
use iced::widget::text_input;
use iced::{Element, Length};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct UndoHistory {
    past: Vec<String>,
    future: Vec<String>,
    current: String,
    last_snapshot_time: Instant,
    debounce_duration: Duration,
}

impl UndoHistory {
    fn new(initial: String) -> Self {
        Self {
            past: Vec::new(),
            future: Vec::new(),
            current: initial,
            last_snapshot_time: Instant::now(),
            debounce_duration: Duration::from_millis(500),
        }
    }

    fn push(&mut self, new_state: String) {
        if self.current == new_state {
            return;
        }

        let now = Instant::now();
        let time_since_last = now.duration_since(self.last_snapshot_time);

        // If enough time passed or past is empty, save current to past
        if time_since_last >= self.debounce_duration || self.past.is_empty() {
            self.past.push(self.current.clone());
            self.last_snapshot_time = now;
        }

        self.current = new_state;
        self.future.clear();
    }

    fn undo(&mut self) -> Option<String> {
        if let Some(prev) = self.past.pop() {
            self.future.push(self.current.clone());
            self.current = prev.clone();
            Some(prev)
        } else {
            None
        }
    }

    fn redo(&mut self) -> Option<String> {
        if let Some(next) = self.future.pop() {
            self.past.push(self.current.clone());
            self.current = next.clone();
            Some(next)
        } else {
            None
        }
    }
}

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
    history: UndoHistory,
    placeholder: String,
    size: f32,
    padding: f32,
}

impl UndoableInput {
    pub fn new(initial_value: String, placeholder: String) -> Self {
        Self {
            value: initial_value.clone(),
            history: UndoHistory::new(initial_value),
            placeholder,
            size: 16.0,
            padding: 10.0,
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

    /// Update the component with a message.
    /// Returns Some(new_value) if the value changed (for parent notification).
    pub fn update(&mut self, message: Message) -> Option<String> {
        match message {
            Message::Changed(new_value) => {
                self.history.push(new_value.clone());
                self.value = new_value.clone();
                Some(new_value)
            }
            Message::Undo => {
                if let Some(prev) = self.history.undo() {
                    self.value = prev.clone();
                    Some(prev)
                } else {
                    None
                }
            }
            Message::Redo => {
                if let Some(next) = self.history.redo() {
                    self.value = next.clone();
                    Some(next)
                } else {
                    None
                }
            }
            Message::None => None,
        }
    }

    pub fn view<'a>(&'a self, value: &'a str) -> Element<'a, Message> {
        let input = text_input(&self.placeholder, value)
            .on_input(Message::Changed)
            .size(self.size)
            .padding(self.padding)
            .width(Length::Fill);

        Undoable::new(input, |action| match action {
            UndoableAction::Undo => Message::Undo,
            UndoableAction::Redo => Message::Redo,
            UndoableAction::Find => Message::None,
        })
        .into()
    }
}
