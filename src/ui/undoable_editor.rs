use crate::ui::undoable::{Action as UndoableAction, Undoable};
use iced::widget::text_editor;
use iced::{Element, Length};

#[derive(Debug, Clone)]
pub struct UndoableEditor {
    height: Length,
}

#[derive(Debug, Clone)]
pub enum EditorMessage {
    Action(text_editor::Action),
    Undo,
    Redo,
}

impl UndoableEditor {
    pub fn new() -> Self {
        Self {
            height: Length::Fill,
        }
    }

    pub fn height(mut self, height: Length) -> Self {
        self.height = height;
        self
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
