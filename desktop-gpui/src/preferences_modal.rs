//! Preferences blur the retained workspace in the ordered GPU paint pass,
//! before painting the modal card. No readback or delayed snapshot replacement.
use crate::workspace_windows::DesktopContextExt as _;
use super::*;
use gpui_kit::component::{WindowExt as _, FocusTrapElement as _};
use crate::chat::feedback::{self, Clock, FeedbackClock};

struct PreferencesOverlay {
    owner: WeakEntity<Desktop>,
    clock: Clock,
}
impl Render for PreferencesOverlay {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let progress = self.clock.borrow_mut().progress(cx.reduce_motion());
        let clock = self.clock.clone();
        self.owner.update(cx, |view, cx| view.with_workspace_window(window, cx, |view, window, cx| view.render_preferences_surface(window, cx, progress, clock)))
            .unwrap_or_else(|_| div().into_any_element())
    }
}

#[derive(Default)]
struct WorkspaceOverlayState { windows: std::collections::HashSet<WindowId> }
impl Global for WorkspaceOverlayState {}
pub(crate) fn workspace_blocked(cx: &App) -> bool { crate::session_lock::locked(cx) || cx.try_global::<WorkspaceOverlayState>().is_some_and(|state| cx.active_window().is_some_and(|window|state.windows.contains(&window.window_id()))) }

impl Desktop {
    pub(super) fn show_preferences(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.startup_pending || crate::session_lock::locked(cx) || self.closing || self.window.preferences_open { return; }
        self.window.preferences_previous_focus = window.focused(cx);
        self.window.preferences_open = true;
        if cx.try_global::<WorkspaceOverlayState>().is_none() {cx.set_global(WorkspaceOverlayState::default());}
        cx.global_mut::<WorkspaceOverlayState>().windows.insert(window.window_handle().window_id());
        cx.stop_active_drag(window);
        window.focus(&self.window.preferences_focus, cx);
        // Native webviews must not sit above the modal.
        self.window.dock.update(cx, |dock, cx| dock.set_shown(false, window, cx));
        cx.notify();
    }
    pub(super) fn close_preferences(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.current_workspace_window == *self.workspace_window_assignments.primary_window_id() {
            self.session_lock.update(cx,|lock,cx|lock.clear_fields(window,cx));
        }
        self.window.preferences_open = false;
        if cx.try_global::<WorkspaceOverlayState>().is_some() {cx.global_mut::<WorkspaceOverlayState>().windows.remove(&window.window_handle().window_id());}
        self.window.dock.update(cx, |dock, cx| dock.set_shown(true, window, cx));
        if let Some(focus) = self.window.preferences_previous_focus.take() { window.focus(&focus, cx); }
        cx.notify();
    }
    pub(super) fn render_preferences_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let owner = cx.weak_entity();
        let holder = window.use_keyed_state("preferences-overlay-state", cx, |_, cx| cx.new(|_| {
            let clock = FeedbackClock::new();
            clock.borrow_mut().start(std::time::Duration::from_millis(120));
            PreferencesOverlay { owner, clock }
        }));
        let overlay = holder.read(cx).clone();
        // The uncached overlay renders when its parent changes. Only its local
        // entrance clock requests additional frames.
        overlay.into_any_element()
    }
    fn render_preferences_surface(&mut self, window: &mut Window, cx: &mut Context<Self>, progress: f32, clock: Clock) -> AnyElement {
        let size = window.viewport_size();
        let palette = self.apply_native_theme(window, cx);
        let content = self.render_preferences_content(window, cx);
        div().id("native-preferences-modal").role(Role::Dialog).absolute().inset_0().occlude()
            .track_focus(&self.window.preferences_focus)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
            .capture_key_down(cx.workspace_listener(|view, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" && !window.has_active_dialog(cx) && !view.preferences_search_composing(window, cx) {
                    if !view.clear_preferences_search(window, cx) { view.close_preferences(window, cx); } cx.stop_propagation();
                }
            }))
            .flex().items_center().justify_center()
            // The ordered backdrop filter finishes before the card is painted,
            // so the first modal frame already has its blurred blackout.
            .child(div().id("preferences-backdrop").absolute().inset_0()
                .bg(Background::from(rgba(0x00000099)).backdrop_blur(20.))
                .on_mouse_down(MouseButton::Left, cx.workspace_listener(|view, _, window, cx| {
                    view.close_preferences(window, cx);
                    cx.stop_propagation();
                })))
            .child(feedback::painted(div().id("preferences-card").w(size.width * 0.9).max_w(design::ui_scaled_px(1296.)).h(size.height * 0.9)
                .debug_selector(|| "preferences-card".to_string())
                .min_w_0().min_h_0().flex().flex_col().overflow_hidden()
                .shadow(vec![
                    gpui_kit::BoxShadow::new(px(0.), px(18.), rgba(0x00000066).into()).blur_radius(px(48.)),
                    gpui_kit::BoxShadow::new(px(0.), px(4.), rgba(0x00000033).into()).blur_radius(px(12.)),
                ])
                .bg(crate::design::modal_surface_color(palette.paper)).rounded(design::ui_scaled_px(design::radius::MODAL)).border_1().border_color(rgb(palette.line))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(div().flex().items_center().justify_between().px(design::ui_scaled_px(design::space::SECTION)).py(design::ui_scaled_px(design::space::INSET)).border_b_1().border_color(rgb(palette.line))
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("Preferences"))
                    .child(Button::new("close-preferences").compact().label("Done").accessibility_label("Close preferences")
                        .on_click(cx.workspace_listener(|view, _, window, cx| view.close_preferences(window, cx)))))
                .child(div().flex_1().min_h_0().min_w_0().child(content))
                .opacity(progress)
                // GPUI fades primitives independently; suppress shadow bleed.
                .shadow(vec![
                    gpui_kit::BoxShadow::new(px(0.), px(18.), rgba(0x00000066).opacity(progress.powi(3)).into()).blur_radius(px(48.)),
                    gpui_kit::BoxShadow::new(px(0.), px(4.), rgba(0x00000033).opacity(progress.powi(3)).into()).blur_radius(px(12.)),
                ]), clock))
            .focus_trap("preferences-modal-focus", &self.window.preferences_focus)
            .into_any_element()
    }
}


