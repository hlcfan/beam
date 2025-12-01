use crate::ui::undoable::{Action as UndoableAction, Undoable};
use crate::ui::url_input::UrlInput;
use iced::{Element, Length};
use log::info;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct UndoHistory {
    past: Vec<String>,
    future: Vec<String>,
    current: Option<String>,
    last_snapshot_time: Instant,
    debounce_duration: Duration,
}

impl UndoHistory {
    pub fn new() -> Self {
        Self {
            past: Vec::new(),
            future: Vec::new(),
            current: None,
            last_snapshot_time: Instant::now(),
            debounce_duration: Duration::from_millis(500),
        }
    }

    pub fn set_initial(&mut self, initial: String) {
        self.current = Some(initial);
    }

    pub fn push(&mut self, new_state: String) {
        if let Some(current) = &self.current {
            if *current == new_state {
                return;
            }
        }

        let now = Instant::now();
        let time_since_last = now.duration_since(self.last_snapshot_time);

        // If enough time passed or past is empty, save current to past
        if let Some(current) = &self.current {
            if time_since_last >= self.debounce_duration || self.past.is_empty() {
                self.past.push(current.clone());
                self.last_snapshot_time = now;
            }
        }

        self.current = Some(new_state);
        self.future.clear();
    }

    pub fn undo(&mut self) -> Option<String> {
        let current = self.current.as_ref()?;

        if let Some(prev) = self.past.pop() {
            self.future.push(current.clone());
            self.current = Some(prev.clone());
            Some(prev)
        } else {
            None
        }
    }

    pub fn redo(&mut self) -> Option<String> {
        let current = self.current.as_ref()?;

        if let Some(next) = self.future.pop() {
            self.past.push(current.clone());
            self.current = Some(next.clone());
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
    Submit,
}

#[derive(Debug, Clone)]
pub struct UndoableInput {
    value: String,
    history: UndoHistory,
    placeholder: String,
    width: Length,
    on_submit: Option<Message>,
}

impl UndoableInput {
    pub fn new(initial_value: String, undo_history: UndoHistory, placeholder: String) -> Self {
        Self {
            value: initial_value.clone(),
            history: undo_history,
            placeholder,
            width: Length::Fill,
            on_submit: None,
        }
    }

    pub fn width(mut self, width: Length) -> Self {
        self.width = width;
        self
    }

    pub fn on_submit(mut self, message: Message) -> Self {
        self.on_submit = Some(message);
        self
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    pub fn set_value(&mut self, value: String) {
        if self.value != value {
            self.history.push(value.clone());
            self.value = value;
        }
    }

    /// Update the component with a message.
    /// Returns Some(new_value) if the value changed (for parent notification).
    pub fn update(&mut self, message: Message) -> Option<String> {
        info!("=========undoable input update: {:?}", message);
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
            Message::Submit => None, // Handled by parent if needed, or just triggers on_submit
        }
    }

    pub fn view(self) -> Element<'static, Message> {
        let mut input = UrlInput::new(&self.placeholder, &self.value)
            .on_input(Message::Changed)
            .width(self.width);

        if self.on_submit.is_some() {
            input = input.on_submit(Message::Submit);
        }

        Undoable::new(input, |action| match action {
            UndoableAction::Undo => Message::Undo,
            UndoableAction::Redo => Message::Redo,
        })
        .into()
    }
}
