use crate::history::{History, TextEditorCommand};
use crate::ui::editor_view::{Action as UndoableAction, EditorView};
use iced::advanced::text;
use iced::widget::text_editor;
use iced::{Color, Element, Length, Theme};

#[derive(Debug, Clone)]
pub enum Message {
    Action(text_editor::Action),
    Undo,
    Redo,
    Find,
    ScrollToMatch(f32),
}

#[derive(Debug, Clone)]
pub struct UndoableEditor {
    history: History<TextEditorCommand>,
    rope: ropey::Rope,
    height: Length,
    version: usize,
}

impl UndoableEditor {
    pub fn new(initial_text: String) -> Self {
        Self {
            history: History::new(),
            rope: ropey::Rope::from_str(&initial_text),
            height: Length::Fill,
            version: 0,
        }
    }

    pub fn new_empty() -> Self {
        Self {
            history: History::new(),
            rope: ropey::Rope::new(),
            height: Length::Fill,
            version: 0,
        }
    }

    pub fn height(mut self, height: Length) -> Self {
        self.height = height;
        self
    }

    /// Helper to convert a byte offset in `text_editor::Content` to a char offset in `ropey::Rope`.
    fn map_char_offset(content: &text_editor::Content, line: usize, column: usize) -> usize {
        let mut char_offset = 0;
        for (i, l) in content.lines().enumerate() {
            if i == line {
                // column in iced is byte offset, we need char offset.
                // We'll iterate chars up to column boundary.
                let mut current_byte = 0;
                for c in l.text.chars() {
                    if current_byte == column {
                        break;
                    }
                    current_byte += c.len_utf8();
                    char_offset += 1;
                }
                break;
            }
            char_offset += l.text.chars().count() + 1; // +1 for newline
        }
        char_offset
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
                // Unforunately `Content` doesn't expose `cursor_position()` directly.
                // But we know what the edit is doing. But to know WHERE to do it on the Rope
                // we have to diff it vs the previous state or look at the rope state.
                // ACTUALLY `action` is passed to `Content::perform(action)` first, THEN we
                // can just rebuild the rope if text changed.
                // Or if we rebuild rope, we just track full texts for undo? No, we need
                // granular diffs.
                // Let's implement diffing for TextEditor command here too just like we did
                // for TextInput, to infer the edit point.
                let old_text = content.text();
                content.perform(action);
                let new_text = content.text();

                if old_text != new_text {
                    if let Some(cmd) =
                        crate::history::diff_to_command(&old_text, &new_text).map(|c| match c {
                            crate::history::TextInputCommand::Insert {
                                at,
                                text,
                                timestamp,
                            } => TextEditorCommand::Insert {
                                at,
                                text,
                                timestamp,
                            },
                            crate::history::TextInputCommand::Delete {
                                at,
                                text,
                                timestamp,
                            } => TextEditorCommand::Delete {
                                at,
                                text,
                                timestamp,
                            },
                            crate::history::TextInputCommand::Replace {
                                at,
                                old,
                                new,
                                timestamp,
                            } => TextEditorCommand::Replace {
                                at,
                                old,
                                new,
                                timestamp,
                            },
                        })
                    {
                        self.history.push(cmd);
                    }
                    self.rope = ropey::Rope::from_str(&new_text);
                    self.version += 1;
                    Some(self.rope.to_string())
                } else {
                    None
                }
            }
            Message::Undo => {
                if self.history.undo(&mut self.rope) {
                    let rope_str = self.rope.to_string();
                    content.perform(text_editor::Action::SelectAll);
                    content.perform(text_editor::Action::Edit(text_editor::Edit::Paste(
                        std::sync::Arc::new(rope_str.clone()),
                    )));
                    Some(rope_str)
                } else {
                    None
                }
            }
            Message::Redo => {
                if self.history.redo(&mut self.rope) {
                    let rope_str = self.rope.to_string();
                    content.perform(text_editor::Action::SelectAll);
                    content.perform(text_editor::Action::Edit(text_editor::Edit::Paste(
                        std::sync::Arc::new(rope_str.clone()),
                    )));
                    Some(rope_str)
                } else {
                    None
                }
            }
            Message::Find => None,
            Message::ScrollToMatch(_) => None,
        }
    }

    pub fn view<'a>(
        &'a self,
        editor_id: impl Into<iced::widget::Id>,
        content: &'a text_editor::Content,
        syntax: Option<&'a str>,
        search_query: Option<&'a str>,
        search_active_match: Option<(text_editor::Position, text_editor::Position)>,
    ) -> Element<'a, Message> {
        let editor_id = editor_id.into();

        // Create editor with or without syntax highlighting
        if let Some(syntax) = syntax {
            let editor = text_editor(content)
                .id(editor_id.clone())
                .on_action(Message::Action)
                .highlight(syntax, iced::highlighter::Theme::SolarizedDark)
                .font(iced::Font::MONOSPACE)
                .size(14)
                .padding(iced::Padding {
                    top: 5.0,
                    right: 20.0,
                    bottom: 5.0,
                    left: 5.0,
                })
                .wrapping(text::Wrapping::Glyph)
                // .height(self.height)
                .style(Self::editor_style);

            Self::wrap_in_undoable(
                editor,
                content,
                search_query,
                search_active_match,
                self.version,
            )
        } else {
            let editor = text_editor(content)
                .id(editor_id)
                .on_action(Message::Action)
                .font(iced::Font::MONOSPACE)
                .size(14)
                .padding(iced::Padding {
                    top: 5.0,
                    right: 20.0,
                    bottom: 5.0,
                    left: 5.0,
                })
                .wrapping(text::Wrapping::Glyph)
                // .height(self.height)
                .style(Self::editor_style);

            Self::wrap_in_undoable(
                editor,
                content,
                search_query,
                search_active_match,
                self.version,
            )
        }
    }

    fn editor_style(theme: &Theme, _status: text_editor::Status) -> text_editor::Style {
        text_editor::Style {
            background: iced::Background::Color(Color::TRANSPARENT), // Use transparent so we can draw custom highlights underneath
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
        search_query: Option<&'a str>,
        search_active_match: Option<(text_editor::Position, text_editor::Position)>,
        version: usize,
    ) -> Element<'a, Message> {
        let mut view = EditorView::new(editor, |action| match action {
            UndoableAction::Undo => Message::Undo,
            UndoableAction::Redo => Message::Redo,
            UndoableAction::Find => Message::Find,
            UndoableAction::ScrollToMatch(y) => Message::ScrollToMatch(y),
        })
        .content_ref(content)
        .search_active_match(search_active_match)
        .version(version)
        .font(iced::Font::MONOSPACE)
        .size(14.0)
        .padding(5.0)
        .padding_right(20.0);

        if let Some(query) = search_query {
            view = view.search_query(query.to_string());
        }

        view.into()
    }
}
