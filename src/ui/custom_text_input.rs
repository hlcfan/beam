use iced::widget::{text_input, text, row, container, stack, mouse_area, Space};
use iced::{Element, Length, Background, Border, Color, Theme, Alignment, Shadow, Vector, Point};
use std::time::{Duration, Instant};
use regex::Regex;

/// Wrapper for CustomTextInput with an input callback
pub struct CustomTextInputWithCallback<Message, F>
where
    F: Fn(String) -> Message,
{
    input: CustomTextInput,
    on_input: F,
}

impl<Message, F> CustomTextInputWithCallback<Message, F>
where
    F: Fn(String) -> Message,
    Message: Clone + 'static,
{
    pub fn size(mut self, size: f32) -> Self {
        self.input.size = size;
        self
    }

    pub fn view<'a>(&'a self) -> Element<'a, Message, Theme>
    where
        F: 'a,
    {
        // Create a simple text input with callback for now
        // We'll overlay syntax highlighting later
        text_input(&self.input.placeholder, &self.input.value)
            .on_input(&self.on_input)
            .width(self.input.width)
            .padding(self.input.padding)
            .size(self.input.size)
            .into()
    }
}

/// History entry for undo/redo functionality
#[derive(Debug, Clone)]
pub struct HistoryEntry {
    pub value: String,
    pub cursor_position: usize,
    pub selection: Option<(usize, usize)>,
    pub timestamp: Instant,
}

impl HistoryEntry {
    pub fn new(value: String, cursor_position: usize, selection: Option<(usize, usize)>) -> Self {
        Self {
            value,
            cursor_position,
            selection,
            timestamp: Instant::now(),
        }
    }
}

/// Represents a segment of text with specific styling
#[derive(Debug, Clone)]
pub struct TextSegment {
    pub text: String,
    pub segment_type: SegmentType,
    pub start_index: usize,
    pub end_index: usize,
}

/// Types of text segments for syntax highlighting
#[derive(Debug, Clone, PartialEq)]
pub enum SegmentType {
    Normal,
    Variable,
    // Future: could add more types like Function, String, etc.
}

/// Configuration for syntax highlighting colors
#[derive(Debug, Clone)]
pub struct SyntaxHighlighting {
    pub enabled: bool,
    pub normal_color: Color,
    pub variable_color: Color,
    pub cursor_color: Color,
    pub variable_pattern: String, // Regex pattern for variables
    pub tooltip_enabled: bool,
    pub tooltip_background_color: Color,
    pub tooltip_text_color: Color,
    pub tooltip_border_color: Color,
}

/// A custom single-line text input widget that provides enhanced functionality
#[derive(Debug, Clone)]
pub struct CustomTextInput {
    value: String,
    placeholder: String,
    is_focused: bool,
    cursor_position: usize,
    selection: Option<(usize, usize)>,
    is_secure: bool,
    width: Length,
    padding: f32,
    size: f32,
    // Cursor blinking
    cursor_blink_visible: bool,
    last_blink_toggle: Instant,
    // Undo/Redo history
    history: Vec<HistoryEntry>,
    history_index: usize,
    max_history: usize,
    grouping_threshold_ms: u64, // Time threshold for grouping changes (500ms)
    // Syntax highlighting configuration
    syntax_highlighting: SyntaxHighlighting,
    
    // Tooltip state
    tooltip_visible: bool,
    tooltip_text: String,
    tooltip_position: (f32, f32),
}

impl Default for CustomTextInput {
    fn default() -> Self {
        let initial_entry = HistoryEntry::new(String::new(), 0, None);
        Self {
            value: String::new(),
            placeholder: String::new(),
            is_focused: false,
            cursor_position: 0,
            selection: None,
            is_secure: false,
            width: Length::Fill,
            padding: 8.0,
            size: 14.0,
            cursor_blink_visible: true,
            last_blink_toggle: Instant::now(),
            history: vec![initial_entry],
            history_index: 0,
            max_history: 100,
            grouping_threshold_ms: 500,
            syntax_highlighting: SyntaxHighlighting {
                enabled: true,
                normal_color: Color::BLACK,
                variable_color: Color::from_rgb(0.2, 0.6, 0.9), // Blue color for variables
                cursor_color: Color::BLACK, // Default cursor color
                variable_pattern: r"\{\{[^}]+\}\}".to_string(), // Pattern for {{variable}}
                tooltip_enabled: true,
                tooltip_background_color: Color::from_rgba(0.0, 0.0, 0.0, 0.9),
                tooltip_text_color: Color::WHITE,
                tooltip_border_color: Color::from_rgb(0.6, 0.6, 0.6),
            },
            
            // Initialize tooltip state
            tooltip_visible: false,
            tooltip_text: String::new(),
            tooltip_position: (0.0, 0.0),
        }
    }
}

