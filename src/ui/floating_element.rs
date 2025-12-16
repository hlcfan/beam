use iced::{
    Element, Event, Length, Point, Rectangle, Size, Vector,
    advanced::{
        Clipboard, Shell, Widget,
        layout::{self, Layout},
        overlay, renderer,
        widget::{self, Operation, Tree},
    },
    mouse,
};

/// Specifies the anchor position for the floating element relative to the content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AnchorPosition {
    /// Positions the anchor at the top-right corner of the content.
    #[default]
    TopRight,
    /// Positions the anchor at the bottom-right corner of the content.
    BottomRight,
    /// Positions the anchor at the bottom-center of the content.
    BottomCenter,
}

/// A container that overlays an "anchor" element on top of a "content" element.
///
/// The anchor can be positioned relative to the content using `AnchorPosition` and an offset.
///
/// This widget handles mouse interactions by checking if the cursor is over the anchor.
/// If it is, the content receives a "hidden" cursor to prevent unwanted interactions
/// on the underlying content while interacting with the floating element.
pub struct FloatingElement<'a, Message, Theme, Renderer> {
    content: Element<'a, Message, Theme, Renderer>,
    anchor: Element<'a, Message, Theme, Renderer>,
    offset: Vector,
    position: AnchorPosition,
    height: Option<Length>,
}

impl<'a, Message, Theme, Renderer> FloatingElement<'a, Message, Theme, Renderer> {
    /// Creates a new `FloatingElement` wrapping the given content and anchor.
    pub fn new<C, A>(content: C, anchor: A) -> Self
    where
        C: Into<Element<'a, Message, Theme, Renderer>>,
        A: Into<Element<'a, Message, Theme, Renderer>>,
    {
        Self {
            content: content.into(),
            anchor: anchor.into(),
            offset: Vector::new(0.0, 0.0),
            position: AnchorPosition::default(),
            height: None,
        }
    }

    /// Sets the offset vector for the anchor position.
    pub fn offset(mut self, offset: Vector) -> Self {
        self.offset = offset;
        self
    }

    /// Sets the position of the anchor relative to the content.
    pub fn position(mut self, position: AnchorPosition) -> Self {
        self.position = position;
        self
    }

    /// Sets the height of the floating element.
    pub fn height(mut self, height: impl Into<Length>) -> Self {
        self.height = Some(height.into());
        self
    }
}

impl<'a, Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for FloatingElement<'a, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
    Message: Clone,
{
    fn size(&self) -> Size<Length> {
        let size = self.content.as_widget().size();
        Size {
            width: size.width,
            height: self.height.unwrap_or(size.height),
        }
    }

    fn size_hint(&self) -> Size<Length> {
        let size = self.content.as_widget().size_hint();
        Size {
            width: size.width,
            height: self.height.unwrap_or(size.height),
        }
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let limits = if let Some(height) = self.height {
            limits.height(height)
        } else {
            *limits
        };

        let content_node =
            self.content
                .as_widget_mut()
                .layout(&mut tree.children[0], renderer, &limits);

        // We only lay out content here. Anchor layout happens in overlay.
        layout::Node::with_children(content_node.size(), vec![content_node])
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
        let mut children = layout.children();
        let content_layout = children.next().unwrap();

        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            content_layout,
            cursor,
            viewport,
        );
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content), Tree::new(&self.anchor)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(&[&self.content, &self.anchor]);
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        let mut children = layout.children();
        let content_layout = children.next().unwrap();

        self.content.as_widget_mut().operate(
            &mut tree.children[0],
            content_layout,
            renderer,
            operation,
        );
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
        let mut children = layout.children();
        let content_layout = children.next().unwrap();

        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            content_layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        let mut children = layout.children();
        let content_layout = children.next().unwrap();

        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            content_layout,
            cursor,
            viewport,
            renderer,
        )
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        let layout = layout.children().next().unwrap();
        let bounds = layout.bounds();
        let position = bounds.position() + translation;

        // Pass the actual content size to the overlay so it can position the anchor correctly
        let content_size = bounds.size();

        Some(overlay::Element::new(Box::new(FloatingElementOverlay {
            anchor: &mut self.anchor,
            tree: &mut tree.children[1],
            position,
            content_size,
            offset: self.offset,
            anchor_position: self.position,
            viewport: *viewport,
        })))
    }
}

