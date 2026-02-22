use ropey::Rope;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Trait defining a generic command that mutates state
pub trait Command<State> {
    fn execute(&mut self, state: &mut State);
    fn undo(&mut self, state: &mut State);
    /// Attempt to merge the `next` command into `self`. Returns true if merged successfully.
    fn try_merge(&mut self, _next: &Self) -> bool {
        false
    }
}

/// Generic history manager parameterized by the Command type and State type it applies to
#[derive(Debug, Clone)]
pub struct History<C> {
    pub undo_stack: VecDeque<C>,
    pub redo_stack: VecDeque<C>,
    max_size: usize,
}

impl<C: Clone> History<C> {
    pub fn new() -> Self {
        Self {
            undo_stack: VecDeque::new(),
            redo_stack: VecDeque::new(),
            max_size: 1000,
        }
    }

    pub fn with_capacity(max_size: usize) -> Self {
        Self {
            undo_stack: VecDeque::new(),
            redo_stack: VecDeque::new(),
            max_size,
        }
    }

    /// Pushes a new command, attempting to merge it with the previous command
    pub fn push<State>(&mut self, mut cmd: C)
    where
        C: Command<State>,
    {
        self.redo_stack.clear();

        if let Some(top) = self.undo_stack.back_mut() {
            if top.try_merge(&cmd) {
                return;
            }
        }

        if self.undo_stack.len() >= self.max_size {
            self.undo_stack.pop_front();
        }
        self.undo_stack.push_back(cmd);
    }

    pub fn undo<State>(&mut self, state: &mut State) -> bool
    where
        C: Command<State>,
    {
        if let Some(mut cmd) = self.undo_stack.pop_back() {
            cmd.undo(state);
            self.redo_stack.push_back(cmd);
            true
        } else {
            false
        }
    }

    pub fn redo<State>(&mut self, state: &mut State) -> bool
    where
        C: Command<State>,
    {
        if let Some(mut cmd) = self.redo_stack.pop_back() {
            cmd.execute(state);
            self.undo_stack.push_back(cmd);
            true
        } else {
            false
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }
}

// -----------------------------------------------------------------------------
// TextInput Implementation
// -----------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum TextInputCommand {
    Insert {
        at: usize,
        text: String,
        timestamp: Instant,
    },
    Delete {
        at: usize,
        text: String,
        timestamp: Instant,
    },
    Replace {
        at: usize,
        old: String,
        new: String,
        timestamp: Instant,
    },
}

impl Command<String> for TextInputCommand {
    fn execute(&mut self, state: &mut String) {
        match self {
            Self::Insert { at, text, .. } => state.insert_str(*at, text),
            Self::Delete { at, text, .. } => {
                state.replace_range(*at..*at + text.len(), "");
            }
            Self::Replace { at, old, new, .. } => {
                state.replace_range(*at..*at + old.len(), new);
            }
        }
    }

    fn undo(&mut self, state: &mut String) {
        match self {
            Self::Insert { at, text, .. } => {
                state.replace_range(*at..*at + text.len(), "");
            }
            Self::Delete { at, text, .. } => state.insert_str(*at, text),
            Self::Replace { at, old, new, .. } => {
                state.replace_range(*at..*at + new.len(), old);
            }
        }
    }

