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
    max_content_width: f32,
    // Cache for highlight rendering
    last_version: Option<usize>,
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

impl<'a, Message, Theme, Renderer, F> EditorView<'a, Message, Theme, Renderer, F>
where
    Message: Clone,
    Renderer: iced::advanced::text::Renderer<Font = Font>,
    F: Fn(Action) -> Message + 'a,
{
    fn measure_line_width(text: &str, size: Pixels, font: Font) -> f32 {
        let p = Renderer::Paragraph::with_text(Text {
            content: text,
            bounds: iced::Size::new(f32::INFINITY, f32::INFINITY),
            size,
            line_height: text::LineHeight::default(),
            font,
            align_x: text::Alignment::Left,
            align_y: iced::alignment::Vertical::Center,
            shaping: text::Shaping::Basic,
            wrapping: text::Wrapping::None,
        });
        p.min_bounds().width
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
        let (gutter_width, max_content_width) = if let Some(content) = self.content_ref {
            let line_count = content.line_count();
            let state = tree.state.downcast_ref::<State>();
            let mut cache = state.cache.borrow_mut();

            // Cache char_width and single_line_height measurements
            if cache.char_width == 0.0 {
                let paragraph = Renderer::Paragraph::with_text(Text {
                    content: "0",
                    bounds: iced::Size::INFINITE,
                    size: self.text_size,
                    line_height: text::LineHeight::default(),
                    font: self.font,
                    align_x: text::Alignment::Center,
                    align_y: iced::alignment::Vertical::Center,
                    shaping: text::Shaping::Basic,
                    wrapping: text::Wrapping::None,
                });
                let min_bounds = paragraph.min_bounds();
                cache.char_width = min_bounds.width;
                cache.single_line_height = self.text_size.0 * 1.3;
            }

            if cache.last_version != Some(self.version) {
                cache.max_content_width = 0.0;
                cache.last_version = Some(self.version);

                // Measure all lines to find max width
                // Note: unique version check prevents doing this every frame
                for i in 0..line_count {
                    let line_text = content
                        .line(i)
                        .map(|l| l.text.to_string())
                        .unwrap_or_default();

                    let w = Self::measure_line_width(&line_text, self.text_size, self.font);
                    if w > cache.max_content_width {
                        cache.max_content_width = w;
                    }
                }
            }

            let gutter_width =
                widget_calc::calculate_gutter_width(line_count, cache.char_width, self.padding);
            (gutter_width, cache.max_content_width)
        } else {
            (0.0, 0.0)
        };

        // Ensure inner element is measured with enough width
        let content_width = max_content_width + self.padding + self.padding_right;

        // We want the inner text editor to be at least content_width,
        // but also respect the limits if they are larger (e.g. fill the screen if text is short)
        let limits = limits.shrink(iced::Size::new(gutter_width, 0.0));

        // We pass relaxed limits to child so it can expand
        // But since text_editor might not expand automatically with Wrapping::None,
        // we might not see a difference unless we force size.
        // However, we can use the measured content_width to determine the final node size.

        let node = self
            .content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, &limits);

        let node = node.move_to(iced::Point::new(gutter_width, 0.0));

        // Final size must include gutter and be at least as wide as content + padding
        let min_width = gutter_width + content_width;

        // If limits.max().width is infinite, we are likely in a scrollable container.
        // We should return the true content width to allow scrolling.
        // Otherwise, we respect the constraints (likely window width).
        let final_width = if limits.max().width.is_infinite() {
            min_width
        } else {
            node.size().width + gutter_width
        };

        layout::Node::with_children(iced::Size::new(final_width, node.size().height), vec![node])
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

            // Estimate if scrollbar is needed to adjust content_width
            // This needs to match the logic in height measurement
            let sample_line_height = self.text_size.0 * 1.3;
            let total_height_estimate = line_count as f32 * sample_line_height;
            let has_scrollbar = total_height_estimate > child_bounds.height;
            let scrollbar_width = if has_scrollbar { 35.0 } else { 0.0 };

            let content_width = child_bounds.width
                - (self.padding + 1.0)
                - (self.padding_right + 1.0)
                - scrollbar_width;

            // Access cache
            let state = tree.state.downcast_ref::<State>();
            let mut cache = state.cache.borrow_mut();

            // Only invalidate cache if width changed significantly or version changed
            let width_changed = (cache.last_width - content_width).abs() > 0.1;
            // version check moved to layout for max width calculation
            // let version_changed = cache.last_version != self.version;

            if width_changed {
                cache.line_heights.clear();
                cache.last_width = content_width;
                // cache.char_width = 0.0; // Don't reset char_width as it is constant for font
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
                    wrapping: text::Wrapping::None,
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

            let mut current_y = child_bounds.y + self.padding + 1.0;

            // Calculate visible line range for viewport culling
            let viewport_start = viewport.y;
            let viewport_end = viewport.y + viewport.height;

            for i in 0..line_count {
                let measured_height = line_height;

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
                            wrapping: text::Wrapping::None,
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
            let offset_left = self.padding + 1.0;
            let offset_y = self.padding + 1.0;

            let child_bounds = child_layout.bounds();

            if let Some(content) = self.content_ref {
                let state = tree.state.downcast_ref::<State>();
                let mut cache = state.cache.borrow_mut();

                // Ensure measurements are cached
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
                        wrapping: text::Wrapping::None,
                    });
                    let min_bounds = paragraph.min_bounds();
                    cache.char_width = min_bounds.width;
                    cache.single_line_height = self.text_size.0 * 1.3;
                }

                let single_line_height = cache.single_line_height;

                // Iterate through lines involved in selection
                for i in start.line..=end.line {
                    let line_text = content
                        .line(i)
                        .map(|l| l.text.to_string())
                        .unwrap_or_default();

                    let col_start = if i == start.line { start.column } else { 0 };
                    let col_end = if i == end.line {
                        end.column
                    } else {
                        line_text.chars().count()
                    };

                    if col_start >= col_end {
                        continue;
                    }

                    // Measure text up to col_start and col_end
                    // Note: This relies on chars count mapping to text substring width
                    let start_str: String = line_text.chars().take(col_start).collect();
                    let end_str: String = line_text.chars().take(col_end).collect();

                    let x_start = Self::measure_line_width(&start_str, self.text_size, self.font);
                    let width =
                        Self::measure_line_width(&end_str, self.text_size, self.font) - x_start;

                    let y_pos = child_bounds.y + offset_y + (i as f32) * single_line_height;
                    let x_pos = child_bounds.x + offset_left + x_start;

                    renderer.fill_quad(
                        renderer::Quad {
                            bounds: Rectangle {
                                x: x_pos,
                                y: y_pos,
                                width,
                                height: single_line_height,
                            },
                            border: iced::Border::default(),
                            shadow: iced::Shadow::default(),
                            snap: true,
                        },
                        Color::from_rgba(0.2, 0.4, 0.7, 0.5),
                    );
                }
            } else {
                // Fallback to old simple logic if content_ref is missing
                let char_width = self.text_size.0 * 0.6; // Rough estimate
                let line_height = self.text_size.0 * 1.3;
                let current_y = child_bounds.y + offset_y + (start.line as f32) * line_height;
                let start_x = child_bounds.x + offset_left + (start.column as f32) * char_width;

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
