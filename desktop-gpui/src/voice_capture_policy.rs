//! Pure microphone preference and key-held state.
#[derive(Default)]
pub(crate) struct CapturePolicy {
    pub push_mode: bool,
    pub explicit_muted: bool,
    held: bool,
}
impl CapturePolicy {
    pub fn desired_muted(&self) -> bool {
        self.explicit_muted || (self.push_mode && !self.held)
    }
    pub fn set_held(&mut self, held: bool) -> bool {
        if self.held == held {
            return false;
        }
        self.held = held;
        self.push_mode
    }
    pub fn toggle_explicit_mute(&mut self) {
        self.explicit_muted = !self.explicit_muted;
    }
    pub fn toggle_mode(&mut self) {
        self.push_mode = !self.push_mode;
        self.held = false;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn held_and_focus_loss_gate_capture() {
        let mut p = CapturePolicy::default();
        assert!(!p.desired_muted());
        p.toggle_mode();
        assert!(p.desired_muted());
        assert!(p.set_held(true));
        assert!(!p.desired_muted());
        assert!(p.set_held(false));
        assert!(p.desired_muted());
    }
    #[test]
    fn explicit_mute_survives_mode_and_press_changes() {
        let mut p = CapturePolicy::default();
        p.toggle_explicit_mute();
        p.toggle_mode();
        p.set_held(true);
        assert!(p.desired_muted());
        p.toggle_mode();
        assert!(p.desired_muted());
        p.toggle_explicit_mute();
        assert!(!p.desired_muted());
    }
    #[test]
    fn key_up_during_unmute_restores_muted_target() {
        let mut p = CapturePolicy::default();
        p.toggle_mode();
        p.set_held(true);
        assert!(!p.desired_muted());
        p.set_held(false);
        assert!(p.desired_muted());
    }
}
