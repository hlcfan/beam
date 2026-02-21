use iced::advanced::text::{self, Paragraph as _, Text};
use iced::{
    Color, Element, Font, Length, Pixels, Rectangle, Vector,
    advanced::{
        Clipboard, Layout, Shell, Widget, layout, overlay, renderer,
        widget::{Operation, Tree},
    },
    event::Event,
    keyboard::Key,
    mouse,
    widget::text_editor::Position,
};

use crate::ui::widget_calc::{self, compute_visual_rows};
use log::info;
use std::cell::RefCell;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Undo,
    Redo,
    Find,
}

#[allow(missing_debug_implementations)]
pub struct EditorView<'a, Message, Theme, Renderer, F>
where
    Message: Clone,
    F: Fn(Action) -> Message + 'a,
{
    content: Element<'a, Message, Theme, Renderer>,
    on_change: F,
    selection: Option<(Position, Position)>,
    content_ref: Option<&'a iced::widget::text_editor::Content>,
    font: Font,
    text_size: Pixels,
    padding: f32,
    padding_right: f32,
    version: usize,
}

#[derive(Debug, Default)]
struct State {
    cache: RefCell<Cache>,
}

#[derive(Debug, Default)]
struct Cache {
    /// Pre-computed visual-row layout. This is the single source of truth
    /// for all Y-coordinate decisions in both the gutter and the highlight overlay.
    visual_rows: Vec<crate::ui::widget_calc::VisualRow>,
    /// Content width at which visual_rows was computed. Invalidated when this changes.
    last_width: f32,
    /// Editor content version at which visual_rows was computed. Invalidated when this changes.
    last_version: usize,
    /// Cached monospace character width.
    char_width: f32,
    /// Height of a single unwrapped visual row.
    single_line_height: f32,
}

impl<'a, Message, Theme, Renderer, F> EditorView<'a, Message, Theme, Renderer, F>
where
    Message: Clone,
    F: Fn(Action) -> Message + 'a,
{
    pub fn new<T>(content: T, on_change: F) -> Self
    where
        T: Into<Element<'a, Message, Theme, Renderer>>,
    {
        Self {
            content: content.into(),
            on_change,
            selection: None,
            content_ref: None,
            font: Font::MONOSPACE,
            text_size: Pixels(14.0),
            padding: 5.0,
            padding_right: 5.0,
            version: 0,
        }
    }

    pub fn selection(mut self, selection: Option<(Position, Position)>) -> Self {
        self.selection = selection;
        self
    }

    pub fn content_ref(mut self, content: &'a iced::widget::text_editor::Content) -> Self {
        self.content_ref = Some(content);
        self
    }

    pub fn font(mut self, font: Font) -> Self {
        self.font = font;
        self
    }

    pub fn size(mut self, size: impl Into<Pixels>) -> Self {
        self.text_size = size.into();
        self
    }

    pub fn padding(mut self, padding: f32) -> Self {
        self.padding = padding;
        self.padding_right = padding;
        self
    }

    pub fn padding_right(mut self, padding: f32) -> Self {
        self.padding_right = padding;
        self
    }

    pub fn version(mut self, version: usize) -> Self {
        self.version = version;
        self
    }
}

impl<'a, Message, Theme, Renderer, F> Widget<Message, Theme, Renderer>
    for EditorView<'a, Message, Theme, Renderer, F>
