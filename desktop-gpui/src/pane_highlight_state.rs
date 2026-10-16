//! Per-frame visibility bookkeeping for the native active-pane outline.
/// Tracks whether an outline was requested during the current GPUI frame and
/// whether the native overlay is currently visible.
#[derive(Default)]
pub(crate) struct OverlayFrameState {
    observed_this_frame: bool,
    candidate_this_frame: bool,
    visible: bool,
}

impl OverlayFrameState {
    /// Starts a GPUI frame. The native view stays visible until the frame's
    /// deferred finalizer knows whether an active pane painted an outline.
    pub(crate) fn begin_frame(&mut self) {
        self.observed_this_frame = false;
        self.candidate_this_frame = false;
    }

    /// Records the pane lifecycle before deferred painting. This survives GPUI
    /// cache replay, where the deferred canvas scene is replayed without
    /// invoking its paint closure.
    pub(crate) fn observe_candidate(&mut self, candidate: bool) {
        self.observed_this_frame = true;
        self.candidate_this_frame |= candidate;
    }

    /// Records an active outline and returns true when the native view needs
    /// to transition from hidden to visible.
    pub(crate) fn mark_painted(&mut self) -> bool {
        self.observe_candidate(true);
        let was_hidden = !self.visible;
        self.visible = true;
        was_hidden
    }

    /// Finishes the frame and returns true when a previously visible outline
    /// should now be hidden because no active pane painted one.
    pub(crate) fn finish_frame(&mut self) -> bool {
        // A clean cached DockArea may replay its deferred outline without
        // calling the native paint callback. With no lifecycle observation,
        // preserve the last visibility state. A rendered pane with no active
        // candidate (closed, unfocused, or blocked) may hide it.
        let should_hide = self.observed_this_frame && !self.candidate_this_frame && self.visible;
        if should_hide {
            self.visible = false;
        }
        self.observed_this_frame = false;
        self.candidate_this_frame = false;
        should_hide
    }

    /// Explicitly hides the outline for transitions such as session locking.
    pub(crate) fn clear(&mut self) -> bool {
        self.observed_this_frame = false;
        self.candidate_this_frame = false;
        let was_visible = self.visible;
        self.visible = false;
        was_visible
    }
}

#[cfg(test)]
mod tests {
    use super::OverlayFrameState;

    #[test]
    fn steady_active_frames_do_not_toggle_native_visibility() {
        let mut state = OverlayFrameState::default();

        state.begin_frame();
        state.observe_candidate(true);
        assert!(state.mark_painted(), "first active frame shows the overlay");
        assert!(!state.finish_frame());

        for _ in 0..4 {
            state.begin_frame();
            state.observe_candidate(true);
            assert!(
                !state.mark_painted(),
                "steady active frames must leave the overlay visible"
            );
            assert!(!state.finish_frame());
        }
    }

    #[test]
    fn absent_candidate_hides_once_after_the_frame() {
        let mut state = OverlayFrameState::default();
        state.begin_frame();
        assert!(state.mark_painted());
        assert!(!state.finish_frame());

        state.begin_frame();
        state.observe_candidate(false);
        assert!(state.finish_frame());
        state.begin_frame();
        assert!(!state.finish_frame(), "idle frames must not keep hiding");
    }

    #[test]
    fn explicit_clear_hides_and_resets_the_frame_candidate() {
        let mut state = OverlayFrameState::default();
        state.begin_frame();
        state.observe_candidate(true);
        assert!(state.mark_painted());
        assert!(!state.finish_frame());

        assert!(state.clear());
        assert!(!state.clear());
        state.begin_frame();
        assert!(!state.finish_frame());
    }

    #[test]
    fn cached_deferred_replay_preserves_visibility_until_pane_is_observed_absent() {
        let mut state = OverlayFrameState::default();
        state.begin_frame();
        state.observe_candidate(true);
        assert!(state.mark_painted());
        assert!(!state.finish_frame());

        // A cached deferred draw replays paint output but does not execute the
        // native paint closure or NativePanel::pane_overlay render callback.
        state.begin_frame();
        assert!(
            !state.finish_frame(),
            "cache replay must not hide the outline"
        );

        state.begin_frame();
        state.observe_candidate(true);
        assert!(
            !state.mark_painted(),
            "the overlay stayed visible through the cached frame"
        );
        assert!(!state.finish_frame());

        state.begin_frame();
        state.observe_candidate(false);
        assert!(state.finish_frame(), "observed absence hides the outline");
    }
}
