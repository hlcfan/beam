use crate::ui::undoable::{Action as UndoableAction, Undoable};
use iced::widget::text_editor;
use iced::{Element, Length};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
struct UndoHistory {
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
    Action(text_editor::Action),
    Undo,
    Redo,
    Find,
}

#[derive(Debug, Clone)]
pub struct UndoableEditor {
    history: UndoHistory,
    height: Length,
}

impl UndoableEditor {
    pub fn new(initial_text: String) -> Self {
        Self {
            history: UndoHistory::new(initial_text),
            height: Length::Fill,
        }
    }

    pub fn height(mut self, height: Length) -> Self {
        self.height = height;
        self
    }

    /// Update the component with a message.
    /// Returns Some(new_text) if the text changed (for parent notification).
    pub fn update(
        &mut self,
        message: Message,
        content: &mut text_editor::Content,
    ) -> Option<String> {
        match message {
            Message::Action(action) => {
                content.perform(action);
                let text = content.text();
                if text != self.history.current {
                    self.history.push(text.clone());
                    Some(text)
                } else {
                    None
                }
            }
            Message::Undo => {
                if let Some(prev) = self.history.undo() {
                    // Use perform to update content while preserving cursor position
                    content.perform(text_editor::Action::SelectAll);
                    content.perform(text_editor::Action::Edit(text_editor::Edit::Paste(
                        std::sync::Arc::new(prev.clone()),
                    )));
                    // Move cursor to end
                    content.perform(text_editor::Action::Move(text_editor::Motion::DocumentEnd));
                    Some(prev)
                } else {
                    None
                }
            }
            Message::Redo => {
                if let Some(next) = self.history.redo() {
                    // Use perform to update content while preserving cursor position
                    content.perform(text_editor::Action::SelectAll);
                    content.perform(text_editor::Action::Edit(text_editor::Edit::Paste(
                        std::sync::Arc::new(next.clone()),
                    )));
                    // Move cursor to end
                    content.perform(text_editor::Action::Move(text_editor::Motion::DocumentEnd));
                    Some(next)
                } else {
                    None
                }
            }
            Message::Find => None,
        }
    }

    pub fn view<'a>(&'a self, content: &'a text_editor::Content) -> Element<'a, Message> {
        let editor = text_editor(content)
            .on_action(Message::Action)
            .height(self.height);

        Undoable::new(editor, |action| match action {
            UndoableAction::Undo => Message::Undo,
            UndoableAction::Redo => Message::Redo,
            UndoableAction::Find => Message::Find,
        })
        .into()
    }
}
