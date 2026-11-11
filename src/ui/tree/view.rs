use super::*;

/// Renders the workspace tree in a view of its own.
///
/// GPUI Fast notifies the view that contains a scrollable element when it scrolls, and the
/// virtual list updates the view it was given while it draws. Inside `BeamView`, every scroll
/// event rebuilt the whole window. Here the same scroll rebuilds only this view.
///
/// This view reads `BeamView` while it renders, so it draws again whenever `BeamView` is notified.
/// It never updates `BeamView` while it renders, lays out, or paints. Listeners reach `BeamView`
/// through [`beam_listener`], which updates it only when the event fires.
pub(in crate::ui) struct WorkspaceTreeView {
    beam: WeakEntity<BeamView>,
}

impl WorkspaceTreeView {
    pub(in crate::ui) fn new(beam: WeakEntity<BeamView>) -> Self {
        Self { beam }
    }
}

impl Render for WorkspaceTreeView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(beam) = self.beam.upgrade() else {
            return div().into_any_element();
        };
        let tree_view = cx.entity();
        beam.read(cx)
            .render_workspace_panel(&beam, tree_view, cx)
            .into_any_element()
    }
}

/// Wraps `listener` as an event handler that updates `beam` when the event fires.
///
/// Unlike `Context::listener`, it does not need a `Context<BeamView>`, so the tree view can build
/// handlers for elements that `BeamView` owns.
pub(in crate::ui) fn beam_listener<E>(
    beam: &Entity<BeamView>,
    listener: impl Fn(&mut BeamView, &E, &mut Window, &mut Context<BeamView>) + 'static,
) -> impl Fn(&E, &mut Window, &mut App) + 'static {
    let beam = beam.downgrade();
    move |event, window, cx| {
        beam.update(cx, |this, cx| listener(this, event, window, cx))
            .ok();
    }
}
