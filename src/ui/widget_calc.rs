/// Widget calculation utilities for EditorView
///
/// This module contains pure calculation functions extracted from the EditorView widget
/// to enable unit testing of complex layout and text measurement logic.

/// Calculate the width needed for the line number gutter
///
/// # Arguments
/// * `line_count` - Total number of lines in the editor
/// * `char_width` - Width of a single monospace character
/// * `padding` - Padding to add around the gutter
///
/// # Returns
/// The total width needed for the gutter in pixels
pub fn calculate_gutter_width(line_count: usize, char_width: f32, padding: f32) -> f32 {
    let digits = line_count.to_string().len();
    (digits as f32 * char_width) + char_width + padding
}

/// Calculate the height of a line of text, accounting for word wrapping
///
/// # Arguments
/// * `text` - The text content of the line
/// * `content_width` - Available width for text content
/// * `char_width` - Width of a single monospace character
/// * `single_line_height` - Height of a single unwrapped line
/// * `max_chars` - Maximum characters that fit in one line
///
/// # Returns
/// The total height of the line in pixels (may be multiple visual lines if wrapped)
pub fn calculate_line_height(
    text: &str,
    content_width: f32,
    char_width: f32,
    single_line_height: f32,
    max_chars: usize,
) -> f32 {
    if text.len() <= max_chars {
        // Line fits without wrapping
        single_line_height
    } else {
        // Line may wrap - need to calculate actual height
        // This is a simplified estimation; actual rendering may differ
        let estimated_lines = ((text.len() as f32 * char_width) / content_width).ceil();
        estimated_lines.max(1.0) * single_line_height
    }
}

/// Simulate word-wrap positioning to find the (x, y) offset of a column within wrapped text
///
/// This function simulates how the text editor wraps text at word boundaries to determine
/// the visual position of a character at a given column index.
///
/// # Arguments
/// * `text` - The text content of the line
/// * `column` - The column index (character position) to find
/// * `char_width` - Width of a single monospace character
/// * `content_width` - Available width for text content
/// * `single_line_height` - Height of a single line
///
/// # Returns
/// A tuple of (x_offset, y_offset) representing the visual position
pub fn simulate_word_wrap_position(
    text: &str,
    column: usize,
    char_width: f32,
    content_width: f32,
    single_line_height: f32,
) -> (f32, f32) {
    if column == 0 {
        return (0.0, 0.0);
    }

    let mut current_x = 0.0;
    let mut current_y = 0.0;

    // Character wrapping (Glyph) simulation.
    // We return the position (current_x, current_y) for the character AT 'column' index.
    for _ in text.chars().take(column) {
        current_x += char_width;

        // If the NEXT character position would exceed content_width, wrap.
        // We use a small epsilon to avoid floating point precision issues.
        if current_x + char_width > content_width + 0.01 {
            current_x = 0.0;
            current_y += single_line_height;
        }
    }

    (current_x, current_y)
}