impl CustomTextInput {
    /// Creates a new CustomTextInput
    pub fn new(placeholder: String) -> Self {
        Self {
            placeholder,
            ..Default::default()
        }
    }

    /// Sets the value of the input
    pub fn with_value(mut self, value: String) -> Self {
        self.value = value;
        self
    }

    /// Sets the width of the input
    pub fn width(mut self, width: Length) -> Self {
        self.width = width;
        self
    }

    /// Sets the padding of the input
    pub fn padding(mut self, padding: f32) -> Self {
        self.padding = padding;
        self
    }

    /// Sets the text size
    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    /// Makes this a password input (shows dots instead of characters)
    pub fn password(mut self) -> Self {
        self.is_secure = true;
        self
    }

    /// Sets the value and adds to history
    pub fn set_value(&mut self, value: String) {
        if value != self.value {
            self.push_to_history(value.clone());
        }
        self.value = value;
        // Adjust cursor position if it's beyond the new text length
        let char_count = self.value.chars().count();
        if self.cursor_position > char_count {
            self.cursor_position = char_count;
        }
    }

    /// Sets the value without adding to history (for undo/redo operations)
    pub fn set_value_without_history(&mut self, value: String) {
        self.value = value;
        // Adjust cursor position if it's beyond the new text length
        let char_count = self.value.chars().count();
        if self.cursor_position > char_count {
            self.cursor_position = char_count;
        }
    }

    /// Pushes a new entry to the history
    pub fn push_to_history(&mut self, value: String) {
        let now = Instant::now();
        
        // Check if we should group this change with the previous one
        if let Some(last_entry) = self.history.get(self.history_index) {
            let time_diff = now.duration_since(last_entry.timestamp).as_millis() as u64;
            if time_diff < self.grouping_threshold_ms && self.history_index == self.history.len() - 1 {
                // Update the current entry instead of creating a new one
                if let Some(entry) = self.history.get_mut(self.history_index) {
                    entry.value = value;
                    entry.cursor_position = self.cursor_position;
                    entry.selection = self.selection;
                    entry.timestamp = now;
                }
                return;
            }
        }

        // Remove any future history if we're not at the end
        if self.history_index < self.history.len() - 1 {
            self.history.truncate(self.history_index + 1);
        }

        // Add new entry
        let entry = HistoryEntry::new(value, self.cursor_position, self.selection);
        self.history.push(entry);
        self.history_index = self.history.len() - 1;

        // Limit history size
        if self.history.len() > self.max_history {
            self.history.remove(0);
            self.history_index = self.history.len() - 1;
        }
    }

    /// Gets the current value
    pub fn value(&self) -> &str {
        &self.value
    }

    /// Focuses the input
    pub fn focus(&mut self) {
        self.is_focused = true;
        self.cursor_blink_visible = true;
        self.last_blink_toggle = Instant::now();
    }

    /// Unfocuses the input
    pub fn unfocus(&mut self) {
        self.is_focused = false;
        self.selection = None;
    }

    /// Moves cursor to the specified position
    pub fn move_cursor_to(&mut self, position: usize) {
        let char_count = self.value.chars().count();
        self.cursor_position = position.min(char_count);
        self.selection = None;
        self.cursor_blink_visible = true;
        self.last_blink_toggle = Instant::now();
    }

    /// Moves cursor to the beginning
    pub fn move_cursor_to_front(&mut self) {
        self.move_cursor_to(0);
    }

    /// Moves cursor to the end
    pub fn move_cursor_to_end(&mut self) {
        let char_count = self.value.chars().count();
        self.move_cursor_to(char_count);
    }

    /// Selects all text
    pub fn select_all(&mut self) {
        let char_count = self.value.chars().count();
        if char_count > 0 {
            self.selection = Some((0, char_count));
            self.cursor_position = char_count;
        }
    }

    /// Updates cursor blink state
    pub fn update_cursor_blink(&mut self) {
        if self.is_focused {
            let now = Instant::now();
            if now.duration_since(self.last_blink_toggle) >= Duration::from_millis(500) {
                self.cursor_blink_visible = !self.cursor_blink_visible;
                self.last_blink_toggle = now;
            }
        }
    }

