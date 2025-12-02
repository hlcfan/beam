use crate::ui::undoable::{Action as UndoableAction, Undoable};
use crate::ui::undoable_input::UndoHistory;
use iced::widget::text_editor;
use iced::{Element, Length};
use log::info;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct UndoableEditor {
    history: UndoHistory,
    height: Length,
}

#[derive(Debug, Clone)]
pub enum EditorMessage {
    Action(text_editor::Action),
    Undo,
    Redo,
}

impl UndoableEditor {
    pub fn new(undo_history: UndoHistory, placeholder: String) -> Self {
        Self {
            history: undo_history,
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
        content: &mut text_editor::Content,
        message: EditorMessage,
    ) -> Option<String> {
        info!("===history: ");
        match message {
            EditorMessage::Action(action) => {
                content.perform(action);
                let text = content.text();
                self.history.push(text.clone());
                Some(text)
            }
            EditorMessage::Undo => {
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
            EditorMessage::Redo => {
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
        }
    }

    pub fn view<'a>(&self, content: &'a text_editor::Content) -> Element<'a, EditorMessage> {
        let editor = text_editor(content)
            .on_action(EditorMessage::Action)
            .height(self.height);

        Undoable::new(editor, |action| match action {
            UndoableAction::Undo => EditorMessage::Undo,
            UndoableAction::Redo => EditorMessage::Redo,
        })
        .into()
    }
}