struct FloatingElementOverlay<'a, 'b, Message, Theme, Renderer> {
    anchor: &'b mut Element<'a, Message, Theme, Renderer>,
    tree: &'b mut Tree,
    position: Point,
    content_size: Size,
    offset: Vector,
    anchor_position: AnchorPosition,
    viewport: Rectangle,
}

impl<'a, 'b, Message, Theme, Renderer> overlay::Overlay<Message, Theme, Renderer>
    for FloatingElementOverlay<'a, 'b, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
    Message: Clone,
{
    fn layout(&mut self, renderer: &Renderer, bounds: Size) -> layout::Node {
        let limits = layout::Limits::new(Size::ZERO, bounds);
        let mut anchor_node = self
            .anchor
            .as_widget_mut()
            .layout(self.tree, renderer, &limits);

        let anchor_size = anchor_node.size();

        let (val_x, val_y) = match self.anchor_position {
            AnchorPosition::TopRight => (
                self.content_size.width - anchor_size.width - self.offset.x,
                self.offset.y,
            ),
            AnchorPosition::BottomRight => (
                self.content_size.width - anchor_size.width - self.offset.x,
                self.content_size.height - anchor_size.height - self.offset.y,
            ),
            AnchorPosition::BottomCenter => (
                (self.content_size.width - anchor_size.width) / 2.0 + self.offset.x,
                self.content_size.height - anchor_size.height - self.offset.y,
            ),
        };

        anchor_node =
            anchor_node.move_to(Point::new(self.position.x + val_x, self.position.y + val_y));

        layout::Node::with_children(bounds, vec![anchor_node])
    }

    fn draw(
        &self,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
    ) {
        let mut children = layout.children();
        let anchor_layout = children.next().unwrap();

        self.anchor.as_widget().draw(
            self.tree,
            renderer,
            theme,
            style,
            anchor_layout,
            cursor,
            &self.viewport,
        );
    }

    fn update(
        &mut self,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
    ) {
        let mut children = layout.children();
        let anchor_layout = children.next().unwrap();

        self.anchor.as_widget_mut().update(
            self.tree,
            event,
            anchor_layout,
            cursor,
            renderer,
            clipboard,
            shell,
            &self.viewport,
        );
    }

    fn operate(
        &mut self,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        let mut children = layout.children();
        let anchor_layout = children.next().unwrap();

        self.anchor.as_widget_mut().operate(
            self.tree,
            anchor_layout,
            renderer,
            operation,
        );
    }

    fn mouse_interaction(
        &self,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        let mut children = layout.children();
        let anchor_layout = children.next().unwrap();

        self.anchor.as_widget().mouse_interaction(
            self.tree,
            anchor_layout,
            cursor,
            &self.viewport,
            renderer,
        )
    }

    fn overlay<'c>(
        &'c mut self,
        layout: Layout<'c>,
        renderer: &Renderer,
    ) -> Option<overlay::Element<'c, Message, Theme, Renderer>> {
        let mut children = layout.children();
        let anchor_layout = children.next().unwrap();

        self.anchor.as_widget_mut().overlay(
            self.tree,
            anchor_layout,
            renderer,
            &self.viewport,
            Vector::ZERO, // Translation is already handled by the overlay position
        )
    }
}

impl<'a, Message, Theme, Renderer> From<FloatingElement<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: 'a + Clone,
    Theme: 'a,
    Renderer: renderer::Renderer + 'a,
{
    fn from(floating_element: FloatingElement<'a, Message, Theme, Renderer>) -> Self {
        Self::new(floating_element)
    }
}