    /// Performs undo operation
    pub fn undo(&mut self) -> bool {
        if self.history_index > 0 {
            self.history_index -= 1;
            if let Some(entry) = self.history.get(self.history_index).cloned() {
                self.set_value_without_history(entry.value);
                self.cursor_position = entry.cursor_position;
                self.selection = entry.selection;
                self.cursor_blink_visible = true;
                self.last_blink_toggle = Instant::now();
                return true;
            }
        }
        false
    }

    /// Performs redo operation
    pub fn redo(&mut self) -> bool {
        if self.history_index < self.history.len() - 1 {
            self.history_index += 1;
            if let Some(entry) = self.history.get(self.history_index).cloned() {
                self.set_value_without_history(entry.value);
                self.cursor_position = entry.cursor_position;
                self.selection = entry.selection;
                self.cursor_blink_visible = true;
                self.last_blink_toggle = Instant::now();
                return true;
            }
        }
        false
    }

    /// Returns true if undo is available
    pub fn can_undo(&self) -> bool {
        self.history_index > 0
    }

    /// Returns true if redo is available
    pub fn can_redo(&self) -> bool {
        self.history_index < self.history.len() - 1
    }

    /// Inserts text at the current cursor position
    pub fn insert_text(&mut self, text: &str) -> String {
        let mut chars: Vec<char> = self.value.chars().collect();
        
        // If there's a selection, delete it first
        if let Some((start, end)) = self.selection {
            let (actual_start, actual_end) = if start <= end { (start, end) } else { (end, start) };
            chars.drain(actual_start..actual_end);
            self.cursor_position = actual_start;
            self.selection = None;
        }

        // Insert the new text
        for (i, ch) in text.chars().enumerate() {
            chars.insert(self.cursor_position + i, ch);
        }
        
        self.cursor_position += text.chars().count();
        let new_value: String = chars.into_iter().collect();
        
        // Add to history
        self.push_to_history(new_value.clone());
        self.value = new_value.clone();
        new_value
    }

    /// Deletes the character before the cursor (backspace)
    pub fn delete_previous(&mut self) -> String {
        let old_value = self.value.clone();
        
        if let Some((start, end)) = self.selection {
            // Delete selection
            let (actual_start, actual_end) = if start <= end { (start, end) } else { (end, start) };
            let mut chars: Vec<char> = self.value.chars().collect();
            chars.drain(actual_start..actual_end);
            self.value = chars.into_iter().collect();
            self.cursor_position = actual_start;
            self.selection = None;
        } else if self.cursor_position > 0 {
            // Delete character before cursor
            let mut chars: Vec<char> = self.value.chars().collect();
            chars.remove(self.cursor_position - 1);
            self.value = chars.into_iter().collect();
            self.cursor_position -= 1;
        }
        
        // Add to history if value changed
        if self.value != old_value {
            self.push_to_history(self.value.clone());
        }
        
        self.value.clone()
    }

    /// Deletes the character after the cursor (delete)
    pub fn delete_next(&mut self) -> String {
        let old_value = self.value.clone();
        
        if let Some((start, end)) = self.selection {
            // Delete selection
            let (actual_start, actual_end) = if start <= end { (start, end) } else { (end, start) };
            let mut chars: Vec<char> = self.value.chars().collect();
            chars.drain(actual_start..actual_end);
            self.value = chars.into_iter().collect();
            self.cursor_position = actual_start;
            self.selection = None;
        } else {
            let char_count = self.value.chars().count();
            if self.cursor_position < char_count {
                let mut chars: Vec<char> = self.value.chars().collect();
                chars.remove(self.cursor_position);
                self.value = chars.into_iter().collect();
            }
        }
        
        // Add to history if value changed
        if self.value != old_value {
            self.push_to_history(self.value.clone());
        }
        
        self.value.clone()
    }

    /// Moves cursor left
    pub fn move_cursor_left(&mut self) {
        if self.cursor_position > 0 {
            self.cursor_position -= 1;
            self.selection = None;
            self.cursor_blink_visible = true;
            self.last_blink_toggle = Instant::now();
        }
    }

    /// Moves cursor right
    pub fn move_cursor_right(&mut self) {
        let char_count = self.value.chars().count();
        if self.cursor_position < char_count {
            self.cursor_position += 1;
            self.selection = None;
            self.cursor_blink_visible = true;
            self.last_blink_toggle = Instant::now();
        }
    }

