use crate::history::UndoHistory;
use crate::ui::undoable::{Action as UndoableAction, Undoable};
use iced::advanced::text;
use iced::widget::text_editor;
use iced::{Element, Length, Theme};

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
    version: usize,
}

impl UndoableEditor {
    pub fn new(initial_text: String) -> Self {
        Self {
            history: UndoHistory::new(initial_text),
            height: Length::Fill,
            version: 0,
        }
    }

    pub fn new_empty() -> Self {
        Self {
            history: UndoHistory::new_empty(),
            height: Length::Fill,
            version: 0,
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
                if self.history.current().as_ref() != Some(&text) {
                    self.history.push(text.clone());
                    self.version += 1;
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
                    // Don't increment version - content dimensions haven't changed
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
                    // Don't increment version - content dimensions haven't changed
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
        editor_id: impl Into<iced::widget::Id>,
        content: &'a text_editor::Content,
        syntax: Option<&'a str>,
        search_selection: Option<(text_editor::Position, text_editor::Position)>,
    ) -> Element<'a, Message> {
        let editor_id = editor_id.into();

        // Add debug log for search selection
        if let Some((start, end)) = search_selection {
            log::info!(
                "UndoableEditor::view - search_selection: start={:?}, end={:?}",
                start,
                end
            );
        } else {
            log::info!("UndoableEditor::view - search_selection: None");
        }

        // Create editor with or without syntax highlighting
        if let Some(syntax) = syntax {
            let editor = text_editor(content)
                .id(editor_id.clone())
                .on_action(Message::Action)
                .highlight(syntax, iced::highlighter::Theme::SolarizedDark)
                .font(iced::Font::MONOSPACE)
                .size(14)
                // .wrapping(text::Wrapping::None)
                // .height(self.height)
                .style(Self::editor_style);

            Self::wrap_in_undoable(editor, content, search_selection, self.version)
        } else {
            let editor = text_editor(content)
                .id(editor_id)
                .on_action(Message::Action)
                .font(iced::Font::MONOSPACE)
                .size(14)
                // .wrapping(text::Wrapping::None)
                // .height(self.height)
                .style(Self::editor_style);

            Self::wrap_in_undoable(editor, content, search_selection, self.version)
        }
    }

    fn editor_style(theme: &Theme, _status: text_editor::Status) -> text_editor::Style {
        text_editor::Style {
            background: iced::Background::Color(theme.palette().background),
            border: iced::Border {
                color: iced::Color::from_rgb(0.9, 0.9, 0.9),
                width: 1.0,
                radius: 4.0.into(),
            },
            placeholder: iced::Color::from_rgb(0.6, 0.6, 0.6),
            value: theme.palette().text,
            selection: theme.palette().primary,
        }
    }

    fn wrap_in_undoable<'a>(
        editor: impl Into<Element<'a, Message>>,
        content: &'a text_editor::Content,
        selection: Option<(text_editor::Position, text_editor::Position)>,
        version: usize,
    ) -> Element<'a, Message> {
        Undoable::new(editor, |action| match action {
            UndoableAction::Undo => Message::Undo,
            UndoableAction::Redo => Message::Redo,
            UndoableAction::Find => Message::Find,
        })
        .content_ref(content)
        .selection(selection)
        .version(version)
        .font(iced::Font::MONOSPACE)
        .size(14.0)
        .padding(5.0)
        .into()
    }
}