    fn try_merge(&mut self, next: &Self) -> bool {
        let max_delay = Duration::from_millis(300);

        match (self, next) {
            (
                Self::Insert {
                    at: at1,
                    text: text1,
                    timestamp: ts1,
                },
                Self::Insert {
                    at: at2,
                    text: text2,
                    timestamp: ts2,
                },
            ) => {
                // Only merge single-character inserts (not pastes/multi-char).
                // Stop at word boundaries (space, newline, punctuation) like VSCode does.
                let is_single_char = text2.chars().count() == 1;
                let is_delimiter = text2
                    .chars()
                    .next()
                    .map(|c| c.is_whitespace() || c.is_ascii_punctuation())
                    .unwrap_or(false);

                if is_single_char
                    && !is_delimiter
                    && *at2 == *at1 + text1.chars().count()
                    && ts2.duration_since(*ts1) < max_delay
                {
                    text1.push_str(text2);
                    *ts1 = *ts2;
                    return true;
                }
            }
            (
                Self::Delete {
                    at: at1,
                    text: text1,
                    timestamp: ts1,
                },
                Self::Delete {
                    at: at2,
                    text: text2,
                    timestamp: ts2,
                },
            ) => {
                let chars2 = text2.chars().count();
                // Only merge single-char deletes (backspace/delete key), not multi-char deletions
                let is_single_char_del = text2.chars().count() == 1;
                // Stop merging at whitespace/delimiter boundaries
                let is_delimiter = text2
                    .chars()
                    .next()
                    .map(|c| c.is_whitespace() || c.is_ascii_punctuation())
                    .unwrap_or(false);

                if is_single_char_del && !is_delimiter && ts2.duration_since(*ts1) < max_delay {
                    // Merge backward deletes (backspace) within short interval
                    if *at2 + chars2 == *at1 {
                        let mut new_text = String::with_capacity(text1.len() + text2.len());
                        new_text.push_str(text2);
                        new_text.push_str(text1);
                        *at1 = *at2;
                        *text1 = new_text;
                        *ts1 = *ts2;
                        return true;
                    }
                    // Also merge forward deletes (delete key)
                    if *at1 == *at2 {
                        text1.push_str(text2);
                        *ts1 = *ts2;
                        return true;
                    }
                }
            }
            _ => {}
        }
        false
    }
}

/// Compute differences between old and new state for single line inputs
pub fn diff_to_command(old: &str, new: &str) -> Option<TextInputCommand> {
    if old == new {
        return None;
    }

    // Work with character boundaries
    let old_chars: Vec<char> = old.chars().collect();
    let new_chars: Vec<char> = new.chars().collect();

    let mut prefix_len = 0;
    while prefix_len < old_chars.len()
        && prefix_len < new_chars.len()
        && old_chars[prefix_len] == new_chars[prefix_len]
    {
        prefix_len += 1;
    }

    let mut suffix_len = 0;
    let max_suffix = old_chars.len().min(new_chars.len()) - prefix_len;
    while suffix_len < max_suffix
        && old_chars[old_chars.len() - 1 - suffix_len]
            == new_chars[new_chars.len() - 1 - suffix_len]
    {
        suffix_len += 1;
    }

    let deleted_chars = &old_chars[prefix_len..old_chars.len() - suffix_len];
    let inserted_chars = &new_chars[prefix_len..new_chars.len() - suffix_len];

    let deleted: String = deleted_chars.iter().collect();
    let inserted: String = inserted_chars.iter().collect();

    // Map char prefix len back to byte offset for insertion
    let byte_prefix = old.chars().take(prefix_len).map(|c| c.len_utf8()).sum();

    match (deleted.is_empty(), inserted.is_empty()) {
        (true, false) => Some(TextInputCommand::Insert {
            at: byte_prefix,
            text: inserted,
            timestamp: Instant::now(),
        }),
        (false, true) => Some(TextInputCommand::Delete {
            at: byte_prefix,
            text: deleted,
            timestamp: Instant::now(),
        }),
        (false, false) => Some(TextInputCommand::Replace {
            at: byte_prefix,
            old: deleted,
            new: inserted,
            timestamp: Instant::now(),
        }),
        _ => None,
    }
}

// -----------------------------------------------------------------------------
// TextEditor Implementation
// -----------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum TextEditorCommand {
    Insert {
        at: usize, // char offset
        text: String,
        cursor_before: usize,
        cursor_after: usize,
        timestamp: Instant,
    },
    Delete {
        at: usize, // char offset
        text: String,
        cursor_before: usize,
        cursor_after: usize,
        timestamp: Instant,
    },
    Replace {
        at: usize, // char offset
        old: String,
        new: String,
        cursor_before: usize,
        cursor_after: usize,
        timestamp: Instant,
    },
    IndentLines {
        lines: Vec<usize>,
        added: String,
        cursor_before: usize,
        cursor_after: usize,
        timestamp: Instant,
    },
}

