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
use std::cell::RefCell;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    Undo,
    Redo,
    Find,
    ScrollToMatch(f32),
}

#[allow(missing_debug_implementations)]
pub struct EditorView<'a, Message, Theme, Renderer, F>
where
    Message: Clone,
    F: Fn(Action) -> Message + 'a,
{
    content: Element<'a, Message, Theme, Renderer>,
    on_change: F,
    search_active_match: Option<(Position, Position)>,
    search_query: Option<String>,
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
    last_active_match: Option<(Position, Position)>,
}

#[derive(Debug, Default)]
struct Cache {
    /// Pre-computed visual-row layout. This is the single source of truth
    /// for all Y-coordinate decisions in both the gutter and the highlight overlay.
    visual_rows: Vec<crate::ui::widget_calc::VisualRow>,
    /// Pre-computed search matches bounds cache.
    search_matches: Vec<iced::Rectangle>,
    /// Search query at which search_matches was computed. Invalidated on changes.
    last_search_query: Option<String>,
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
            search_active_match: None,
            search_query: None,
            content_ref: None,
            font: Font::MONOSPACE,
            text_size: Pixels(14.0),
            padding: 5.0,
            padding_right: 5.0,
            version: 0,
        }
    }

    pub fn search_active_match(
        mut self,
        search_active_match: Option<(Position, Position)>,
    ) -> Self {
        self.search_active_match = search_active_match;
        self
    }

    pub fn search_query(mut self, search_query: String) -> Self {
        self.search_query = Some(search_query);
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
        operation.custom(
            None,
            layout.bounds(),
            &mut tree.state as &mut dyn std::any::Any,
        );

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
        {
            let state = tree.state.downcast_mut::<State>();
            if state.last_active_match != self.search_active_match {
                state.last_active_match = self.search_active_match;
                if let Some(active) = self.search_active_match {
                    if let Some(row) = state
                        .cache
                        .borrow()
                        .visual_rows
                        .iter()
                        .find(|r| r.logical_line_index == active.0.line)
                    {
                        shell.publish((self.on_change)(Action::ScrollToMatch(row.y)));
                    }
                }
            }
        }

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

            let child_bounds = child_layout.bounds();

            // ── Single content-width formula used everywhere ──────────────────────
            // The text editor renders text in a sub-area that excludes its internal padding
            // on left and right. We must use exactly this width when computing visual rows.
            let text_left_pad = self.padding;
            let text_right_pad = self.padding_right;
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
                cache.single_line_height = min_bounds.height;
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

                let measure_line = |text: &str| -> usize {
                    let measure_text = if text.is_empty() { " " } else { text };
                    let paragraph = Renderer::Paragraph::with_text(Text {
                        content: measure_text,
                        bounds: iced::Size::new(content_width, f32::INFINITY),
                        size: self.text_size,
                        line_height: text::LineHeight::default(),
                        font: self.font,
                        align_x: text::Alignment::Left,
                        align_y: iced::alignment::Vertical::Top,
                        shaping: text::Shaping::Basic,
                        wrapping: text::Wrapping::Glyph,
                    });

                    let min_height = paragraph.min_bounds().height;
                    let num_visual = (min_height / single_line_height).round() as usize;
                    num_visual.max(1)
                };

                cache.visual_rows =
                    compute_visual_rows(&line_strs, single_line_height, measure_line);
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
                        width: 0.0,
                        radius: 0.0.into(),
                    },
                    shadow: iced::Shadow::default(),
                    snap: true,
                },
                Color::from_rgb(0.98, 0.98, 0.98),
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

        // The text_editor's background is natively handled by the framework when it draws,
        // BUT we set text_editor's background to TRANSPARENT so we could draw highlights underneath text.
        // Iced widget drawing happens bottom up, so if we don't draw the background here, it will be blank if the parent doesn't have a background.
        // `EditorView`'s parent `Container` in `UndoableEditor` can draw the background instead!
        // So we just skip drawing background here to avoid generic `Theme` trait bound issues.

        let offset_left = self.padding;
        let offset_right = self.padding_right;
        let content_width = child_layout.bounds().width - offset_left - offset_right;

        // Populate Cache
        if let Some(content) = self.content_ref {
            let state = tree.state.downcast_ref::<State>();
            let mut cache = state.cache.borrow_mut();

            // Populate char metrics
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
                cache.single_line_height = mb.height;
            }
            let single_line_height = cache.single_line_height;

            let needs_rebuild = (cache.last_width - content_width).abs() > 0.1
                || cache.last_version != self.version;

            if needs_rebuild {
                // ... same calculation for visual_rows ...
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

                let measure_line = |text: &str| -> usize {
                    let measure_text = if text.is_empty() { " " } else { text };
                    let paragraph = Renderer::Paragraph::with_text(Text {
                        content: measure_text,
                        bounds: iced::Size::new(content_width, f32::INFINITY),
                        size: self.text_size,
                        line_height: text::LineHeight::default(),
                        font: self.font,
                        align_x: text::Alignment::Left,
                        align_y: iced::alignment::Vertical::Top,
                        shaping: text::Shaping::Basic,
                        wrapping: text::Wrapping::Glyph,
                    });

                    let min_height = paragraph.min_bounds().height;
                    let num_visual = (min_height / single_line_height).round() as usize;
                    num_visual.max(1)
                };

                cache.visual_rows =
                    compute_visual_rows(&line_strs, single_line_height, measure_line);
                cache.last_width = content_width;
                cache.last_version = self.version;
            }

            // Always recalculate search highlights if content/query changed
            let search_query_changed =
                cache.last_search_query.as_deref() != self.search_query.as_deref();
            if needs_rebuild || search_query_changed {
                cache.last_search_query = self.search_query.clone();
                cache.search_matches.clear();

                if let Some(query) = &self.search_query {
                    if !query.is_empty() {
                        // Scan content line by line instead of whole text to build per-line paragraph geometry easily
                        let line_count = content.line_count();
                        for i in 0..line_count {
                            if let Some(line) = content.line(i) {
                                let line_text = line.text;
                                let mut start_idx = 0;
                                while let Some(match_idx) = line_text[start_idx..].find(query) {
                                    let absolute_idx = start_idx + match_idx;
                                    let match_len = query.len();

                                    // Build spans to measure the exact bounding boxes of the match
                                    let span_before = &line_text[0..absolute_idx];
                                    let span_match =
                                        &line_text[absolute_idx..absolute_idx + match_len];
                                    let span_after = &line_text[absolute_idx + match_len..];

                                    use iced::advanced::text::Span;
                                    let spans = vec![
                                        Span::<()>::new(span_before),
                                        Span::<()>::new(span_match),
                                        Span::<()>::new(span_after),
                                    ];

                                    let paragraph = Renderer::Paragraph::with_spans(Text {
                                        content: spans.as_slice(),
                                        bounds: iced::Size::new(content_width, f32::INFINITY),
                                        size: self.text_size,
                                        line_height: text::LineHeight::default(),
                                        font: self.font,
                                        align_x: text::Alignment::Left,
                                        align_y: iced::alignment::Vertical::Top,
                                        shaping: text::Shaping::Basic,
                                        wrapping: text::Wrapping::Glyph,
                                    });

                                    // Get bounding boxes of the match span (index 1)
                                    let bounds = paragraph.span_bounds(1);

                                    // Y offset for the current line
                                    let line_y = cache
                                        .visual_rows
                                        .iter()
                                        .find(|r| {
                                            r.logical_line_index == i && r.is_first_visual_row
                                        })
                                        .map(|r| r.y)
                                        .unwrap_or(0.0);

                                    for rect in bounds {
                                        cache.search_matches.push(Rectangle {
                                            x: rect.x,
                                            y: rect.y + line_y,
                                            width: rect.width,
                                            height: rect.height,
                                        });
                                    }

                                    start_idx = absolute_idx + match_len;
                                }
                            }
                        }
                    }
                }
            }

            // Draw Search Highlights
            let text_top = child_layout.bounds().y + offset_left; // border(1)+padding(left)
            let base_x = child_layout.bounds().x + offset_left;

            // Calculate active match bounds exactly as we did the passive ones if it is set.
            let mut active_rects = Vec::new();
            if let Some((start_pos, end_pos)) = self.search_active_match {
                if start_pos.line == end_pos.line {
                    if let Some(line) = content.line(start_pos.line) {
                        let line_text = line.text;
                        // Note: iced positions use cursor positions, but indices in text
                        // To properly highlight the active span we could convert columns to byte indices,
                        // but since search query logic in main.rs operates on exact string matches,
                        // we can do a simplified span match here.
                        // But grapheme/col calculation works for monospace:
                        // grapheme/col calculation properly maps columns to byte offsets
                        if let Some((before_len, match_len)) =
                            crate::ui::widget_calc::get_byte_offsets_for_columns(
                                line_text.as_ref(),
                                start_pos.column,
                                end_pos.column,
                            )
                        {
                            let span_before = &line_text[0..before_len];
                            let span_match = &line_text[before_len..before_len + match_len];
                            let span_after = &line_text[before_len + match_len..];

                            use iced::advanced::text::Span;
                            let spans = vec![
                                Span::<()>::new(span_before),
                                Span::<()>::new(span_match),
                                Span::<()>::new(span_after),
                            ];

                            let paragraph = Renderer::Paragraph::with_spans(Text {
                                content: spans.as_slice(),
                                bounds: iced::Size::new(content_width, f32::INFINITY),
                                size: self.text_size,
                                line_height: text::LineHeight::default(),
                                font: self.font,
                                align_x: text::Alignment::Left,
                                align_y: iced::alignment::Vertical::Top,
                                shaping: text::Shaping::Basic,
                                wrapping: text::Wrapping::Glyph,
                            });

                            let bounds = paragraph.span_bounds(1);

                            let line_y = cache
                                .visual_rows
                                .iter()
                                .find(|r| {
                                    r.logical_line_index == start_pos.line && r.is_first_visual_row
                                })
                                .map(|r| r.y)
                                .unwrap_or(0.0);

                            for rect in bounds {
                                active_rects.push(Rectangle {
                                    x: rect.x + base_x,
                                    y: rect.y + line_y + text_top,
                                    width: rect.width,
                                    height: rect.height,
                                });
                            }
                        }
                    }
                }
            }

            let passive_color = Color::from_rgba(1.0, 0.85, 0.0, 0.4);
            let active_color = Color::from_rgba(1.0, 0.55, 0.0, 0.7);

            // Draw passive
            for rect in &cache.search_matches {
                let abs_x = rect.x + base_x;
                let abs_y = rect.y + text_top;

                // Skip drawing this one if it exactly overlaps an active match
                let is_active = active_rects
                    .iter()
                    .any(|ar| (ar.x - abs_x).abs() < 1.0 && (ar.y - abs_y).abs() < 1.0);
                if !is_active {
                    renderer.fill_quad(
                        renderer::Quad {
                            bounds: Rectangle {
                                x: abs_x,
                                y: abs_y,
                                width: rect.width,
                                height: rect.height,
                            },
                            border: iced::Border::default(),
                            shadow: iced::Shadow::default(),
                            snap: true,
                        },
                        passive_color,
                    );
                }
            }

            // Draw active
            for rect in active_rects {
                renderer.fill_quad(
                    renderer::Quad {
                        bounds: rect,
                        border: iced::Border::default(),
                        shadow: iced::Shadow::default(),
                        snap: true,
                    },
                    active_color,
                );
            }
        }

        // Now draw the text content on top so the highlight appears *behind* it!
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            child_layout,
            cursor,
            viewport,
        );
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
// Custom operation to query the visual Y position of a matching line
pub struct QueryScrollY {
    pub logical_line: usize,
    pub result: Option<f32>,
}

