//! Preferences blur the retained workspace in the ordered GPU paint pass,
//! before painting the modal card. No readback or delayed snapshot replacement.
use super::*;
use gpui_kit::component::{WindowExt as _, FocusTrapElement as _};

#[derive(Default)]
struct WorkspaceOverlayState { open: bool }
impl Global for WorkspaceOverlayState {}
pub(crate) fn workspace_blocked(cx: &App) -> bool { crate::session_lock::locked(cx) || cx.try_global::<WorkspaceOverlayState>().is_some_and(|state| state.open) }

impl Desktop {
    pub(super) fn show_preferences(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if crate::session_lock::locked(cx) || self.closing || self.preferences_open { return; }
        self.preferences_previous_focus = window.focused(cx);
        self.preferences_open = true;
        cx.set_global(WorkspaceOverlayState { open: true });
        cx.stop_active_drag(window);
        window.focus(&self.preferences_focus, cx);
        // Native webviews must not sit above the modal.
        self.dock.update(cx, |dock, cx| dock.set_shown(false, window, cx));
        cx.notify();
    }
    pub(super) fn close_preferences(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.session_lock.update(cx,|lock,cx|lock.clear_fields(window,cx));
        self.preferences_open = false;
        cx.set_global(WorkspaceOverlayState { open: false });
        self.dock.update(cx, |dock, cx| dock.set_shown(true, window, cx));
        if let Some(focus) = self.preferences_previous_focus.take() { window.focus(&focus, cx); }
        cx.notify();
    }
    pub(super) fn render_preferences_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let size = window.viewport_size();
        let palette = self.apply_native_theme(window, cx);
        let content = self.render_preferences_content(window, cx);
        div().id("native-preferences-modal").role(Role::Dialog).absolute().inset_0().occlude()
            .track_focus(&self.preferences_focus)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
            .capture_key_down(cx.listener(|view, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" && !window.has_active_dialog(cx) {
                    view.close_preferences(window, cx); cx.stop_propagation();
                }
            }))
            .flex().items_center().justify_center()
            // The ordered backdrop filter finishes before the card is painted,
            // so the first modal frame already has its blurred blackout.
            .child(div().id("preferences-backdrop").absolute().inset_0()
                .bg(Background::from(rgba(0x00000045)).backdrop_blur(20.))
                .on_mouse_down(MouseButton::Left, cx.listener(|view, _, window, cx| {
                    view.close_preferences(window, cx);
                    cx.stop_propagation();
                })))
            .child(div().id("preferences-card").w(size.width * 0.9).max_w(gpui_kit::px(1200.)).h(size.height * 0.9)
                .debug_selector(|| "preferences-card".to_string())
                .min_w_0().min_h_0().flex().flex_col().overflow_hidden()
                .shadow(vec![
                    gpui_kit::BoxShadow::new(px(0.), px(18.), rgba(0x00000066).into()).blur_radius(px(48.)),
                    gpui_kit::BoxShadow::new(px(0.), px(4.), rgba(0x00000033).into()).blur_radius(px(12.)),
                ])
                .bg(crate::design::modal_surface_color(palette.paper)).rounded(design::ui_scaled_px(design::radius::CARD)).border_1().border_color(rgb(palette.line))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(div().flex().items_center().justify_between().px(design::ui_scaled_px(design::space::SECTION)).py(design::ui_scaled_px(design::space::INSET)).border_b_1().border_color(rgb(palette.line))
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("Preferences"))
                    .child(Button::new("close-preferences").compact().label("Done").accessibility_label("Close preferences")
                        .on_click(cx.listener(|view, _, window, cx| view.close_preferences(window, cx)))))
                .child(div().flex_1().min_h_0().min_w_0().child(content)))
            .focus_trap("preferences-modal-focus", &self.preferences_focus)
            .into_any_element()
    }
}


