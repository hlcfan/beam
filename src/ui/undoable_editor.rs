use crate::constant::REQUEST_BODY_EDITOR_ID;
use crate::history::UndoHistory;
use crate::ui::undoable::{Action as UndoableAction, Undoable};
use iced::widget::text_editor;
use iced::{Element, Length};
use log::info;

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

    pub fn new_empty() -> Self {
        Self {
            history: UndoHistory::new_empty(),
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
                info!("===Action: {:?}", action);
                content.perform(action);
                let text = content.text();
                if self.history.current().as_ref() != Some(&text) {
                    info!("====diff");
                    self.history.push(text.clone());
                    Some(text)
                } else {
                    info!("====same");
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

    pub fn view<'a>(
        &'a self,
        content: &'a text_editor::Content,
        syntax: Option<&'a str>,
    ) -> Element<'a, Message> {
        if let Some(syntax) = syntax {
            let editor = text_editor(content)
                .id(REQUEST_BODY_EDITOR_ID)
                .on_action(Message::Action)
                .highlight(syntax, iced::highlighter::Theme::SolarizedDark);

            Undoable::new(editor, |action| match action {
                UndoableAction::Undo => Message::Undo,
                UndoableAction::Redo => Message::Redo,
                UndoableAction::Find => Message::Find,
            })
            .into()
        } else {
            let editor = text_editor(content)
                .id(REQUEST_BODY_EDITOR_ID)
                .on_action(Message::Action);

            Undoable::new(editor, |action| match action {
                UndoableAction::Undo => Message::Undo,
                UndoableAction::Redo => Message::Redo,
                UndoableAction::Find => Message::Find,
            })
            .into()
        }
    }
}
