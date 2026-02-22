use crate::history::{Command, History, TextEditorCommand};
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
    /// Track cursor and anchor as positions
    cursor: text_editor::Position,
    anchor: text_editor::Position,
}

impl UndoableEditor {
    pub fn new(initial_text: String) -> Self {
        let pos = text_editor::Position { line: 0, column: 0 };
        Self {
            history: History::new(),
            rope: ropey::Rope::from_str(&initial_text),
            height: Length::Fill,
            version: 0,
            cursor: pos,
            anchor: pos,
        }
    }

    pub fn new_empty() -> Self {
        let pos = text_editor::Position { line: 0, column: 0 };
        Self {
            history: History::new(),
            rope: ropey::Rope::new(),
            height: Length::Fill,
            version: 0,
            cursor: pos,
            anchor: pos,
        }
    }

    pub fn height(mut self, height: Length) -> Self {
        self.height = height;
        self
    }

    /// Helper to convert a `text_editor::Position` to a char offset in the `Rope`.
    fn pos_to_char(&self, pos: text_editor::Position) -> usize {
        let n_lines = self.rope.len_lines();
        let line = pos.line.min(n_lines.saturating_sub(1));
        let line_offset = self.rope.line_to_char(line);
        let line_len = self.rope.line(line).len_chars();
        line_offset + pos.column.min(line_len)
    }