#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SidebarMode { Hidden, Icons, Full }
impl SidebarMode {
    fn next(self) -> Self { match self { Self::Full => Self::Icons, Self::Icons => Self::Hidden, Self::Hidden => Self::Full } }
    fn id(self) -> &'static str { match self { Self::Hidden => "hidden", Self::Icons => "icons", Self::Full => "full" } }
}
fn load_sidebar_mode() -> SidebarMode {
    let Some(dir) = monitter_lib::experimental_desktop::profile_dir().ok() else { return SidebarMode::Full; };
    match std::fs::read_to_string(dir.join("sidebar-mode.v2")).ok().as_deref().map(str::trim) {
        Some("hidden") => SidebarMode::Hidden,
        Some("icons") => SidebarMode::Icons,
        Some("full") => SidebarMode::Full,
        _ => if std::fs::read(dir.join("sidebar-collapsed.v1")).is_ok_and(|bytes| bytes == b"true\n") { SidebarMode::Icons } else { SidebarMode::Full },
    }
}
impl Desktop {
    pub(super) fn toggle_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.sidebar_collapsed && !self.sidebar_is_hidden() {
            self.sidebar_expanded_width = super::expanded_sidebar_width(self.current_sidebar_width(cx));
        }
        let tern = design::tile_theme() == monitter_lib::experimental_desktop::NativeTileTheme::Tern;
        let current = if self.sidebar_hidden && tern { SidebarMode::Hidden } else if self.sidebar_collapsed { SidebarMode::Icons } else { SidebarMode::Full };
        let next = if tern { current.next() } else if self.sidebar_collapsed { SidebarMode::Full } else { SidebarMode::Icons };
        self.sidebar_hidden = next == SidebarMode::Hidden;
        self.sidebar_collapsed = next == SidebarMode::Icons;
        save_sidebar_mode(next);
        let width = if self.sidebar_collapsed { design::compact_sidebar_width_design(self.sidebar_scale) } else { super::expanded_sidebar_width(self.sidebar_expanded_width) };
        self.sidebar_width_touched = true;
        self.sidebar_resize_suppressed = true;
        self.sidebar_resize_generation = self.sidebar_resize_generation.wrapping_add(1);
        let generation = self.sidebar_resize_generation;
        self.sidebar_resize.update(cx, |state, cx| state.resize_panel(0, design::ui_px(width, self.sidebar_scale), window, cx));
        // Reattaching the split after Hidden emits layout resize events using
        // the previous rail width. Keep those events from changing this mode.
        cx.spawn_in(window, async move |view, cx| {
            cx.background_executor().timer(std::time::Duration::from_millis(250)).await;
            let _ = view.update_in(cx, |view, window, cx| {
                if view.sidebar_resize_generation == generation {
                    if !view.sidebar_is_hidden() {
                        let width = if view.sidebar_collapsed { design::compact_sidebar_width_design(view.sidebar_scale) } else { super::expanded_sidebar_width(view.sidebar_expanded_width) };
                        // Hidden removes the split. Restore width once the
                        // reattached split has current window bounds.
                        view.sidebar_resize.update(cx, |state, cx| state.resize_panel(0, design::ui_px(width, view.sidebar_scale), window, cx));
                    }
                    view.sidebar_resize_suppressed = false;
                    cx.notify();
                }
            });
        }).detach();
        self.queue_preferences_save(cx);
        cx.notify();
    }
}
/// GUI-only presentation state, compatible with an older resident backend.
pub fn load_sidebar_collapse() -> bool { load_sidebar_mode() == SidebarMode::Icons }
pub fn load_sidebar_hidden() -> bool { load_sidebar_mode() == SidebarMode::Hidden }
pub fn save_sidebar_collapse(collapsed: bool) { save_sidebar_mode(if collapsed { SidebarMode::Icons } else { SidebarMode::Full }); }
fn save_sidebar_mode(mode: SidebarMode) {
    static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let generation = GENERATION.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
    let _ = std::thread::Builder::new().name("monitter-sidebar-presentation".into()).spawn(move || {
        let Ok(_guard) = LOCK.lock() else { return; };
        if GENERATION.load(std::sync::atomic::Ordering::SeqCst) != generation { return; }
        if let Ok(dir) = monitter_lib::experimental_desktop::profile_dir() {
            let temp = dir.join("sidebar-mode.v2.tmp");
            if std::fs::write(&temp, format!("{}\n", mode.id())).is_ok() { let _ = std::fs::rename(temp, dir.join("sidebar-mode.v2")); }
        }
    });
}
#[cfg(test)]
mod sidebar_mode_tests {
    use super::SidebarMode;
    #[test]
    fn tern_sidebar_cycle_has_a_reachable_hidden_state_and_restore_path() {
        assert_eq!(SidebarMode::Full.next(), SidebarMode::Icons);
        assert_eq!(SidebarMode::Icons.next(), SidebarMode::Hidden);
        assert_eq!(SidebarMode::Hidden.next(), SidebarMode::Full);
    }
}
