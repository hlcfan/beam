use super::*;

type BuildInput = Box<dyn Fn(&mut Window, &mut App) -> Option<AnyElement>>;

/// Renders one `Input` or `Editor` component in a view of its own.
///
/// GPUI Fast draws a view again from last frame unless something it read changed. The input
/// components read and update their state entity while they render, so the view that renders a
/// component depends on that state. Rendered inside `BeamView`, every scroll, keystroke, and cursor
/// blink in any input rebuilt the whole window. Rendered here, the same change rebuilds only this
/// view, and `BeamView` is drawn again from last frame.
///
/// The host never reads or updates `BeamView`. Listeners that need `BeamView` belong on the
/// elements around the host.
pub(super) struct InputHost {
    build: BuildInput,
}

impl Render for InputHost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        (self.build)(window, cx).unwrap_or_else(|| div().into_any_element())
    }
}

/// The host views `BeamView` has created, keyed by the entity of the input each one renders.
#[derive(Default)]
pub(super) struct InputHosts(HashMap<EntityId, (AnyWeakEntity, Entity<InputHost>)>);

impl BeamView {
    /// Returns the host view for `state`, creating it on first use with `build`.
    ///
    /// The first `build` for an input is kept for the life of the host, so it must not capture
    /// values that change between renders of `BeamView`. Read theme values from the `App` it
    /// receives.
    pub(super) fn input_host<S: 'static>(
        &self,
        state: &Entity<S>,
        cx: &mut Context<Self>,
        build: impl Fn(&Entity<S>, &mut Window, &mut App) -> AnyElement + 'static,
    ) -> Entity<InputHost> {
        let mut hosts = self.input_hosts.borrow_mut();
        if let Some((_, host)) = hosts.0.get(&state.entity_id()) {
            return host.clone();
        }
        // Rebuilding the param, header, and auth rows replaces their inputs. Drop the hosts of
        // inputs that no longer exist before adding a new one.
        hosts.0.retain(|_, (input, _)| input.upgrade().is_some());
        let weak_state = state.downgrade();
        let host = cx.new(|_| InputHost {
            build: Box::new(move |window, cx| {
                let state = weak_state.upgrade()?;
                Some(build(&state, window, cx))
            }),
        });
        hosts
            .0
            .insert(state.entity_id(), (state.downgrade().into(), host.clone()));
        host
    }
}
