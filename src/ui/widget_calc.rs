/// Widget calculation utilities for EditorView
///
/// This module contains pure calculation functions extracted from the EditorView widget
/// to enable unit testing of complex layout and text measurement logic.

/// One visual (screen) row produced by glyph-wrapping a logical line.
///
/// Both the gutter and the text content area must use this shared structure
/// as the single source of truth for Y positions — neither widget may compute
/// Y independently (e.g. `y = line_index * line_height`).
#[derive(Debug, Clone)]
pub struct VisualRow {
    /// Index of the logical (buffer) line this row belongs to.
    pub logical_line_index: usize,
    /// `true` only for the *first* visual row of a logical line.
    /// The gutter should render the line number here; all other rows get blank/soft-wrap.
    pub is_first_visual_row: bool,
    /// Y offset of this row's top, **relative to the top of the text content area**
    /// (i.e. already accounts for internal editor border + padding — do not add them again).
    pub y: f32,
    /// Height of this visual row in pixels.
    pub height: f32,
}

/// Break every logical line into its visual (screen) rows using glyph-wrap rules.
///
/// # Arguments
/// - `lines`               – the logical lines (split on `\n`), as string slices.
/// - `content_width`       – exact pixel width available for text
///                           (widget width − border − left_padding − right_padding).
/// - `char_width`          – pixel width of one monospace glyph.
/// - `single_line_height`  – pixel height of one unwrapped visual row.
///
/// # Returns
/// A `Vec<VisualRow>` listing every visual row in document order.
/// An empty line always produces exactly one `VisualRow`.
pub fn compute_visual_rows(
    lines: &[&str],
    content_width: f32,
    char_width: f32,
    single_line_height: f32,
) -> Vec<VisualRow> {
    let mut rows = Vec::new();
    let mut current_y = 0.0f32;

    let chars_per_row = if char_width > 0.0 && content_width > 0.0 {
        (content_width / char_width).floor() as usize
    } else {
        usize::MAX
    };

    for (logical_line_index, line) in lines.iter().enumerate() {
        // Count visual rows needed for this logical line.
        // An empty line still occupies exactly one visual row.
        let char_count = line.chars().count();
        let num_visual = if chars_per_row == 0 || char_count == 0 {
            1
        } else {
            ((char_count as f32) / (chars_per_row as f32)).ceil() as usize
        };
        // Clamp to at least 1
        let num_visual = num_visual.max(1);

        for sub in 0..num_visual {
            rows.push(VisualRow {
                logical_line_index,
                is_first_visual_row: sub == 0,
                y: current_y,
                height: single_line_height,
            });
            current_y += single_line_height;
        }
    }

    rows
}

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