    /// Enables or disables syntax highlighting
    pub fn with_syntax_highlighting(mut self, enabled: bool) -> Self {
        self.syntax_highlighting.enabled = enabled;
        self
    }

    /// Sets the color for normal text
    pub fn with_normal_color(mut self, color: Color) -> Self {
        self.syntax_highlighting.normal_color = color;
        self
    }

    /// Sets the color for variables
    pub fn with_variable_color(mut self, color: Color) -> Self {
        self.syntax_highlighting.variable_color = color;
        self
    }

    /// Sets the regex pattern for detecting variables
    pub fn with_variable_pattern(mut self, pattern: String) -> Self {
        self.syntax_highlighting.variable_pattern = pattern;
        self
    }

    /// Sets the cursor color
    pub fn with_cursor_color(mut self, color: Color) -> Self {
        self.syntax_highlighting.cursor_color = color;
        self
    }

    /// Enables or disables tooltips for variables
    pub fn with_tooltip_enabled(mut self, enabled: bool) -> Self {
        self.syntax_highlighting.tooltip_enabled = enabled;
        self
    }

    /// Sets the tooltip background color
    pub fn with_tooltip_background_color(mut self, color: Color) -> Self {
        self.syntax_highlighting.tooltip_background_color = color;
        self
    }

    /// Sets the tooltip text color
    pub fn with_tooltip_text_color(mut self, color: Color) -> Self {
        self.syntax_highlighting.tooltip_text_color = color;
        self
    }

    /// Sets the tooltip border color
    pub fn with_tooltip_border_color(mut self, color: Color) -> Self {
        self.syntax_highlighting.tooltip_border_color = color;
        self
    }

    /// Shows a tooltip with the given text at the specified position
    pub fn show_tooltip(&mut self, text: String, position: (f32, f32)) {
        self.tooltip_visible = true;
        self.tooltip_text = text;
        self.tooltip_position = position;
    }

    /// Hides the tooltip
    pub fn hide_tooltip(&mut self) {
        self.tooltip_visible = false;
        self.tooltip_text.clear();
    }

    /// Checks if a point is over a variable segment and returns the variable text
    pub fn get_variable_at_position(&self, _point: Point) -> Option<String> {
        // For now, this is a simplified implementation
        // In a real implementation, you would need to calculate text layout
        // and determine which segment the point is over
        if !self.syntax_highlighting.enabled || !self.syntax_highlighting.tooltip_enabled {
            return None;
        }

        let segments = self.parse_text_segments();
        for segment in segments {
            if segment.segment_type == SegmentType::Variable {
                // For demo purposes, return the variable text
                // In a real implementation, you'd check if the point is within the segment bounds
                return Some(segment.text);
            }
        }
        None
    }

    /// Add an input callback to create a functional widget
    pub fn on_input<F, Message>(self, f: F) -> CustomTextInputWithCallback<Message, F>
    where
        F: Fn(String) -> Message,
    {
        CustomTextInputWithCallback {
            input: self,
            on_input: f,
        }
    }

    /// Parses the text into segments for syntax highlighting
    fn parse_text_segments(&self) -> Vec<TextSegment> {
        if !self.syntax_highlighting.enabled {
            return vec![TextSegment {
                text: self.value.clone(),
                segment_type: SegmentType::Normal,
                start_index: 0,
                end_index: self.value.len(),
            }];
        }

        let mut segments = Vec::new();
        
        // Use regex to find variable patterns
        if let Ok(regex) = Regex::new(&self.syntax_highlighting.variable_pattern) {
            let mut last_end = 0;
            
            for mat in regex.find_iter(&self.value) {
                // Add normal text before the variable
                if mat.start() > last_end {
                    segments.push(TextSegment {
                        text: self.value[last_end..mat.start()].to_string(),
                        segment_type: SegmentType::Normal,
                        start_index: last_end,
                        end_index: mat.start(),
                    });
                }
                
                // Add the variable segment
                segments.push(TextSegment {
                    text: mat.as_str().to_string(),
                    segment_type: SegmentType::Variable,
                    start_index: mat.start(),
                    end_index: mat.end(),
                });
                
                last_end = mat.end();
            }
            
            // Add remaining normal text after the last variable
            if last_end < self.value.len() {
                segments.push(TextSegment {
                    text: self.value[last_end..].to_string(),
                    segment_type: SegmentType::Normal,
                    start_index: last_end,
                    end_index: self.value.len(),
                });
            }
        } else {
            // If regex is invalid, treat all text as normal
            segments.push(TextSegment {
                text: self.value.clone(),
                segment_type: SegmentType::Normal,
                start_index: 0,
                end_index: self.value.len(),
            });
        }

        // If no segments were created (empty text), add an empty normal segment
        if segments.is_empty() {
            segments.push(TextSegment {
                text: String::new(),
                segment_type: SegmentType::Normal,
                start_index: 0,
                end_index: 0,
            });
        }

        segments
    }