    /// Helper to convert a char offset to a `text_editor::Position`.
    fn char_to_pos(&self, offset: usize) -> text_editor::Position {
        let offset = offset.min(self.rope.len_chars());
        let line = self.rope.char_to_line(offset);
        let line_offset = self.rope.line_to_char(line);
        text_editor::Position {
            line,
            column: offset - line_offset,
        }
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
                // Determine what type of action it is
                let is_edit = action.is_edit();

                // Capture state before action
                let cursor_before_struct = content.cursor();
                let cursor_before = cursor_before_struct.position;
                let anchor_before = cursor_before_struct
                    .selection
                    .unwrap_or(cursor_before_struct.position);

                let offset_before = self.pos_to_char(cursor_before);
                let anchor_offset_before = self.pos_to_char(anchor_before);

                content.perform(action.clone());

                // Sync state after action
                let cursor_after_struct = content.cursor();
                self.cursor = cursor_after_struct.position;
                self.anchor = cursor_after_struct
                    .selection
                    .unwrap_or(cursor_after_struct.position);

                if is_edit {
                    if let text_editor::Action::Edit(edit) = action {
                        let mut cmd = None;
                        let offset_after = self.pos_to_char(self.cursor);

                        match edit {
                            text_editor::Edit::Insert(c) => {
                                let text = c.to_string();
                                let at = offset_before.min(anchor_offset_before);

                                if offset_before != anchor_offset_before {
                                    let start = offset_before.min(anchor_offset_before);
                                    let end = offset_before.max(anchor_offset_before);
                                    let old_text = self.rope.slice(start..end).to_string();
                                    cmd = Some(TextEditorCommand::Replace {
                                        at,
                                        old: old_text,
                                        new: text,
                                        cursor_before: offset_before,
                                        cursor_after: offset_after,
                                        timestamp: std::time::Instant::now(),
                                    });
                                } else {
                                    cmd = Some(TextEditorCommand::Insert {
                                        at,
                                        text,
                                        cursor_before: offset_before,
                                        cursor_after: offset_after,
                                        timestamp: std::time::Instant::now(),
                                    });
                                }
                            }
                            text_editor::Edit::Paste(text) => {
                                let text_str = (*text).to_string();
                                let at = offset_before.min(anchor_offset_before);

                                if offset_before != anchor_offset_before {
                                    let start = offset_before.min(anchor_offset_before);
                                    let end = offset_before.max(anchor_offset_before);
                                    let old_text = self.rope.slice(start..end).to_string();
                                    cmd = Some(TextEditorCommand::Replace {
                                        at,
                                        old: old_text,
                                        new: text_str,
                                        cursor_before: offset_before,
                                        cursor_after: offset_after,
                                        timestamp: std::time::Instant::now(),
                                    });
                                } else {
                                    cmd = Some(TextEditorCommand::Insert {
                                        at,
                                        text: text_str,
                                        cursor_before: offset_before,
                                        cursor_after: offset_after,
                                        timestamp: std::time::Instant::now(),
                                    });
                                }
                            }
                            text_editor::Edit::Enter => {
                                let at = offset_before.min(anchor_offset_before);
                                let text = "\n".to_string();

                                if offset_before != anchor_offset_before {
                                    let start = offset_before.min(anchor_offset_before);
                                    let end = offset_before.max(anchor_offset_before);
                                    let old_text = self.rope.slice(start..end).to_string();
                                    cmd = Some(TextEditorCommand::Replace {
                                        at,
                                        old: old_text,
                                        new: text,
                                        cursor_before: offset_before,
                                        cursor_after: offset_after,
                                        timestamp: std::time::Instant::now(),
                                    });
                                } else {
                                    cmd = Some(TextEditorCommand::Insert {
                                        at,
                                        text,
                                        cursor_before: offset_before,
                                        cursor_after: offset_after,
                                        timestamp: std::time::Instant::now(),
                                    });
                                }
                            }
                            text_editor::Edit::Backspace => {
                                if offset_before != anchor_offset_before {
                                    let start = offset_before.min(anchor_offset_before);
                                    let end = offset_before.max(anchor_offset_before);
                                    let old_text = self.rope.slice(start..end).to_string();
                                    cmd = Some(TextEditorCommand::Delete {
                                        at: start,
                                        text: old_text,
                                        cursor_before: offset_before,
                                        cursor_after: offset_after,
                                        timestamp: std::time::Instant::now(),
                                    });
                                } else if offset_before > 0 {
                                    let start = offset_before - 1;
                                    let deleted = self.rope.char(start).to_string();
                                    cmd = Some(TextEditorCommand::Delete {
                                        at: start,
                                        text: deleted,
                                        cursor_before: offset_before,
                                        cursor_after: offset_after,
                                        timestamp: std::time::Instant::now(),
                                    });
                                }
                            }
                            text_editor::Edit::Delete => {
                                if offset_before != anchor_offset_before {
                                    let start = offset_before.min(anchor_offset_before);
                                    let end = offset_before.max(anchor_offset_before);
                                    let old_text = self.rope.slice(start..end).to_string();
                                    cmd = Some(TextEditorCommand::Delete {
                                        at: start,
                                        text: old_text,
                                        cursor_before: offset_before,
                                        cursor_after: offset_after,
                                        timestamp: std::time::Instant::now(),
                                    });
                                } else if offset_before < self.rope.len_chars() {
                                    let deleted = self.rope.char(offset_before).to_string();
                                    cmd = Some(TextEditorCommand::Delete {
                                        at: offset_before,
                                        text: deleted,
                                        cursor_before: offset_before,
                                        cursor_after: offset_after,
                                        timestamp: std::time::Instant::now(),
                                    });
                                }
                            }
                            _ => {
                                let new_text = content.text();
                                self.rope = ropey::Rope::from_str(&new_text);
                                self.version += 1;
                                return Some(new_text);
                            }
                        }

                        if let Some(mut c) = cmd {
                            c.execute(&mut self.rope);
                            self.history.push(c);
                            self.version += 1;
                            return Some(self.rope.to_string());
                        }
                    }
                }
                None
            }
            Message::Undo => {
                if let Some(mut cmd) = self.history.undo_stack.pop_back() {
                    cmd.undo(&mut self.rope);

                    let pos = self.char_to_pos(cmd.cursor_before());
                    let rope_str = self.rope.to_string();
                    content.perform(text_editor::Action::SelectAll);
                    content.perform(text_editor::Action::Edit(text_editor::Edit::Paste(
                        std::sync::Arc::new(rope_str.clone()),
                    )));
                    content.move_to(text_editor::Cursor {
                        position: pos,
                        selection: None,
                    });
                    self.cursor = pos;
                    self.anchor = pos;

                    self.history.redo_stack.push_back(cmd);
                    self.version += 1;
                    Some(rope_str)
                } else {
                    None
                }
            }
            Message::Redo => {
                if let Some(mut cmd) = self.history.redo_stack.pop_back() {
                    cmd.execute(&mut self.rope);

                    let pos = self.char_to_pos(cmd.cursor_after());
                    let rope_str = self.rope.to_string();
                    content.perform(text_editor::Action::SelectAll);
                    content.perform(text_editor::Action::Edit(text_editor::Edit::Paste(
                        std::sync::Arc::new(rope_str.clone()),
                    )));
                    content.move_to(text_editor::Cursor {
                        position: pos,
                        selection: None,
                    });
                    self.cursor = pos;
                    self.anchor = pos;

                    self.history.undo_stack.push_back(cmd);
                    self.version += 1;
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
        _editor_id: impl Into<iced::widget::Id>,
        content: &'a text_editor::Content,
        syntax: Option<&'a str>,
        search_query: Option<&'a str>,
        search_active_match: Option<(text_editor::Position, text_editor::Position)>,
    ) -> Element<'a, Message> {
        let editor_id = _editor_id.into();

        let editor: Element<'a, Message> = if let Some(syntax) = syntax {
            text_editor(content)
                .id(editor_id)
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
                .style(Self::editor_style)
                .into()
        } else {
            text_editor(content)
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
                .style(Self::editor_style)
                .into()
        };

        Self::wrap_in_undoable(
            editor,
            content,
            search_query,
            search_active_match,
            self.version,
        )
    }

    fn editor_style(theme: &Theme, _status: text_editor::Status) -> text_editor::Style {
        text_editor::Style {
            background: iced::Background::Color(Color::TRANSPARENT),
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