/// Check if a line is visible within the viewport
///
/// # Arguments
/// * `line_y` - Y position of the top of the line
/// * `line_height` - Height of the line
/// * `viewport_start` - Y position of the top of the viewport
/// * `viewport_end` - Y position of the bottom of the viewport
///
/// # Returns
/// `true` if any part of the line is visible in the viewport
pub fn is_line_in_viewport(
    line_y: f32,
    line_height: f32,
    viewport_start: f32,
    viewport_end: f32,
) -> bool {
    let line_bottom = line_y + line_height;
    // Line is visible if it overlaps with viewport
    line_bottom > viewport_start && line_y < viewport_end
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_gutter_width() {
        let char_width = 8.0;
        let padding = 5.0;

        // 1-9 lines: 1 digit
        assert_eq!(calculate_gutter_width(5, char_width, padding), 21.0);
        assert_eq!(calculate_gutter_width(9, char_width, padding), 21.0);

        // 10-99 lines: 2 digits
        assert_eq!(calculate_gutter_width(10, char_width, padding), 29.0);
        assert_eq!(calculate_gutter_width(99, char_width, padding), 29.0);

        // 100-999 lines: 3 digits
        assert_eq!(calculate_gutter_width(100, char_width, padding), 37.0);
        assert_eq!(calculate_gutter_width(999, char_width, padding), 37.0);

        // 1000+ lines: 4 digits
        assert_eq!(calculate_gutter_width(1000, char_width, padding), 45.0);
    }

    #[test]
    fn test_calculate_line_height_no_wrap() {
        let char_width = 8.0f32;
        let content_width = 800.0f32;
        let single_line_height = 20.0f32;
        let max_chars = (content_width / char_width).floor() as usize; // 100 chars

        // Short text that fits in one line
        let text = "Hello, world!";
        let height = calculate_line_height(
            text,
            content_width,
            char_width,
            single_line_height,
            max_chars,
        );
        assert_eq!(height, single_line_height);
    }

    #[test]
    fn test_calculate_line_height_with_wrap() {
        let char_width = 8.0f32;
        let content_width = 800.0f32;
        let single_line_height = 20.0f32;
        let max_chars = (content_width / char_width).floor() as usize; // 100 chars

        // Long text that exceeds max_chars
        let text = "a".repeat(150);
        let height = calculate_line_height(
            &text,
            content_width,
            char_width,
            single_line_height,
            max_chars,
        );
        // Should be at least 2 lines worth of height
        assert!(height >= single_line_height * 2.0);
    }

    #[test]
    fn test_simulate_word_wrap_position_start() {
        let char_width = 8.0;
        let content_width = 800.0;
        let single_line_height = 20.0;

        let text = "Hello world";
        let (x, y) =
            simulate_word_wrap_position(text, 0, char_width, content_width, single_line_height);
        assert_eq!(x, 0.0);
        assert_eq!(y, 0.0);
    }

    #[test]
    fn test_simulate_word_wrap_position_same_line() {
        let char_width = 8.0;
        let content_width = 800.0;
        let single_line_height = 20.0;

        let text = "Hello world";
        // Position at 'w' (column 6)
        let (x, y) =
            simulate_word_wrap_position(text, 6, char_width, content_width, single_line_height);
        assert_eq!(x, 6.0 * char_width);
        assert_eq!(y, 0.0);
    }

    #[test]
    fn test_simulate_word_wrap_position_wrapped() {
        let char_width = 8.0;
        let content_width = 80.0; // Only 10 chars fit
        let single_line_height = 20.0;

        // "Hello world" - "world" should wrap to next line
        let text = "Hello world";
        // Position at 'w' (column 6) - should be on second line
        let (x, y) =
            simulate_word_wrap_position(text, 6, char_width, content_width, single_line_height);
        assert_eq!(x, 0.0); // Start of wrapped line
        assert_eq!(y, single_line_height); // Second line
    }

    #[test]
    fn test_simulate_word_wrap_position_long_text() {
        let char_width = 8.0;
        let content_width = 80.0; // 10 chars fit
        let single_line_height = 20.0;

        let text = "This is a very long line that will definitely wrap multiple times";
        // Find position of 'v' in "very" (around column 10)
        let column = text.find('v').unwrap();
        let (_x, y) = simulate_word_wrap_position(
            text,
            column,
            char_width,
            content_width,
            single_line_height,
        );

        // Should be on a wrapped line (y > 0)
        assert!(y >= single_line_height);
    }

    #[test]
    fn test_is_line_in_viewport_fully_visible() {
        let line_y = 100.0;
        let line_height = 20.0;
        let viewport_start = 50.0;
        let viewport_end = 200.0;

        assert!(is_line_in_viewport(
            line_y,
            line_height,
            viewport_start,
            viewport_end
        ));
    }

    #[test]
    fn test_is_line_in_viewport_partially_visible_top() {
        let line_y = 40.0;
        let line_height = 20.0;
        let viewport_start = 50.0;
        let viewport_end = 200.0;

        // Line bottom (60.0) is below viewport start
        assert!(is_line_in_viewport(
            line_y,
            line_height,
            viewport_start,
            viewport_end
        ));
    }

    #[test]
    fn test_is_line_in_viewport_partially_visible_bottom() {
        let line_y = 190.0;
        let line_height = 20.0;
        let viewport_start = 50.0;
        let viewport_end = 200.0;

        // Line top (190.0) is above viewport end
        assert!(is_line_in_viewport(
            line_y,
            line_height,
            viewport_start,
            viewport_end
        ));
    }

    #[test]
    fn test_is_line_in_viewport_not_visible_above() {
        let line_y = 10.0;
        let line_height = 20.0;
        let viewport_start = 50.0;
        let viewport_end = 200.0;

        // Line bottom (30.0) is above viewport start
        assert!(!is_line_in_viewport(
            line_y,
            line_height,
            viewport_start,
            viewport_end
        ));
    }

    #[test]
    fn test_is_line_in_viewport_not_visible_below() {
        let line_y = 250.0;
        let line_height = 20.0;
        let viewport_start = 50.0;
        let viewport_end = 200.0;

        // Line top (250.0) is below viewport end
        assert!(!is_line_in_viewport(
            line_y,
            line_height,
            viewport_start,
            viewport_end
        ));
    }
}