    /// Creates a view of the custom text input with syntax highlighting and tooltip support
    pub fn view_with_tooltip<'a, Message, F>(&'a self, on_hover: F, on_exit: Message) -> Element<'a, Message, Theme>
    where
        Message: Clone + 'a,
        F: Fn(Point) -> Message + 'a,
    {
        let base_view = self.view();
        
        if !self.syntax_highlighting.tooltip_enabled {
            return base_view;
        }

        // Create a mouse area overlay for hover detection
        let overlay = mouse_area(
            Space::new()
                .width(Length::Fill)
                .height(Length::Fill)
        )
        .on_move(on_hover)
        .on_exit(on_exit);

        // Stack the base view with the mouse area overlay
        let content = stack![base_view, overlay];

        // Add tooltip if visible
        if self.tooltip_visible && !self.tooltip_text.is_empty() {
            stack![
                content,
                // Tooltip overlay
                container(
                    container(
                        text(&self.tooltip_text)
                            .size(12)
                            .color(self.syntax_highlighting.tooltip_text_color)
                    )
                    .padding(8)
                    .style(move |_theme| container::Style {
                        background: Some(Background::Color(self.syntax_highlighting.tooltip_background_color)),
                        border: Border {
                            color: self.syntax_highlighting.tooltip_border_color,
                            width: 1.0,
                            radius: 4.0.into(),
                        },
                        shadow: Shadow {
                            color: Color::from_rgba(0.0, 0.0, 0.0, 0.3),
                            offset: Vector::new(2.0, 2.0),
                            blur_radius: 4.0,
                        },
                        ..Default::default()
                    })
                )
                .width(Length::Fill)
                .height(Length::Fill)
                .padding(iced::Padding::new(self.tooltip_position.0).top(self.tooltip_position.1))
            ]
            .into()
        } else {
            content.into()
        }
    }

    /// Creates a view of the custom text input with syntax highlighting
    pub fn view<'a, Message>(&'a self) -> Element<'a, Message, Theme>
    where
        Message: Clone + 'a,
    {
        // For now, return a simple container with syntax-highlighted text
        // This is a display-only version until we can properly integrate with text_input
        
        if self.value.is_empty() {
            // Show placeholder
            container(
                text(&self.placeholder)
                    .size(self.size)
                    .color(Color::from_rgb(0.6, 0.6, 0.6))
            )
            .width(self.width)
            .padding(self.padding)
            .style(|theme: &Theme| {
                container::Style {
                    background: Some(Background::Color(theme.palette().background)),
                    border: Border {
                        color: Color::from_rgb(0.7, 0.7, 0.7),
                        width: 1.0,
                        radius: 4.0.into(),
                    },
                    ..Default::default()
                }
            })
            .into()
        } else {
            // Parse text segments for syntax highlighting
            let segments = self.parse_text_segments();
            
            // Create colored text elements
            let text_elements: Vec<Element<'a, Message, Theme>> = segments
                .into_iter()
                .map(|segment| {
                    let color = match segment.segment_type {
                        SegmentType::Variable => self.syntax_highlighting.variable_color,
                        SegmentType::Normal => self.syntax_highlighting.normal_color,
                    };
                    text(segment.text)
                        .size(self.size)
                        .color(color)
                        .into()
                })
                .collect();

            container(
                row(text_elements)
                    .align_y(Alignment::Center)
            )
            .width(self.width)
            .padding(self.padding)
            .style(|theme: &Theme| {
                container::Style {
                    background: Some(Background::Color(theme.palette().background)),
                    border: Border {
                        color: Color::from_rgb(0.7, 0.7, 0.7),
                        width: 1.0,
                        radius: 4.0.into(),
                    },
                    ..Default::default()
                }
            })
            .into()
        }
    }
}