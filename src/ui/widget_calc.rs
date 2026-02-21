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

/// Break every logical line into its visual (screen) rows using a provided line measurement closure.
///
/// # Arguments
/// - `lines`               – the logical lines (split on `\n`), as string slices.
/// - `single_line_height`  – pixel height of one unwrapped visual row.
/// - `measure_line`        – a closure that takes a single logical line string and returns
///                           the number of visual rows it will occupy when wrapped.
///
/// # Returns
/// A `Vec<VisualRow>` listing every visual row in document order.
/// An empty line always produces exactly one `VisualRow`.
pub fn compute_visual_rows(
    lines: &[&str],
    single_line_height: f32,
    measure_line: impl Fn(&str) -> usize,
) -> Vec<VisualRow> {
    let mut rows = Vec::new();
    let mut current_y = 0.0f32;

    for (logical_line_index, line) in lines.iter().enumerate() {
        // Measure how many visual rows this logical line needs.
        let num_visual = measure_line(line).max(1);

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
        let rows = compute_visual_rows(&[""], 20.0, |_| 1);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].logical_line_index, 0);
        assert!(rows[0].is_first_visual_row);
        assert_eq!(rows[0].y, 0.0);
        assert_eq!(rows[0].height, 20.0);
    }

    #[test]
    fn test_compute_visual_rows_short_line() {
        // A line that fits entirely within one row.
        let rows = compute_visual_rows(&["Hello"], 20.0, |_| 1);
        assert_eq!(rows.len(), 1);
        assert!(rows[0].is_first_visual_row);
    }

    #[test]
    fn test_compute_visual_rows_wraps_to_two() {
        // Mock a line measuring 2 visual rows
        let rows = compute_visual_rows(&["long line wrapping"], 20.0, |_| 2);
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
        let mock_measure = |text: &str| {
            if text == "long line" { 2 } else { 1 }
        };
        let rows = compute_visual_rows(&["long line", "short"], 20.0, mock_measure);
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
        let rows = compute_visual_rows(&[], 20.0, |_| 1);
        assert!(rows.is_empty());
    }

    // ---- calculate_gutter_width tests ----

    #[test]
    fn test_calculate_gutter_width() {
        let char_width = 8.0;
        let padding = 5.0;

        assert_eq!(calculate_gutter_width(5, char_width, padding), 21.0);
        assert_eq!(calculate_gutter_width(9, char_width, padding), 21.0);
        assert_eq!(calculate_gutter_width(10, char_width, padding), 29.0);
        assert_eq!(calculate_gutter_width(99, char_width, padding), 29.0);
        assert_eq!(calculate_gutter_width(100, char_width, padding), 37.0);
        assert_eq!(calculate_gutter_width(999, char_width, padding), 37.0);
        assert_eq!(calculate_gutter_width(1000, char_width, padding), 45.0);
    }

    #[test]
    fn test_is_line_in_viewport() {
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
        assert!(is_line_in_viewport(40.0, 20.0, 50.0, 200.0));
        assert!(is_line_in_viewport(190.0, 20.0, 50.0, 200.0));
        assert!(!is_line_in_viewport(10.0, 20.0, 50.0, 200.0));
        assert!(!is_line_in_viewport(250.0, 20.0, 50.0, 200.0));
    }
}