where
    Message: Clone,
    Renderer: iced::advanced::text::Renderer<Font = Font>,
    F: Fn(Action) -> Message + 'a,
{
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content))
    }

    fn size(&self) -> iced::Size<Length> {
        self.content.as_widget().size()
    }

    fn size_hint(&self) -> iced::Size<Length> {
        self.content.as_widget().size_hint()
    }

    fn state(&self) -> iced::advanced::widget::tree::State {
        iced::advanced::widget::tree::State::new(State::default())
    }

    fn tag(&self) -> iced::advanced::widget::tree::Tag {
        iced::advanced::widget::tree::Tag::of::<State>()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let gutter_width = if let Some(content) = self.content_ref {
            let line_count = content.line_count();

            // Measure "0" to get character width in monospace
            let paragraph = Renderer::Paragraph::with_text(Text {
                content: "0",
                bounds: iced::Size::INFINITE,
                size: self.text_size,
                line_height: text::LineHeight::default(),
                font: self.font,
                align_x: text::Alignment::Center,
                align_y: iced::alignment::Vertical::Center,
                shaping: text::Shaping::Basic,
                wrapping: text::Wrapping::default(),
            });
            let char_width = paragraph.min_bounds().width;

            widget_calc::calculate_gutter_width(line_count, char_width, self.padding)
        } else {
            0.0
        };

        let limits = limits.shrink(iced::Size::new(gutter_width, 0.0));

        let node = self
            .content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, &limits);

        let node = node.move_to(iced::Point::new(gutter_width, 0.0));
        let size = node.size().expand(iced::Size::new(gutter_width, 0.0));

        layout::Node::with_children(size, vec![node])
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        let children = layout.children();
        if let Some(child) = children.into_iter().next() {
            self.content
                .as_widget_mut()
                .operate(&mut tree.children[0], child, renderer, operation);
        }
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        // Intercept Cmd+Z and Cmd+Shift+Z BEFORE the wrapped widget sees them
        if let Event::Keyboard(iced::keyboard::Event::KeyPressed { key, modifiers, .. }) = event {
            // Check if the wrapped widget is focused (text_input or text_editor)
            let is_focused =
                if let iced::advanced::widget::tree::State::Some(state) = &tree.children[0].state {
                    if let Some(state) =
                        state.downcast_ref::<iced::widget::text_input::State<Renderer::Paragraph>>()
                    {
                        state.is_focused()
                    } else if let Some(state) = state
                        .downcast_ref::<iced::widget::text_editor::State<
                            iced::advanced::text::highlighter::PlainText,
                        >>()
                    {
                        state.is_focused()
                    } else if let Some(state) = state
                        .downcast_ref::<iced::widget::text_editor::State<
                            iced::highlighter::Highlighter,
                        >>()
                    {
                        state.is_focused()
                    } else {
                        false
                    }
                } else {
                    false
                };

            if is_focused {
                match (key.as_ref(), modifiers.command(), modifiers.shift()) {
                    (Key::Character(c), true, false) if c == "z" => {
                        // Undo: Cmd+Z
                        shell.publish((self.on_change)(Action::Undo));
                        return; // Don't forward event to wrapped widget
                    }
                    (Key::Character(c), true, false) if c == "y" => {
                        // Redo: Cmd+Y
                        shell.publish((self.on_change)(Action::Redo));
                        return;
                    }
                    (Key::Character(c), true, true) if c == "z" => {
                        // Redo: Cmd+Shift+Z
                        shell.publish((self.on_change)(Action::Redo));
                        return;
                    }
                    (Key::Character(c), true, _) if c == "f" => {
                        // Find: Cmd+F
                        shell.publish((self.on_change)(Action::Find));
                        return;
                    }
                    _ => (),
                }
            }
        }

        let children = layout.children();
        if let Some(child) = children.into_iter().next() {
            self.content.as_widget_mut().update(
                &mut tree.children[0],
                event,
                child,
                cursor,
                renderer,
                clipboard,
                shell,
                viewport,
            )
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor_position: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        let children = layout.children();
        if let Some(child) = children.into_iter().next() {
            self.content.as_widget().mouse_interaction(
                &tree.children[0],
                child,
                cursor_position,
                viewport,
                renderer,
            )
        } else {
            mouse::Interaction::default()
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        let children = layout.children();
        let child_layout = match children.into_iter().next() {
            Some(l) => l,
            None => return,
        };

        // Draw gutter if content_ref is present
        if let Some(content) = self.content_ref {
            let line_count = content.line_count();
            info!("===line_count: {:?}", line_count);

            let child_bounds = child_layout.bounds();

            // ── Single content-width formula used everywhere ──────────────────────
            // The text editor renders text in a sub-area that excludes its border (1 px)
            // and internal padding on left and right.  We must use exactly this width
            // when computing visual rows so wrap points match the actual rendering.
            let text_left_pad = self.padding + 1.0; // left_padding + border
            let text_right_pad = self.padding_right + 1.0; // right_padding + border
            let content_width = child_bounds.width - text_left_pad - text_right_pad;

            // ── Access cache ──────────────────────────────────────────────────────
            let state = tree.state.downcast_ref::<State>();
            let mut cache = state.cache.borrow_mut();

            // Measure char metrics once (invalidated when char_width is zero).
            if cache.char_width == 0.0 {
                let paragraph = Renderer::Paragraph::with_text(Text {
                    content: "0",
                    bounds: iced::Size::INFINITE,
                    size: self.text_size,
                    line_height: text::LineHeight::default(),
                    font: self.font,
                    align_x: text::Alignment::Left,
                    align_y: iced::alignment::Vertical::Center,
                    shaping: text::Shaping::Basic,
                    wrapping: text::Wrapping::default(),
                });
                let min_bounds = paragraph.min_bounds();
                cache.char_width = min_bounds.width;
                cache.single_line_height = self.text_size.0 * 1.3;
            }

            let char_width = cache.char_width;
            let single_line_height = cache.single_line_height;
            let gutter_width =
                widget_calc::calculate_gutter_width(line_count, char_width, self.padding);

            // Invalidate visual-row cache when content_width or version changed.
            let needs_rebuild = (cache.last_width - content_width).abs() > 0.1
                || cache.last_version != self.version
                || cache.visual_rows.is_empty();

            if needs_rebuild {
                // Collect all logical lines into owned Strings first (content.line() returns
                // Cow<str> with a lifetime bound to the iterator, so we materialise them before
                // building the &str slice).
                let owned: Vec<String> = (0..line_count)
                    .map(|i| {
                        content
                            .line(i)
                            .map(|l| l.text.to_string())
                            .unwrap_or_default()
                    })
                    .collect();
                let line_strs: Vec<&str> = owned.iter().map(|s| s.as_str()).collect();

                cache.visual_rows =
                    compute_visual_rows(&line_strs, content_width, char_width, single_line_height);
                cache.last_width = content_width;
                cache.last_version = self.version;
            }

            // Draw gutter background
            renderer.fill_quad(
                renderer::Quad {
                    bounds: Rectangle {
                        x: bounds.x,
                        y: bounds.y,
                        width: gutter_width,
                        height: bounds.height,
                    },
                    border: iced::Border {
                        color: Color::from_rgb(0.9, 0.9, 0.9),
                        width: 1.0,
                        radius: 0.0.into(),
                    },
                    shadow: iced::Shadow::default(),
                    snap: true,
                },
                Color::from_rgb8(184, 230, 254),
            );

            // Y offset to the top of the text content area inside the child widget.
            // child_bounds.y is already past the gutter strip; adding border+padding
            // brings us to the exact row where text is rendered.
            let text_top = child_bounds.y + text_left_pad;

            let viewport_start = viewport.y;
            let viewport_end = viewport.y + viewport.height;

            // ── Gutter draw loop (VisualRow-based) ──────────────────────────────
            for row in &cache.visual_rows {
                let abs_y = text_top + row.y;

                // Viewport culling
                if !widget_calc::is_line_in_viewport(
                    abs_y,
                    row.height,
                    viewport_start,
                    viewport_end,
                ) {
                    // Optimisation: once we are below the viewport we can stop.
                    if abs_y > viewport_end {
                        break;
                    }
                    continue;
                }

                if row.is_first_visual_row {
                    // Render the 1-based line number right-aligned in the gutter.
                    let number_text = (row.logical_line_index + 1).to_string();
                    renderer.fill_text(
                        iced::advanced::text::Text {
                            content: number_text,
                            bounds: iced::Size::new(gutter_width, single_line_height),
                            size: self.text_size,
                            line_height: text::LineHeight::default(),
                            font: self.font,
                            align_x: text::Alignment::Right,
                            align_y: iced::alignment::Vertical::Top,
                            shaping: text::Shaping::Basic,
                            wrapping: text::Wrapping::None,
                        },
                        iced::Point::new(child_bounds.x - 3.0, abs_y),
                        Color::from_rgb(0.6, 0.6, 0.6),
                        *viewport,
                    );
                }
                // Soft-wrap continuation rows: render blank (no line number).
            }
        }

        // Draw content
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            child_layout,
            cursor,
            viewport,
        );

        if let Some((start, end)) = self.selection {
            // Draw highlight ON TOP of content

            // Text editor has internal padding (default 5.0) and border (1.0)
            // We assume border is 1.0 based on usage in undoable_editor.rs.
            // The padding is passed via self.padding.
            let offset_left = self.padding + 1.0;
            let offset_right = self.padding_right + 1.0;
            let offset_y = self.padding + 1.0;

            // Adjust content_width to match the text_editor's actual text area width
            // The text_editor has a border and internal padding.
            // We assume standard usage: 1.0 border + 5.0 padding left + 5.0 padding right
            // Our offset_x accounts for padding + border on the left.
            // But we need to ensure the width reflects the available space for text.

            let sample = Renderer::Paragraph::with_text(Text {
                content: "M",
                bounds: iced::Size::INFINITE,
                size: self.text_size,
                line_height: text::LineHeight::default(),
                font: self.font,
                align_x: text::Alignment::Left,
                align_y: iced::alignment::Vertical::Center,
                shaping: text::Shaping::Basic,
                wrapping: text::Wrapping::default(),
            });
            let min_bounds = sample.min_bounds();
            let char_width = min_bounds.width;
            let single_line_height = min_bounds.height;

            // Do NOT subtract a scrollbar width: the text_editor lays out text
            // in its full child area minus its own internal padding; the scrollbar
            // is an overlay and does not affect the paragraph's text-layout width.
            // Subtracting it here would shift the Glyph-wrap point, causing
            // grapheme_position to return wrong X offsets for wrapped lines.
            let content_width = child_layout.bounds().width - offset_left - offset_right;

            if let Some(content) = self.content_ref {
                // Advanced calculation handling wrapping.
                // Reuse the VisualRow cache built during gutter rendering.
                let state = tree.state.downcast_ref::<State>();
                let mut cache = state.cache.borrow_mut();

                // Ensure cache is populated (it may not be if gutter was not rendered).
                if cache.char_width == 0.0 {
                    let p = Renderer::Paragraph::with_text(Text {
                        content: "0",
                        bounds: iced::Size::INFINITE,
                        size: self.text_size,
                        line_height: text::LineHeight::default(),
                        font: self.font,
                        align_x: text::Alignment::Left,
                        align_y: iced::alignment::Vertical::Center,
                        shaping: text::Shaping::Basic,
                        wrapping: text::Wrapping::default(),
                    });
                    let mb = p.min_bounds();
                    cache.char_width = mb.width;
                    cache.single_line_height = self.text_size.0 * 1.3;
                }
                let single_line_height = cache.single_line_height;
                let char_width_cached = cache.char_width;

                let needs_rebuild_sel = (cache.last_width - content_width).abs() > 0.1
                    || cache.last_version != self.version
                    || cache.visual_rows.is_empty();

                if needs_rebuild_sel {
                    let line_count = content.line_count();
                    let owned: Vec<String> = (0..line_count)
                        .map(|i| {
                            content
                                .line(i)
                                .map(|l| l.text.to_string())
                                .unwrap_or_default()
                        })
                        .collect();
                    let line_strs: Vec<&str> = owned.iter().map(|s| s.as_str()).collect();
                    cache.visual_rows = compute_visual_rows(
                        &line_strs,
                        content_width,
                        char_width_cached,
                        single_line_height,
                    );
                    cache.last_width = content_width;
                    cache.last_version = self.version;
                }

                // Find the Y offset of the first visual row for start.line.
                // VisualRow.y is relative to text content top; add border+padding to get absolute.
                let text_top = child_layout.bounds().y + offset_left; // border(1)+padding(left)
                let row_y = cache
                    .visual_rows
                    .iter()
                    .find(|r| r.logical_line_index == start.line && r.is_first_visual_row)
                    .map(|r| r.y)
                    .unwrap_or(0.0);

                let accumulated_y = row_y;
                let current_y = text_top + accumulated_y;

                // For now we assume start.line == end.line as per original code structure for search results

                if start.line == end.line {
                    let line_text = content
                        .line(start.line)
                        .map(|l| l.text.to_string())
                        .unwrap_or_default();

                    // Use paragraph grapheme_position for precise overlay positioning
                    // This delegates positioning to iced's text layout engine
                    let measure_content = if line_text.is_empty() {
                        " ".to_string()
                    } else {
                        line_text.clone()
                    };
                    let paragraph = Renderer::Paragraph::with_text(Text {
                        content: &measure_content,
                        bounds: iced::Size::new(content_width, f32::INFINITY),
                        size: self.text_size,
                        line_height: text::LineHeight::default(),
                        font: self.font,
                        align_x: text::Alignment::Left,
                        align_y: iced::alignment::Vertical::Top,
                        shaping: text::Shaping::Basic,
                        wrapping: text::Wrapping::Glyph,
                    });

                    // --- Glyph-wrap aware grapheme_position ---
                    // grapheme_position(line, index) expects:
                    //   line  = *visual* (wrapped) line index within the paragraph
                    //   index = grapheme index *within that visual line*
                    //
                    // For monospace Glyph-wrapping every character is the same width, so:
                    //   chars_per_row = floor(content_width / char_width)
                    //   visual_line   = logical_col / chars_per_row
                    //   col_in_row    = logical_col % chars_per_row
                    let chars_per_row = if char_width > 0.0 {
                        (content_width / char_width).floor() as usize
                    } else {
                        usize::MAX
                    };

                    let (start_visual_line, start_col_in_row) = if chars_per_row > 0 {
                        (start.column / chars_per_row, start.column % chars_per_row)
                    } else {
                        (0, start.column)
                    };
                    let (end_visual_line, end_col_in_row) = if chars_per_row > 0 {
                        (end.column / chars_per_row, end.column % chars_per_row)
                    } else {
                        (0, end.column)
                    };

                    let start_pos = paragraph
                        .grapheme_position(start_visual_line, start_col_in_row)
                        .unwrap_or(iced::Point::new(0.0, 0.0));
                    let (start_x, start_y_offset) = (start_pos.x, start_pos.y);
                    let end_pos = paragraph
                        .grapheme_position(end_visual_line, end_col_in_row)
                        .unwrap_or(iced::Point::new(0.0, 0.0));
                    let (end_x, end_y_offset) = (end_pos.x, end_pos.y);

                    // Draw highlighting
                    let abs_start_x = child_layout.bounds().x + offset_left + start_x;
                    let abs_y = current_y + start_y_offset;

                    if (start_y_offset - end_y_offset).abs() < 1.0 {
                        // Same visual line
                        renderer.fill_quad(
                            renderer::Quad {
                                bounds: Rectangle {
                                    x: abs_start_x,
                                    y: abs_y,
                                    width: end_x - start_x,
                                    height: single_line_height,
                                },
                                border: iced::Border::default(),
                                shadow: iced::Shadow::default(),
                                snap: true,
                            },
                            Color::from_rgba(0.2, 0.4, 0.7, 0.5),
                        );
                    } else {
                        // Multi-visual-line selection
                        // 1. First part
                        renderer.fill_quad(
                            renderer::Quad {
                                bounds: Rectangle {
                                    x: abs_start_x,
                                    y: abs_y,
                                    width: content_width - start_x,
                                    height: single_line_height,
                                },
                                border: iced::Border::default(),
                                shadow: iced::Shadow::default(),
                                snap: true,
                            },
                            Color::from_rgba(0.2, 0.4, 0.7, 0.5),
                        );

                        // 2. Middle parts (full width)
                        let mut y = start_y_offset + single_line_height;
                        while y < end_y_offset - 0.5 {
                            renderer.fill_quad(
                                renderer::Quad {
                                    bounds: Rectangle {
                                        x: child_layout.bounds().x + offset_left,
                                        y: current_y + y,
                                        width: content_width,
                                        height: single_line_height,
                                    },
                                    border: iced::Border::default(),
                                    shadow: iced::Shadow::default(),
                                    snap: true,
                                },
                                Color::from_rgba(0.2, 0.4, 0.7, 0.5),
                            );
                            y += single_line_height;
                        }

                        // 3. Last part
                        renderer.fill_quad(
                            renderer::Quad {
                                bounds: Rectangle {
                                    x: child_layout.bounds().x + offset_left,
                                    y: current_y + end_y_offset,
                                    width: end_x,
                                    height: single_line_height,
                                },
                                border: iced::Border::default(),
                                shadow: iced::Shadow::default(),
                                snap: true,
                            },
                            Color::from_rgba(0.2, 0.4, 0.7, 0.5),
                        );
                    }
                }
            } else {
                // Fallback to old simple logic if content_ref is missing
                let line_height = self.text_size.0 * 1.3;
                let current_y = bounds.y + offset_y + (start.line as f32) * line_height;
                let start_x =
                    child_layout.bounds().x + offset_left + (start.column as f32) * char_width;

                if start.line == end.line {
                    let width = ((end.column - start.column) as f32) * char_width;
                    renderer.fill_quad(
                        renderer::Quad {
                            bounds: Rectangle {
                                x: start_x,
                                y: current_y,
                                width,
                                height: line_height,
                            },
                            border: iced::Border::default(),
                            shadow: iced::Shadow::default(),
                            snap: true,
                        },
                        Color::from_rgba(0.2, 0.4, 0.7, 0.5),
                    );
                }
            }
        }
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

#[allow(missing_debug_implementations)]
impl<'a, Message, Theme, Renderer, F> From<EditorView<'a, Message, Theme, Renderer, F>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: 'a + Clone,
    Theme: 'a,
    Renderer: iced::advanced::text::Renderer<Font = Font> + 'a,
    F: Fn(Action) -> Message + 'a,
{
    fn from(editor_view: EditorView<'a, Message, Theme, Renderer, F>) -> Self {
        Self::new(editor_view)
    }
}
