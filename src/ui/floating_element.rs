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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AnchorPosition {
    #[default]
    TopRight,
    BottomRight,
    BottomCenter,
}

pub struct FloatingElement<'a, Message, Theme, Renderer> {
    content: Element<'a, Message, Theme, Renderer>,
    anchor: Element<'a, Message, Theme, Renderer>,
    offset: Vector,
    position: AnchorPosition,
}

impl<'a, Message, Theme, Renderer> FloatingElement<'a, Message, Theme, Renderer> {
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
        }
    }

    pub fn offset(mut self, offset: Vector) -> Self {
        self.offset = offset;
        self
    }

    pub fn position(mut self, position: AnchorPosition) -> Self {
        self.position = position;
        self
    }
}

impl<'a, Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for FloatingElement<'a, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
{
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn size_hint(&self) -> Size<Length> {
        self.content.as_widget().size_hint()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let content_node =
            self.content
                .as_widget_mut()
                .layout(&mut tree.children[0], renderer, limits);

        let anchor_limits = limits.loose();
        let mut anchor_node =
            self.anchor
                .as_widget_mut()
                .layout(&mut tree.children[1], renderer, &anchor_limits);

        // Position anchor based on configured position
        let content_size = content_node.size();
        let anchor_size = anchor_node.size();

        let (val_x, val_y) = match self.position {
            AnchorPosition::TopRight => (
                content_size.width - anchor_size.width - self.offset.x,
                self.offset.y,
            ),
            AnchorPosition::BottomRight => (
                content_size.width - anchor_size.width - self.offset.x,
                content_size.height - anchor_size.height - self.offset.y,
            ),
            AnchorPosition::BottomCenter => (
                (content_size.width - anchor_size.width) / 2.0 + self.offset.x,
                content_size.height - anchor_size.height - self.offset.y,
            ),
        };

        anchor_node = anchor_node.move_to(Point::new(val_x, val_y));

        layout::Node::with_children(content_size, vec![content_node, anchor_node])
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
        let anchor_layout = children.next().unwrap();

        let is_over_anchor = cursor
            .position()
            .map(|p| anchor_layout.bounds().contains(p))
            .unwrap_or(false);

        let content_cursor = if is_over_anchor {
            mouse::Cursor::Available(Point::new(-1.0, -1.0))
        } else {
            cursor
        };

        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            content_layout,
            content_cursor,
            viewport,
        );

        self.anchor.as_widget().draw(
            &tree.children[1],
            renderer,
            theme,
            style,
            anchor_layout,
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
        let anchor_layout = children.next().unwrap();

        self.anchor.as_widget_mut().operate(
            &mut tree.children[1],
            anchor_layout,
            renderer,
            operation,
        );
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
        let anchor_layout = children.next().unwrap();

        let is_over_anchor = cursor
            .position()
            .map(|p| anchor_layout.bounds().contains(p))
            .unwrap_or(false);

        let content_cursor = if is_over_anchor {
            mouse::Cursor::Available(Point::new(-1.0, -1.0))
        } else {
            cursor
        };

        let (content_tree, anchor_tree) = tree.children.split_at_mut(1);
        let content_tree = &mut content_tree[0];
        let anchor_tree = &mut anchor_tree[0];

        // Update anchor first
        self.anchor.as_widget_mut().update(
            anchor_tree,
            event,
            anchor_layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );

        // Update content second
        self.content.as_widget_mut().update(
            content_tree,
            event,
            content_layout,
            content_cursor,
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
        let anchor_layout = children.next().unwrap();

        let is_over_anchor = cursor
            .position()
            .map(|p| anchor_layout.bounds().contains(p))
            .unwrap_or(false);

        let content_cursor = if is_over_anchor {
            mouse::Cursor::Available(Point::new(-1.0, -1.0))
        } else {
            cursor
        };

        let anchor_interaction = self.anchor.as_widget().mouse_interaction(
            &tree.children[1],
            anchor_layout,
            cursor,
            viewport,
            renderer,
        );

        let content_interaction = self.content.as_widget().mouse_interaction(
            &tree.children[0],
            content_layout,
            content_cursor,
            viewport,
            renderer,
        );

        if anchor_interaction != mouse::Interaction::None {
            anchor_interaction
        } else {
            content_interaction
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
        let mut children = layout.children();
        let content_layout = children.next().unwrap();
        let anchor_layout = children.next().unwrap();

        let (content_tree, anchor_tree) = tree.children.split_at_mut(1);
        let content_tree = &mut content_tree[0];
        let anchor_tree = &mut anchor_tree[0];

        // Check if anchor has overlay? Maybe dropdowns in anchor?
        let anchor_overlay = self.anchor.as_widget_mut().overlay(
            anchor_tree,
            anchor_layout,
            renderer,
            viewport,
            translation,
        );

        if anchor_overlay.is_some() {
            return anchor_overlay;
        }

        self.content.as_widget_mut().overlay(
            content_tree,
            content_layout,
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a, Message, Theme, Renderer> From<FloatingElement<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: 'a,
    Renderer: renderer::Renderer + 'a,
{
    fn from(floating_element: FloatingElement<'a, Message, Theme, Renderer>) -> Self {
        Self::new(floating_element)
    }
}
