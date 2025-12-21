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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Undo,
    Redo,
    Find,
}

#[allow(missing_debug_implementations)]
pub struct Undoable<'a, Message, Theme, Renderer, F>
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
}

impl<'a, Message, Theme, Renderer, F> Undoable<'a, Message, Theme, Renderer, F>
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
        self
    }
}

impl<'a, Message, Theme, Renderer, F> Widget<Message, Theme, Renderer>
    for Undoable<'a, Message, Theme, Renderer, F>
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
        self.content.as_widget().state()
    }

    fn tag(&self) -> iced::advanced::widget::tree::Tag {
        self.content.as_widget().tag()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let gutter_width = if let Some(content) = self.content_ref {
            let line_count = content.line_count();
            let digits = line_count.to_string().len();
            // Estimate width: digits * char_width + padding
            // We use a rough estimation or measure '0' * digits
            // For monospace, all chars are same width.
            // Let's measure "0"
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
            let min_bounds = paragraph.min_bounds();
            let char_width = min_bounds.width;

            (digits as f32 * char_width) + char_width + self.padding
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
            let digits = line_count.to_string().len();

            // Measure "0" for char width
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
            let char_width = min_bounds.width;
            let line_height_factor = 1.3; // Default line height factor
            let line_height = self.text_size.0 * line_height_factor;

            let gutter_width = (digits as f32 * char_width) + char_width + self.padding;
            let child_bounds = child_layout.bounds();
            let content_width = child_bounds.width - 2.0; // Subtract approximate border/padding of text_editor

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

            for i in 0..line_count {
                let line_text_str = match content.line(i) {
                    Some(l) => l.text.to_string(), // Need string for measurement if wrapping
                    None => continue,
                };

                let measured_height = if line_text_str.len() <= max_chars {
                    line_height
                } else {
                    // Only measure if potentially wrapping
                    let paragraph = Renderer::Paragraph::with_text(Text {
                        content: line_text_str.as_str(),
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

                // Draw number only if visible
                if current_y + measured_height > viewport.y
                    && current_y < viewport.y + viewport.height
                {
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

            // Measure "M" to get char width and roughly line height
            let paragraph = Renderer::Paragraph::with_text(Text {
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

            let min_bounds = paragraph.min_bounds();
            let char_width = min_bounds.width;
            // Line height is usually consistent.
            // TextEditor uses 1.3 * size for line height usually.
            // But checking min_bounds.height might give the height of the line box if "M" is representative.
            // A safer bet for line height in text_editor is usually derived from size.
            // Default line height in Iced is 1.3
            let line_height = self.text_size.0 * 1.3;

            let bounds = layout.bounds();

            // Text editor has internal padding (default 5.0) and border (1.0)
            // We assume border is 1.0 based on usage in undoable_editor.rs.
            // The padding is passed via self.padding.
            let offset_x = self.padding + 1.0;
            let offset_y = self.padding + 1.0;

            let current_y = bounds.y + offset_y + (start.line as f32) * line_height;
            let start_x = child_layout.bounds().x + offset_x + (start.column as f32) * char_width;

            log::info!(
                "Undoable::draw highlight - selection: {:?} -> {:?}, bounds: {:?}, child_bounds: {:?}, start_x: {}, current_y: {}, line_height: {}, char_width: {}",
                start,
                end,
                bounds,
                child_layout.bounds(),
                start_x,
                current_y,
                line_height,
                char_width
            );

            // If selection is on the same line
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
                    Color::from_rgba(0.2, 0.4, 0.7, 0.5), // Semi-transparent highlight
                );
            } else {
                // Multi-line selection - naive implementation
                // Draw first line
                // We don't know line length, so we draw a large width? No that's ugly.
                // For now, we only support single line highlighting nicely.
                // Or just highlight what we can.
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
impl<'a, Message, Theme, Renderer, F> From<Undoable<'a, Message, Theme, Renderer, F>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: 'a + Clone,
    Theme: 'a,
    Renderer: iced::advanced::text::Renderer<Font = Font> + 'a,
    F: Fn(Action) -> Message + 'a,
{
    fn from(undoable: Undoable<'a, Message, Theme, Renderer, F>) -> Self {
        Self::new(undoable)
    }
}
