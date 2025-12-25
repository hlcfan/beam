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

use crate::ui::widget_calc;
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
    line_heights: Vec<f32>,
    last_width: f32,
    char_width: f32,
    single_line_height: f32,
    // Cache for highlight rendering
    last_selection: Option<(Position, Position)>,
    highlight_rects: Vec<Rectangle>,
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

            let child_bounds = child_layout.bounds();
            let content_width = child_bounds.width - 2.0; // Subtract approximate border/padding of text_editor

            // Access cache
            let state = tree.state.downcast_ref::<State>();
            let mut cache = state.cache.borrow_mut();

            // Only invalidate cache if width changed significantly
            let width_changed = (cache.last_width - content_width).abs() > 0.1;
            if width_changed {
                cache.line_heights.clear();
                cache.last_width = content_width;
                cache.char_width = 0.0; // Force remeasurement
            }

            // Cache char_width and single_line_height measurements
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
            let line_height = cache.single_line_height;
            let gutter_width =
                widget_calc::calculate_gutter_width(line_count, char_width, self.padding);

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
                Color::from_rgb(0.97, 0.97, 0.97),
            );

            // Draw line numbers
            let mut current_y = child_bounds.y + self.padding + 1.0;

            // Optimization: Calculate max chars that can fit in a line to avoid measuring every line
            let max_chars = if char_width > 0.0 {
                (content_width / char_width).floor() as usize
            } else {
                0
            };

            // Calculate visible line range for viewport culling
            let viewport_start = viewport.y;
            let viewport_end = viewport.y + viewport.height;

            for i in 0..line_count {
                // Optimization: Stop if we are below the viewport
                if current_y > viewport_end {
                    break;
                }

                let measured_height = if i < cache.line_heights.len() {
                    cache.line_heights[i]
                } else {
                    let line_text = match content.line(i) {
                        Some(l) => l.text,
                        None => continue,
                    };

                    let height = if line_text.len() <= max_chars {
                        line_height
                    } else {
                        // Only measure if potentially wrapping
                        let paragraph = Renderer::Paragraph::with_text(Text {
                            content: &line_text,
                            bounds: iced::Size::new(content_width, f32::INFINITY),
                            size: self.text_size,
                            line_height: text::LineHeight::default(),
                            font: self.font,
                            align_x: text::Alignment::Left,
                            align_y: iced::alignment::Vertical::Center,
                            shaping: text::Shaping::Basic,
                            wrapping: text::Wrapping::Word,
                        });
                        paragraph.min_bounds().height.max(line_height)
                    };
                    cache.line_heights.push(height);
                    height
                };

                // Optimization: Skip rendering if line is not in viewport
                if widget_calc::is_line_in_viewport(
                    current_y,
                    measured_height,
                    viewport_start,
                    viewport_end,
                ) {
                    let number_text = (i + 1).to_string();

                    renderer.fill_text(
                        iced::advanced::text::Text {
                            content: number_text,
                            bounds: iced::Size::new(gutter_width - self.padding * 1.0, line_height),
                            size: self.text_size,
                            line_height: text::LineHeight::default(),
                            font: self.font,
                            align_x: text::Alignment::Right,
                            align_y: iced::alignment::Vertical::Top,
                            shaping: text::Shaping::Basic,
                            wrapping: text::Wrapping::Word,
                        },
                        iced::Point::new(child_bounds.x - 3.0, current_y),
                        Color::from_rgb(0.6, 0.6, 0.6),
                        *viewport,
                    );
                }

                current_y += measured_height;
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

            // Estimate if scrollbar is needed
            let has_scrollbar = if let Some(content) = self.content_ref {
                let total_height = content.line_count() as f32 * single_line_height;
                total_height > bounds.height
            } else {
                false
            };

            let scrollbar_width = if has_scrollbar { 35.0 } else { 0.0 };
            let content_width =
                child_layout.bounds().width - offset_left - offset_right - scrollbar_width;

            if let Some(content) = self.content_ref {
                // Advanced calculation handling wrapping

                // 1. Calculate accumulated height of lines before start.line
                let state = tree.state.downcast_ref::<State>();
                let mut cache = state.cache.borrow_mut();

                // Reuse cached measurements from line number rendering
                let cached_char_width = cache.char_width;
                let cached_single_line_height = cache.single_line_height;

                // Use cached values if available, otherwise fall back to measurement
                let (char_width, single_line_height) = if cached_char_width > 0.0 {
                    (cached_char_width, cached_single_line_height)
                } else {
                    (char_width, single_line_height)
                };

                if cache.line_heights.len() < start.line {
                    for i in cache.line_heights.len()..start.line {
                        let line_text = content
                            .line(i)
                            .map(|l| l.text.to_string())
                            .unwrap_or_default();
                        let paragraph = Renderer::Paragraph::with_text(Text {
                            content: &line_text,
                            bounds: iced::Size::new(content_width, f32::INFINITY),
                            size: self.text_size,
                            line_height: text::LineHeight::default(),
                            font: self.font,
                            align_x: text::Alignment::Left,
                            align_y: iced::alignment::Vertical::Center,
                            shaping: text::Shaping::Basic,
                            wrapping: text::Wrapping::Word,
                        });
                        cache.line_heights.push(paragraph.min_bounds().height);
                    }
                }

                let accumulated_y: f32 = cache.line_heights.iter().take(start.line).sum();

                let current_y = bounds.y + offset_y + accumulated_y;

                // 2. Handle the start line
                // For now we assume start.line == end.line as per original code structure for search results
                if start.line == end.line {
                    let line_text = content
                        .line(start.line)
                        .map(|l| l.text.to_string())
                        .unwrap_or_default();

                    // Helper to measure position (x, y_offset) of a column index
                    // using manual word-wrapping simulation to match TextEditor behavior
                    let (start_x, start_y_offset) = widget_calc::simulate_word_wrap_position(
                        &line_text,
                        start.column,
                        char_width,
                        content_width,
                        single_line_height,
                    );
                    let (end_x, end_y_offset) = widget_calc::simulate_word_wrap_position(
                        &line_text,
                        end.column,
                        char_width,
                        content_width,
                        single_line_height,
                    );

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