#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SidebarMode {
    Hidden,
    Icons,
    Full,
}
impl SidebarMode {
    fn toggle_visibility(self) -> Self {
        match self {
            Self::Hidden => Self::Full,
            Self::Full | Self::Icons => Self::Hidden,
        }
    }
    fn visibility_target(self, expanded: f32, scale: u16) -> (Self, f32) {
        let next = self.toggle_visibility();
        (
            next,
            if next == Self::Hidden {
                0.
            } else {
                super::expanded_sidebar_width_for_scale(expanded, scale)
            },
        )
    }
    fn id(self) -> &'static str {
        match self {
            Self::Hidden => "hidden",
            Self::Icons => "resized-icons",
            Self::Full => "full",
        }
    }
}
fn sidebar_mode_from_saved(value: Option<&str>) -> SidebarMode {
    match value.map(str::trim) {
        Some("hidden") => SidebarMode::Hidden,
        Some("resized-icons") => SidebarMode::Icons,
        // Old icons/boolean preferences could be produced by the three-state
        // button. Only a new divider-resize token establishes compact intent.
        _ => SidebarMode::Full,
    }
}
fn load_sidebar_mode() -> SidebarMode {
    let Some(dir) = monitter_lib::experimental_desktop::profile_dir().ok() else {
        return SidebarMode::Full;
    };
    let value = std::fs::read_to_string(dir.join("sidebar-mode.v2")).ok();
    sidebar_mode_from_saved(value.as_deref())
}
impl Desktop {
    pub(super) fn toggle_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let from = self.displayed_sidebar_width(cx);
        if !self.window.sidebar_motion.active() && !self.window.sidebar_collapsed && !self.sidebar_is_hidden() {
            self.window.sidebar_expanded_width = super::expanded_sidebar_width_for_scale(self.current_sidebar_width(cx),self.window.sidebar_scale);
        }
        let current = if self.window.sidebar_hidden { SidebarMode::Hidden } else if self.window.sidebar_collapsed { SidebarMode::Icons } else { SidebarMode::Full };
        let (next,target) = current.visibility_target(self.window.sidebar_expanded_width,self.window.sidebar_scale);
        self.window.sidebar_motion_compact = self.window.sidebar_collapsed;
        self.window.sidebar_hidden = next == SidebarMode::Hidden;
        // The button only changes visibility. Showing restores the last full
        // width; compact presentation is entered solely by a divider gesture.
        self.window.sidebar_collapsed = false;
        if self.window.sidebar_collapsed || (self.sidebar_is_hidden() && self.window.sidebar_motion_compact) { crate::shell::set_task_markers_visible(false, self.window.sidebar_view, cx); }
        if self.sidebar_is_hidden() && self.window.sidebar_focus.contains_focused(window, cx) {
            window.focus(&self.window.sidebar_toggle_focus, cx);
        }
        save_sidebar_mode(next);
        self.start_sidebar_motion(from, target, window, cx);
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
    use super::{sidebar_mode_from_saved, SidebarMode};
    #[test]
    fn sidebar_only_restores_icons_from_a_confirmed_divider_resize_token() {
        assert_eq!(sidebar_mode_from_saved(Some("icons")), SidebarMode::Full);
        assert_eq!(sidebar_mode_from_saved(None), SidebarMode::Full);
        assert_eq!(
            sidebar_mode_from_saved(Some("resized-icons\n")),
            SidebarMode::Icons
        );
        assert_eq!(sidebar_mode_from_saved(Some("hidden")), SidebarMode::Hidden);
        assert_eq!(SidebarMode::Icons.id(), "resized-icons");
    }
    #[test]
    fn sidebar_button_only_toggles_visible_and_hidden_and_restores_full_width_mode() {
        assert_eq!(SidebarMode::Full.toggle_visibility(), SidebarMode::Hidden);
        assert_eq!(SidebarMode::Icons.toggle_visibility(), SidebarMode::Hidden);
        assert_eq!(SidebarMode::Hidden.toggle_visibility(), SidebarMode::Full);
        assert_eq!(
            SidebarMode::Full.toggle_visibility().toggle_visibility(),
            SidebarMode::Full
        );
        for scale in [100, 150] {
            let (off, width) = SidebarMode::Full.visibility_target(278., scale);
            assert_eq!((off, width), (SidebarMode::Hidden, 0.));
            assert_eq!(
                off.visibility_target(278., scale),
                (SidebarMode::Full, 278.)
            );
            assert_eq!(
                SidebarMode::Icons.visibility_target(278., scale),
                (SidebarMode::Hidden, 0.)
            );
        }
    }
}