impl TextEditorCommand {
    pub fn cursor_before(&self) -> usize {
        match self {
            Self::Insert { cursor_before, .. } => *cursor_before,
            Self::Delete { cursor_before, .. } => *cursor_before,
            Self::Replace { cursor_before, .. } => *cursor_before,
            Self::IndentLines { cursor_before, .. } => *cursor_before,
        }
    }

    pub fn cursor_after(&self) -> usize {
        match self {
            Self::Insert { cursor_after, .. } => *cursor_after,
            Self::Delete { cursor_after, .. } => *cursor_after,
            Self::Replace { cursor_after, .. } => *cursor_after,
            Self::IndentLines { cursor_after, .. } => *cursor_after,
        }
    }
}

impl Command<Rope> for TextEditorCommand {
    fn execute(&mut self, rope: &mut Rope) {
        match self {
            Self::Insert { at, text, .. } => {
                rope.insert(*at, text);
            }
            Self::Delete { at, text, .. } => {
                let chars_len = text.chars().count();
                rope.remove(*at..(*at + chars_len));
            }
            Self::Replace { at, old, new, .. } => {
                let old_chars_len = old.chars().count();
                rope.remove(*at..(*at + old_chars_len));
                rope.insert(*at, new);
            }
            Self::IndentLines { lines, added, .. } => {
                // Loop reverse so early insertions don't invalidate later line char offsets
                for &line_idx in lines.iter().rev() {
                    let char_idx = rope.line_to_char(line_idx);
                    rope.insert(char_idx, added);
                }
            }
        }
    }

    fn undo(&mut self, rope: &mut Rope) {
        match self {
            Self::Insert { at, text, .. } => {
                let chars_len = text.chars().count();
                rope.remove(*at..(*at + chars_len));
            }
            Self::Delete { at, text, .. } => {
                rope.insert(*at, text);
            }
            Self::Replace { at, old, new, .. } => {
                let new_chars_len = new.chars().count();
                rope.remove(*at..(*at + new_chars_len));
                rope.insert(*at, old);
            }
            Self::IndentLines { lines, added, .. } => {
                let added_len = added.chars().count();
                // Loop reverse so early deletions don't invalidate later line char offsets
                for &line_idx in lines.iter().rev() {
                    let char_idx = rope.line_to_char(line_idx);
                    rope.remove(char_idx..(char_idx + added_len));
                }
            }
        }
    }

    fn try_merge(&mut self, next: &Self) -> bool {
        let max_delay = Duration::from_millis(300);

        match (self, next) {
            (
                Self::Insert {
                    at: at1,
                    text: text1,
                    cursor_after: ca1,
                    timestamp: ts1,
                    ..
                },
                Self::Insert {
                    at: at2,
                    text: text2,
                    cursor_after: ca2,
                    timestamp: ts2,
                    ..
                },
            ) => {
                if text1.contains('\n') || text2.contains('\n') {
                    return false;
                }
                if *at2 == *at1 + text1.chars().count() && ts2.duration_since(*ts1) < max_delay {
                    text1.push_str(text2);
                    *ca1 = *ca2;
                    *ts1 = *ts2;
                    return true;
                }
            }
            (
                Self::Delete {
                    at: at1,
                    text: text1,
                    cursor_after: ca1,
                    timestamp: ts1,
                    ..
                },
                Self::Delete {
                    at: at2,
                    text: text2,
                    cursor_after: ca2,
                    timestamp: ts2,
                    ..
                },
            ) => {
                if text1.contains('\n') || text2.contains('\n') {
                    return false;
                }
                let chars2 = text2.chars().count();
                if *at2 + chars2 == *at1 && ts2.duration_since(*ts1) < max_delay {
                    let mut new_text = String::with_capacity(text1.len() + text2.len());
                    new_text.push_str(text2);
                    new_text.push_str(text1);
                    *at1 = *at2;
                    *text1 = new_text;
                    *ca1 = *ca2;
                    *ts1 = *ts2;
                    return true;
                }
                if *at1 == *at2 && ts2.duration_since(*ts1) < max_delay {
                    text1.push_str(text2);
                    *ca1 = *ca2;
                    *ts1 = *ts2;
                    return true;
                }
            }
            _ => {}
        }
        false
    }
}