/// Calculate the total rendered height of a single logical line, accounting for glyph-wrap.
///
/// This is a convenience wrapper around `compute_visual_rows` for a single line.
///
/// # Arguments
/// * `text` - The text content of the line
/// * `content_width` - Available width for text content
/// * `char_width` - Width of a single monospace character
/// * `single_line_height` - Height of a single unwrapped line
///
/// # Returns
/// The total height of the line in pixels (may span multiple visual rows if wrapped)
pub fn calculate_line_height(
    text: &str,
    content_width: f32,
    char_width: f32,
    single_line_height: f32,
) -> f32 {
    let rows = compute_visual_rows(&[text], content_width, char_width, single_line_height);
    rows.iter()
        .map(|r| r.height)
        .sum::<f32>()
        .max(single_line_height)
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
    let mut processed_chars = 0;

    // Collect character indices for proper string slicing
    let mut char_indices: Vec<(usize, char)> = text.char_indices().collect();
    // Add a dummy end index to handle the last slice
    char_indices.push((text.len(), '\0'));

    let mut i = 0;
    while i < char_indices.len() - 1 {
        // Identify next token (word or sequence of spaces)
        let start_idx = i;
        let (_, start_char) = char_indices[i];
        let is_whitespace = start_char.is_whitespace();

        while i < char_indices.len() - 1 {
            let (_, c) = char_indices[i];
            if c.is_whitespace() != is_whitespace {
                break;
            }
            i += 1;
        }
        // Token is from start_idx to i (exclusive)
        let token_len = i - start_idx; // number of chars in token

        // Measure token width
        let token_width = token_len as f32 * char_width;

        // Check wrap - if a word doesn't fit, wrap to next line
        if !is_whitespace && current_x + token_width > content_width && current_x > 0.0 {
            current_x = 0.0;
            current_y += single_line_height;
        }

        // Check if our target column is within this token
        if processed_chars + token_len > column {
            // Target is inside this token
            let offset_in_token = column - processed_chars;
            let offset_x = offset_in_token as f32 * char_width;
            return (current_x + offset_x, current_y);
        }

        current_x += token_width;
        processed_chars += token_len;
    }

    // If we fall through (e.g. column is at end of line), return current pos
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

    // ---- compute_visual_rows tests ----

    #[test]
    fn test_compute_visual_rows_empty_line() {
        // An empty logical line must produce exactly one VisualRow.
        let rows = compute_visual_rows(&[""], 800.0, 8.0, 20.0);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].logical_line_index, 0);
        assert!(rows[0].is_first_visual_row);
        assert_eq!(rows[0].y, 0.0);
        assert_eq!(rows[0].height, 20.0);
    }

    #[test]
    fn test_compute_visual_rows_short_line() {
        // A line that fits entirely within one row.
        let rows = compute_visual_rows(&["Hello"], 800.0, 8.0, 20.0);
        assert_eq!(rows.len(), 1);
        assert!(rows[0].is_first_visual_row);
    }

    #[test]
    fn test_compute_visual_rows_exact_fit() {
        // A line whose character count exactly equals chars_per_row.
        // 800 / 8 = 100 chars per row; a 100-char line → still 1 row.
        let text = "a".repeat(100);
        let rows = compute_visual_rows(&[&text], 800.0, 8.0, 20.0);
        assert_eq!(rows.len(), 1);
        assert!(rows[0].is_first_visual_row);
    }

    #[test]
    fn test_compute_visual_rows_wraps_to_two() {
        // 101 chars with 100 chars/row → 2 visual rows.
        let text = "a".repeat(101);
        let rows = compute_visual_rows(&[&text], 800.0, 8.0, 20.0);
        assert_eq!(rows.len(), 2);
        assert!(rows[0].is_first_visual_row);
        assert!(!rows[1].is_first_visual_row);
        assert_eq!(rows[0].logical_line_index, 0);
        assert_eq!(rows[1].logical_line_index, 0);
        assert!((rows[0].y - 0.0).abs() < f32::EPSILON);
        assert!((rows[1].y - 20.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_compute_visual_rows_multiple_logical_lines() {
        // Two logical lines: first wraps into 2 rows, second fits in 1.
        let long_line = "a".repeat(101);
        let short_line = "hi";
        let rows = compute_visual_rows(&[&long_line, short_line], 800.0, 8.0, 20.0);
        assert_eq!(rows.len(), 3);
        // Row 0: first visual row of logical line 0
        assert_eq!(rows[0].logical_line_index, 0);
        assert!(rows[0].is_first_visual_row);
        assert!((rows[0].y - 0.0).abs() < f32::EPSILON);
        // Row 1: continuation of logical line 0
        assert_eq!(rows[1].logical_line_index, 0);
        assert!(!rows[1].is_first_visual_row);
        assert!((rows[1].y - 20.0).abs() < f32::EPSILON);
        // Row 2: first visual row of logical line 1
        assert_eq!(rows[2].logical_line_index, 1);
        assert!(rows[2].is_first_visual_row);
        assert!((rows[2].y - 40.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_compute_visual_rows_no_lines() {
        let rows = compute_visual_rows(&[], 800.0, 8.0, 20.0);
        assert!(rows.is_empty());
    }

    // ---- calculate_gutter_width tests ----

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

        // Short text that fits in one line
        let text = "Hello, world!";
        let height = calculate_line_height(text, content_width, char_width, single_line_height);
        assert_eq!(height, single_line_height);
    }

    #[test]
    fn test_calculate_line_height_with_wrap() {
        let char_width = 8.0f32;
        let content_width = 800.0f32;
        let single_line_height = 20.0f32;

        // Long text that exceeds 100 chars per row (800/8)
        let text = "a".repeat(150);
        let height = calculate_line_height(&text, content_width, char_width, single_line_height);
        // Should be at least 2 rows worth of height
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