impl QueryScrollY {
    pub fn new(logical_line: usize) -> Self {
        Self {
            logical_line,
            result: None,
        }
    }
}

impl iced::advanced::widget::Operation<f32> for QueryScrollY {
    fn traverse(
        &mut self,
        operate: &mut dyn FnMut(&mut dyn iced::advanced::widget::Operation<f32>),
    ) {
        operate(self)
    }

    fn custom(
        &mut self,
        _id: Option<&iced::widget::Id>,
        _bounds: iced::Rectangle,
        state: &mut dyn std::any::Any,
    ) {
        if let Some(tree_state) = state.downcast_mut::<iced::advanced::widget::tree::State>() {
            let editor_state = tree_state.downcast_mut::<State>();
            if let Some(row) = editor_state
                .cache
                .borrow()
                .visual_rows
                .iter()
                .find(|r| r.logical_line_index == self.logical_line)
            {
                self.result = Some(row.y);
            }
        }
    }

    fn finish(&self) -> iced::advanced::widget::operation::Outcome<f32> {
        if let Some(y) = self.result {
            iced::advanced::widget::operation::Outcome::Some(y)
        } else {
            iced::advanced::widget::operation::Outcome::None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::widget_calc::VisualRow;
    use std::cell::RefCell;

    #[test]
    fn test_query_scroll_y_finds_row() {
        // Build mock cache with some visual rows
        let cache = Cache {
            visual_rows: vec![
                VisualRow {
                    logical_line_index: 0,
                    y: 0.0,
                    is_first_visual_row: true,
                    height: 20.0,
                },
                VisualRow {
                    logical_line_index: 10,
                    y: 125.0,
                    is_first_visual_row: true,
                    height: 20.0,
                },
            ],
            search_matches: vec![],
            last_search_query: None,
            last_width: 0.0,
            last_version: 0,
            char_width: 0.0,
            single_line_height: 0.0,
        };

        let state = State {
            cache: RefCell::new(cache),
            last_active_match: None,
        };

        let mut tree_state = iced::advanced::widget::tree::State::new(state);

        // Test querying an existing line
        let mut op = QueryScrollY::new(10);
        op.custom(
            None,
            iced::Rectangle::default(),
            &mut tree_state as &mut dyn std::any::Any,
        );

        if let iced::advanced::widget::operation::Outcome::Some(y) = op.finish() {
            assert_eq!(y, 125.0);
        } else {
            panic!("Expected Outcome::Some(125.0)");
        }
    }

    #[test]
    fn test_query_scroll_y_not_found() {
        let state = State::default();
        let mut tree_state = iced::advanced::widget::tree::State::new(state);

        // Test querying a non-existent line
        let mut op = QueryScrollY::new(99);
        op.custom(
            None,
            iced::Rectangle::default(),
            &mut tree_state as &mut dyn std::any::Any,
        );

        assert!(matches!(
            op.finish(),
            iced::advanced::widget::operation::Outcome::None
        ));
    }
}
