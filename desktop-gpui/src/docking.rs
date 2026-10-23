//! Native DockArea adapter. Hosts retain pane entities and own filesystem IO.
//! Layout persistence contains stable pane IDs only, never snapshots or secrets.
use crate::design::ui_scaled_px as px;
use crate::empty_pane::{EmptyPane, EmptyPaneAction, EmptyPaneEvent};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Icon, Sizable, WindowExt as _,
    button::{Button, ButtonVariants as _}, menu::DropdownMenu as _,
};
use gpui_kit::{component::dock::*, prelude::*, *};
use monitter_lib::experimental_desktop::NativeTileTheme;
use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    rc::Rc,
};
#[cfg(feature = "performance-profiling")]
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(feature = "performance-profiling")]
static PAINTED_PANE_OBSERVATIONS: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "performance-profiling")]
static MISSING_VISIBLE_PANE_OBSERVATIONS: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "performance-profiling")]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct VisibilityPerformanceCounts {
    pub painted_pane_observations: u64,
    pub missing_visible_pane_observations: u64,
}

#[cfg(feature = "performance-profiling")]
pub(crate) fn visibility_performance_counts() -> VisibilityPerformanceCounts {
    VisibilityPerformanceCounts {
        painted_pane_observations: PAINTED_PANE_OBSERVATIONS.load(Ordering::Relaxed),
        missing_visible_pane_observations: MISSING_VISIBLE_PANE_OBSERVATIONS.load(Ordering::Relaxed),
    }
}

#[path = "split_allocation.rs"]
mod split_allocation;
#[cfg(all(test, feature = "visual-export"))]
#[path = "split_allocation_tests.rs"]
mod split_allocation_tests;
#[cfg(all(test, feature = "visual-export"))]
#[path = "pane_movement_tests.rs"]
mod pane_movement_tests;
#[cfg(all(test, feature = "visual-export"))]
#[path = "auto_size_panes_tests.rs"]
mod auto_size_panes_tests;
const PANEL_NAME: &str = "MonitterNativePane";
const VERSION: usize = 1;
#[cfg(all(test, feature = "visual-export", feature = "performance-profiling"))]
#[path = "pane_mode_restore_tests.rs"]
mod pane_mode_restore_tests;
#[cfg(all(test, feature = "visual-export"))]
#[path = "pane_cache_tests.rs"]
mod pane_cache_tests;
#[cfg(all(test, feature = "visual-export"))]
#[path = "pane_scroll_tests.rs"]
mod pane_scroll_tests;
#[cfg(all(test, feature = "visual-export"))]
#[path = "duplicate_add_tests.rs"]
mod duplicate_add_tests;
#[cfg(all(test, feature = "visual-export"))]
#[path = "pane_header_tests.rs"]
mod pane_header_tests;
#[cfg(all(test, feature = "visual-export"))]
#[path = "docking_frame_coalesce_tests.rs"]
mod docking_frame_coalesce_tests;
#[cfg(all(test, feature = "visual-export"))]
#[path = "workspace_switch_tests.rs"]
mod workspace_switch_tests;
#[cfg(all(test, feature = "visual-export"))]
#[path = "empty_session_tests.rs"]
mod empty_session_tests;

#[derive(Clone, Debug)]
pub enum PaneLifecycle {
    Active(bool),
    Visible(bool),
    Removed,
}
pub type LifecycleHook = Rc<dyn Fn(PaneLifecycle, &mut Window, &mut App)>;
/// A factory must return the already-retained pane, or construct a native pane.
/// It must not open services, start jobs, or read unbounded files synchronously.
pub type PaneFactory = Rc<dyn Fn(&str, &mut Window, &mut App) -> Option<PaneSpec>>;
pub type LazyPaneFactory = Rc<dyn Fn(&mut App) -> Result<PaneSpec, String>>;

pub struct PaneSpec {
    pub id: String,
    pub title: SharedString,
    pub content: AnyView,
    pub focus: FocusHandle,
    pub lifecycle: Option<LifecycleHook>,
    pub closable: bool,
}

const EMPTY_TAB_PREFIX: &str = "empty:";
#[derive(Clone, Debug)]
pub enum WorkspaceEvent {
    LayoutChanged,
    AutoSizePanesChanged(bool),
    VisibilityChanged {
        id: String,
        visible: bool,
    },
    Removed {
        id: String,
    },
    EmptyAction {
        pane_id: String,
        action: EmptyPaneAction,
    },
    EmptyWorkspaceAction(EmptyPaneAction),
    EmptySession { pane_id: String, task_id: String },
    EmptyClose { pane_id: String },
}
#[derive(Default)]
struct PaintState {
    painted: HashSet<String>,
    visible: HashSet<String>,
    shown: bool,
    frame_callback_pending: bool,
    removed: Vec<(String, bool)>,
}
struct LazyPanePlaceholder { label: String }
impl Render for LazyPanePlaceholder {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().flex().items_center().justify_center().child(self.label.clone())
    }
}
struct LazyPaneError { error: String, retry: Rc<dyn Fn(&mut App)> }
impl Render for LazyPaneError {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let retry = self.retry.clone();
        div().size_full().flex().flex_col().items_center().justify_center().gap_2()
            .child(self.error.clone())
            .child(div().id("retry-saved-pane").cursor_pointer().child("Retry")
                .on_click(move |_, _, cx| retry(cx)))
    }
}
struct NativePanel {
    spec: PaneSpec,
    lazy_factory: Option<LazyPaneFactory>,
    saved_watch_mode: Option<bool>,
    active: bool,
    paint: Rc<RefCell<PaintState>>,
    removed: bool,
    retaining_layout: Rc<Cell<bool>>,
    appearance: Rc<Cell<Option<crate::pane_appearance::PaneAppearance>>>,
    auto_hide_tabs: Rc<Cell<bool>>,
    background: Option<Entity<crate::pane_appearance::PaneBackground>>,
    working: bool,
    _activity_subscription: Option<Subscription>,
    dock_group: Option<WeakEntity<TabGroup>>,
}
impl NativePanel {
    fn install_lazy_spec(&mut self, spec: PaneSpec, window: &mut Window, cx: &mut Context<Self>) {
        if spec.id != self.spec.id || self.removed { return; }
        let focused = self.spec.focus.contains_focused(window, cx);
        self.spec = spec;
        self.wire_content_group(cx);
        if let Some(watch_mode) = self.saved_watch_mode {
            if let Ok(chat) = self.spec.content.clone().downcast::<crate::chat::ChatPane>() {
                chat.update(cx, |chat, cx| chat.restore_watch_mode(watch_mode, cx));
            }
        }
        let chat = self.spec.content.clone().downcast::<crate::chat::ChatPane>().ok();
        let browser = self.spec.content.clone().downcast::<crate::browser::BrowserTab>().ok();
        self.working = chat.as_ref().is_some_and(|chat| chat.read(cx).is_working())
            || browser.as_ref().is_some_and(|browser| browser.read(cx).is_loading());
        self._activity_subscription = if let Some(chat) = chat {
            Some(cx.observe(&chat, |panel, chat, cx| {
                let working = chat.read(cx).is_working();
                panel.set_working(working, cx);
            }))
        } else { browser.map(|browser| cx.observe(&browser, |panel, browser, cx| {
            let working = browser.read(cx).is_loading();
            panel.set_working(working, cx);
        })) };
        if let Some(hook) = &self.spec.lifecycle {
            let visible = self.paint.borrow().visible.contains(&self.spec.id);
            hook(PaneLifecycle::Visible(visible), window, cx);
            hook(PaneLifecycle::Active(self.active), window, cx);
        }
        if focused { window.focus(&self.spec.focus, cx); }
        if let Some(group) = self.dock_group.as_ref().and_then(|group| group.upgrade()) {
            group.update(cx, |_, cx| cx.notify());
        }
        cx.notify();
    }
    fn wire_content_group(&mut self, cx: &mut Context<Self>) {
        let group = self.dock_group.clone();
        if let Ok(ideas) = self.spec.content.clone().downcast::<crate::ideas::IdeasPane>() {
            ideas.update(cx, |ideas, cx| ideas.set_dock_group(group.clone(), cx));
        }
        if let Ok(chat) = self.spec.content.clone().downcast::<crate::chat::ChatPane>() {
            chat.update(cx, |chat, cx| chat.set_dock_group(group.clone(), cx));
        }
        if let Ok(browser) = self.spec.content.clone().downcast::<crate::browser::BrowserTab>() {
            browser.update(cx, |browser, cx| browser.set_dock_group(group, cx));
        }
    }
    fn is_git_pane(&self) -> bool {
        self.spec.id.starts_with("git:") || self.spec.id == "route:repository"
    }
    fn git_header_visible(&self) -> bool {
        self.is_git_pane() && (self.auto_hide_tabs.get()
            || self.appearance.get().is_some_and(|appearance| appearance.tile_theme == NativeTileTheme::Tern))
    }
    fn git_header(&self) -> Option<AnyElement> {
        self.git_header_visible().then(|| self.dock_group.clone()).flatten()
            .map(|group| GitPaneHeader { group }.into_any_element())
    }
    fn set_working(&mut self, working: bool, cx: &mut Context<Self>) {
        if self.working == working { return; }
        self.working = working;
        if let Some(group) = self.dock_group.as_ref().and_then(|group| group.upgrade()) {
            group.update(cx, |_, cx| cx.notify());
        }
        cx.notify();
    }
}
/// Resolve the owning group only after NativePanel::render has returned.
/// Dock helpers read the active panel's policy, so doing this while that same
/// panel is mutably borrowed would re-enter its entity during render.
#[derive(IntoElement)]
struct GitPaneHeader {
    group: WeakEntity<TabGroup>,
}
impl RenderOnce for GitPaneHeader {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let Some(group) = self.group.upgrade() else { return div().into_any_element(); };
        let context = group.read(cx).context(cx);
        let menu_group = group.downgrade();
        let identity = pane_drag_region(
            div().flex_1().min_w_0().h_full().flex().items_center().gap(px(6.))
                .child(Icon::new(IconName::GitBranch).with_size(px(13.)).flex_shrink_0())
                .child(div().min_w_0().truncate().child("Git Changes")),
            &context, cx,
        );
        div().id("git-pane-header").debug_selector(|| "git-pane-header".into())
            .w_full().h(px(36.)).flex_shrink_0().min_w_0().flex().items_center()
            .px(px(8.)).gap(px(4.)).text_size(px(11.)).text_color(cx.theme().foreground)
            .border_b_1().border_color(cx.theme().border)
            .border_blend_mode(crate::surface_blend::separator_mode())
            .child(identity)
            .child(div().flex_shrink_0().debug_selector(|| "git-pane-move".into())
                .child(pane_move_control(&context, cx)))
            .children(pane_zoom_control(&context, cx).map(|button| button
                .w(px(24.)).h(px(24.)).p_0().debug_selector(|| "git-pane-zoom".into())))
            .child(Button::new("git-pane-menu").compact().small().ghost()
                .w(px(24.)).h(px(24.)).p_0().icon(IconName::Ellipsis)
                .tooltip("Pane menu").accessibility_label("Git changes pane menu")
                .debug_selector(|| "git-pane-menu".into())
                .dropdown_menu(move |menu, window, cx| {
                    let Some(group) = menu_group.upgrade() else { return menu; };
                    pane_menu(menu, &group.read(cx).context(cx), window, cx)
                }).anchor(Anchor::TopRight))
            .child(pane_close_control(&context, cx).w(px(24.)).h(px(24.)).p_0()
                .debug_selector(|| "git-pane-close".into()))
            .into_any_element()
    }
}
impl EventEmitter<PanelEvent> for NativePanel {}
impl Focusable for NativePanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.spec.focus.clone()
    }
}
impl BasePanel for NativePanel {
    fn panel_name(&self) -> &'static str {
        PANEL_NAME
    }
    fn closable(&self, _: &App) -> bool {
        self.spec.closable
    }
    fn set_active(&mut self, active: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.active = active;
        if let Some(hook) = &self.spec.lifecycle {
            hook(PaneLifecycle::Active(active), window, cx);
        }
    }
    fn set_zoomed(&mut self, _: bool, _: &mut Window, cx: &mut Context<Self>) {
        // Re-read the owning group's zoom state for the retained content header.
        if self.git_header_visible() { cx.notify(); }
    }
    fn on_removed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dock_group = None;
        if let Ok(ideas) = self.spec.content.clone().downcast::<crate::ideas::IdeasPane>() {
            ideas.update(cx, |ideas, cx| ideas.set_dock_group(None, cx));
        }
        crate::native_pane_highlight::observe_candidate(&self.spec.id, false, window);
        if let Some(background) = &self.background { background.update(cx, |background, cx| background.set_visible(false, cx)); }
        if self.retaining_layout.get() {
            if let Ok(chat) = self.spec.content.clone().downcast::<crate::chat::ChatPane>() {
                chat.update(cx, |chat, cx| chat.set_dock_group(None, cx));
            }
            if let Ok(browser) = self.spec.content.clone().downcast::<crate::browser::BrowserTab>() {
                browser.update(cx, |browser, cx| browser.set_dock_group(None, cx));
            }
            if let Some(hook) = &self.spec.lifecycle { hook(PaneLifecycle::Visible(false), window, cx); }
            return;
        }
        self.removed = true;
        if let Ok(chat) = self.spec.content.clone().downcast::<crate::chat::ChatPane>() {
            chat.update(cx, |chat, cx| chat.set_dock_group(None, cx));
        }
        if let Ok(browser) = self.spec.content.clone().downcast::<crate::browser::BrowserTab>() {
            browser.update(cx, |browser, cx| browser.set_dock_group(None, cx));
        }
        let mut paint = self.paint.borrow_mut();
        paint.painted.remove(&self.spec.id);
        let was_visible = paint.visible.remove(&self.spec.id);
        paint.removed.push((self.spec.id.clone(), was_visible));
        drop(paint);
        if let Some(hook) = &self.spec.lifecycle {
            hook(PaneLifecycle::Visible(false), window, cx);
            hook(PaneLifecycle::Removed, window, cx);
        }
    }
    fn on_added_to(&mut self, group: WeakEntity<TabGroup>, _: &mut Window, cx: &mut Context<Self>) {
        self.removed = false;
        self.dock_group = Some(group.clone());
        self.wire_content_group(cx);
    }
    fn dump(&self, cx: &App) -> PanelState {
        let mut info = json!({"id": self.spec.id});
        if let Ok(chat) = self.spec.content.clone().downcast::<crate::chat::ChatPane>() {
            info["watchMode"] = json!(chat.read(cx).persisted_watch_mode());
        } else if let Some(watch_mode) = self.saved_watch_mode {
            info["watchMode"] = json!(watch_mode);
        }
        PanelState {
            panel_name: PANEL_NAME.into(),
            children: vec![],
            info: PanelInfo::panel(info),
        }
    }
}
/// Pane identity is stable across title edits and restored layouts.
fn pane_type_icon(id: &str) -> IconName {
    if id == "route:app-resource-usage" { return IconName::Activity; }
    match id.split(':').next().unwrap_or(id) {
        "chat" | "draft" => IconName::MessageSquare,
        "terminal" => IconName::Terminal,
        "browser" => IconName::Globe,
        "empty" => IconName::Plus,
        "mail" => IconName::Mail,
        "projects" => IconName::Folder,
        "agents" => IconName::Bot,
        "activity" => IconName::Activity,
        "settings" => IconName::Settings,
        _ => IconName::PanelLeft,
    }
}
impl Panel for NativePanel {
    fn separator_blend_mode(&self, _: &App) -> u32 {
        crate::surface_blend::current(crate::surface_blend::Area::Separators).shader()
    }
    fn tab_surface_blend_mode(&self, _: &App) -> u32 {
        if self.spec.id.starts_with("chat:") || self.spec.id.starts_with("draft:") {
            crate::surface_blend::effective_shader(crate::surface_blend::Area::Pane)
        } else { 0 }
    }
    fn inner_padding(&self, _: &App) -> bool { false }
    fn toolbar_in_content(&self, _: &App) -> bool {
        self.spec.id.starts_with("chat:") || self.spec.id.starts_with("draft:") || self.spec.id.starts_with(EMPTY_TAB_PREFIX)
            || self.spec.id == "route:ideas" || self.git_header_visible()
            || self.spec.content.clone().downcast::<crate::browser::BrowserTab>().is_ok()
    }
    fn close_button_visible(&self, cx: &App) -> bool {
        self.spec.content.clone().downcast::<crate::browser::BrowserTab>()
            .map_or(true, |browser| !browser.read(cx).has_native_close_control())
    }
    fn pane_overlay(&mut self, tab: Rc<Cell<Option<gpui::Bounds<Pixels>>>>, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let appearance = self.appearance.get();
        let focused = appearance.as_ref().is_some_and(|appearance| appearance.show_active_border)
            && pane_visual_active(&self.spec.focus, window, cx);
        let working = self.working;
        let candidate = (focused || working) && !crate::preferences_modal::workspace_blocked(cx);
        crate::native_pane_highlight::observe_candidate(&self.spec.id, candidate, window);
        let appearance = appearance.filter(|_| candidate)?;
        let border_width = px(appearance.active_border_width);
        let pane_id = self.spec.id.clone();
        let mut color = rgb(appearance.active_border_color);
        if !focused {
            color = cx.theme().primary.into();
            color.a = if working { 0.38 } else { 0. };
        }
        let mut glow_color: Rgba = cx.theme().primary.into();
        glow_color.a = if working { 0.44 } else { 0. };
        if appearance.tile_theme == NativeTileTheme::Tern {
            let mut color = rgb(appearance.resolved_active_border_color());
            if focused { color.a = appearance.resolved_active_border_opacity(); }
            else {
                color = cx.theme().primary.into();
                color.a = if working { 0.38 } else { 0. };
            }
            return Some(canvas(|_, _, _| (), move |bounds, _, window, cx| {
                if crate::preferences_modal::workspace_blocked(cx) { return; }
                let contour = crate::pane_appearance::rounded_contour(bounds, px(crate::pane_appearance::PANE_RADIUS), border_width);
                // Native and GPUI strokes must not stack their alpha.
                if !crate::native_pane_highlight::paint(
                    &pane_id,
                    bounds,
                    color,
                    glow_color,
                    border_width,
                    &contour,
                    window,
                ) {
                    if working { crate::pane_highlight_geometry::paint_outward_glow(&contour, glow_color, window); }
                    let mut outline = gpui::outline(bounds, color, gpui::BorderStyle::Solid)
                        .corner_radii(px(crate::pane_appearance::PANE_RADIUS));
                    outline.border_widths = border_width.into();
                    window.paint_quad(outline);
                }
            }).absolute().inset_0().into_any_element());
        }
        Some(canvas(|_, _, _| (), move |bounds, _, window, cx| {
            if crate::preferences_modal::workspace_blocked(cx) { return; }
            let contour = crate::pane_highlight_geometry::contour(bounds, tab.get(), border_width);
            if !crate::native_pane_highlight::paint(
                &pane_id,
                bounds,
                color,
                glow_color,
                border_width,
                &contour,
                window,
            ) {
                if working { crate::pane_highlight_geometry::paint_outward_glow(&contour, glow_color, window); }
                let mut path = gpui::PathBuilder::stroke(border_width);
                if let Some(first) = contour.first() {
                    path.move_to(*first);
                    for point in &contour[1..] { path.line_to(*point); }
                    path.close();
                    if let Ok(path) = path.build() { window.paint_path(path, color); }
                }
            }
        }).absolute().inset_0().into_any_element())
    }

    fn tab_opacity(&self, window: &Window, cx: &App) -> f32 {
        self.appearance.get().filter(|appearance| appearance.dim_inactive
            && !pane_visual_active(&self.spec.focus, window, cx))
            .map_or(1., |appearance| appearance.inactive_opacity)
    }
    fn title_bar(&self, _: &App) -> bool {
        !self.auto_hide_tabs.get()
    }
    fn tab_name(&self, _: &App) -> Option<SharedString> {
        Some(self.spec.title.clone())
    }
    fn title_style(&self, cx: &App) -> Option<TitleStyle> {
        if self.spec.id == "browser" {
            return Some(TitleStyle {
                background: *cx.theme().tokens.tab_active,
                foreground: *cx.theme().tokens.tab_active_foreground,
            });
        }
        if self.spec.id != "terminal" && !self.spec.id.starts_with("terminal:") { return None; }
        let terminal = self.spec.content.clone().downcast::<crate::terminal::TerminalPane>().ok()?;
        let terminal = terminal.read(cx);
        Some(TitleStyle {
            background: *cx.theme().tokens.tab_active,
            foreground: terminal.foreground_surface().into(),
        })
    }
    fn tab_background(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let task = self.spec.id.strip_prefix("chat:")?;
        crate::activity_effects::for_task_in_window(task, "tab", crate::activity_sparkles::SparkleLayout::Tab, true, window, cx)
            .map(|effect| effect.into_any_element())
    }
    fn tab_bar_background(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let task = self.spec.id.strip_prefix("chat:")?;
        crate::activity_effects::for_task_in_window(task, "dock-wash", crate::activity_sparkles::SparkleLayout::DockWash, false, window, cx)
            .map(|effect| effect.into_any_element())
    }
    fn title(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let icon = Icon::new(pane_type_icon(&self.spec.id)).with_size(px(13.)).flex_shrink_0();
        let title = if let Some(task) = self.spec.id.strip_prefix("chat:").filter(|task| crate::activity_effects::is_naming(task, cx)) {
            crate::naming_title::render(task, "tab", self.spec.title.clone(), self.appearance.get().map(|appearance| appearance.active_border_color).unwrap_or(0x559fff), window, cx)
        } else {
            div().child(self.spec.title.clone()).into_any_element()
        };
        div()
            .w_full()
            .flex()
            .items_center()
            .min_w_0()
            .gap(px(6.))
            .child(icon)
            .child(div().flex_1().min_w_0().truncate().child(title))
    }
}
pub(crate) fn pane_visual_active(focus: &FocusHandle, window: &Window, cx: &App) -> bool {
    focus.contains_focused(window, cx)
        || gpui_kit::component::surface_backdrop::popup_focus_origin(window, cx)
            .is_some_and(|origin| focus.contains(&origin, window))
}

impl Render for NativePanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(factory) = self.lazy_factory.take() {
            let panel = cx.weak_entity();
            let handle = window.window_handle();
            // Construct outside both panel and Desktop entity leases. Hidden
            // tabs keep only their identity/header until they actually render.
            cx.defer(move |cx| {
                if !panel.read_with(cx, |panel, _| !panel.removed).unwrap_or(false) { return; }
                let result = factory(cx);
                let _ = handle.update(cx, |_, window, cx| {
                    let _ = panel.update(cx, |panel, cx| match result {
                        Ok(spec) => panel.install_lazy_spec(spec, window, cx),
                        Err(error) => {
                            let retry = factory.clone();
                            let owner = cx.weak_entity();
                            let placeholder = cx.new(|_| LazyPaneError { error, retry: Rc::new(move |cx| {
                                let _ = owner.update(cx, |panel, cx| { panel.lazy_factory = Some(retry.clone()); cx.notify(); });
                            }) });
                            panel.spec.content = placeholder.into();
                            cx.notify();
                        }
                    });
                });
            });
        }
        let appearance = self.appearance.get();
        let active = pane_visual_active(&self.spec.focus, window, cx);
        let background = self.background.get_or_insert_with(|| cx.new(|_| crate::pane_appearance::PaneBackground::new())).clone();
        if let Some(appearance) = appearance {
            let browser = self.spec.id.starts_with("browser:") || self.spec.id == "browser" || self.spec.id == "route:browser";
            let fill = if browser {
                (*cx.theme().tokens.tab_active).into()
            } else {
                let tint = if self.spec.id == "terminal" || self.spec.id.starts_with("terminal:") || self.spec.id == "route:terminal" { 0.25 } else { 0.15 };
                crate::surface_blend::tinted_pane_background(appearance.paper_color, appearance.dark, tint)
            };
            // A retained pane also supports plain GPUI windows without the
            // component Root (e.g. native fixture and embedded pane hosts).
            let has_dialog = window.root::<gpui_kit::base::Root>().flatten().is_some()
                && window.has_active_dialog(cx);
            background.update(cx, |background, cx| {
                background.set_appearance(appearance, fill, !browser, active, cx);
                background.set_visible(self.paint.borrow().shown && !has_dialog, cx);
            });
        }
        let hover_background = background.clone();
        let focus = self.spec.focus.clone();
        let paint = self.paint.clone();
        let id = self.spec.id.clone();
        // Decorative animation is a sibling of cached chat content. Its frame
        // notifications must never invalidate the transcript or composer.
        let mut decorations = Vec::new();
        let mut working_bubble = None;
        let mut chat_header_hover = None;
        if let Some(task) = self.spec.id.strip_prefix("chat:") {
            if let Ok(chat) = self.spec.content.clone().downcast::<crate::chat::ChatPane>() {
                let geometry = chat.read(cx).decoration_bounds.clone();
                working_bubble = Some(chat.read(cx).working_bubble.clone());
                chat_header_hover = Some((chat.clone(), chat.read(cx).pane_header_height()));
                for (surface, layout, mirror, bounds) in [
                    ("pane", crate::activity_sparkles::SparkleLayout::Pane, false, geometry.pane),
                    ("header", crate::activity_sparkles::SparkleLayout::Header, true, geometry.header),
                ] {
                    if let Some(field) = crate::activity_effects::for_task_in_window(task, surface, layout, mirror, window, cx) {
                        field.update(cx, |field, _| {
                            field.set_paint_bounds(bounds);
                            field.set_active_pane(active);
                        });
                        decorations.push(field);
                    }
                }
            }
        }
        let content_opacity = appearance
            .filter(|appearance| !active && appearance.dim_inactive)
            .map_or(1., |appearance| appearance.inactive_opacity);
        let content = self.spec.content.clone().cached(StyleRefinement::default().size_full()).into_any_element();
        let content = if let Some(header) = self.git_header() {
            div().size_full().min_h_0().flex().flex_col().child(header)
                .child(div().flex_1().min_h_0().w_full().relative().child(content)).into_any_element()
        } else { content };
        let chat_header_sensor = chat_header_hover.map(|(chat, height)| {
            div()
                .id("chat-header-hover-sensor")
                .debug_selector(|| "chat-header-hover-sensor".into())
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .h(height)
                .on_hover(move |hovered, _, cx| {
                    chat.update(cx, |chat, cx| {
                        chat.set_header_controls_hovered(*hovered, cx)
                    });
                })
        });
        // Only paint proves effective visibility. Active slots can be hidden,
        // collapsed, displaced by zoom, or outside the host's current route.
        div()
            .id(SharedString::from(format!("pane-emphasis-{}", self.spec.id)))
            .size_full()
            .relative()
            .track_focus(&self.spec.focus)
            .when_some(appearance, |this, appearance| {
                this.when(appearance.focus_follows_mouse, |this| {
                    this.on_mouse_move(move |event, window, cx| {
                        if event.pressed_button.is_none()
                            && !window.has_active_dialog(cx)
                            && !focus.contains_focused(window, cx)
                        {
                            window.focus(&focus, cx);
                        }
                    })
                })
            })
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| {
                        if bounds.size.width > px(0.) && bounds.size.height > px(0.) {
                            window.observe_paint(move || {
                                paint.borrow_mut().painted.insert(id.clone());
                            });
                        }
                    },
                )
                .absolute()
                .size_full(),
            )
            .on_hover(move |hovered, _, cx| hover_background.update(cx, |background, cx| background.set_hovered(*hovered, cx)))
            .child(div().absolute().inset_0().child(background))
            .child(pane_content_layers(content, decorations, content_opacity)
                .when_some(working_bubble, |this, bubble| this.child(
                    div().absolute().inset_0().opacity(content_opacity).child(bubble))))
            .when_some(chat_header_sensor, |this, sensor| this.child(sensor))
    }
}
// Activity fields retain their own focus/lifecycle opacity independently of the
// user's inactive-content setting, while remaining outside the content cache.
fn pane_content_layers(
    content: AnyElement,
    decorations: impl IntoIterator<Item = Entity<crate::activity_sparkles::SparkleField>>,
    content_opacity: f32,
) -> Div {
    div().size_full().relative()
        .child(div().size_full().opacity(content_opacity).child(content))
        .children(decorations)
}

#[derive(Clone)]
struct RegistryEntry {
    panels: Rc<RefCell<HashMap<String, Entity<NativePanel>>>>,
}
#[derive(Default)]
struct NativeRegistry(HashMap<EntityId, RegistryEntry>);
impl Global for NativeRegistry {}

pub struct DockWorkspace {
    area: Entity<DockArea>,
    skin: Rc<DockSkin>,
    panels: Rc<RefCell<HashMap<String, Entity<NativePanel>>>>,
    paint: Rc<RefCell<PaintState>>,
    factory: PaneFactory,
    appearance: Rc<Cell<Option<crate::pane_appearance::PaneAppearance>>>,
    auto_hide_tabs: Rc<Cell<bool>>,
    retaining_layout: Rc<Cell<bool>>,
    empty_panes: HashMap<String, Entity<EmptyPane>>,
    empty_subscriptions: HashMap<String, Subscription>,
    _subscriptions: Vec<Subscription>,
    equalize_motion: Option<(std::time::Instant, Vec<DockSplitSnapshot>)>,
    equalize_frame_pending: bool,
    auto_size_panes: bool,
    auto_size_pending: bool,
    auto_size_generation: u64,
    restoring_layout: bool,
    restore_generation: u64,
    live_group_count: usize,
    live_panel_count: usize,
    host_generation: u64,
}
impl EventEmitter<WorkspaceEvent> for DockWorkspace {}
impl DockWorkspace {
    pub fn new(
        id: impl Into<SharedString>,
        factory: PaneFactory,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (area, skin) = DockSkin::dock_area(id, Some(VERSION), window, cx);
        skin.set_close_button_visible(true, cx);
        let panels = Rc::new(RefCell::new(HashMap::new()));
        let paint = Rc::new(RefCell::new(PaintState {
            shown: true,
            ..Default::default()
        }));
        if cx.try_global::<NativeRegistry>().is_none() {
            cx.set_global(NativeRegistry::default());
        }
        cx.global_mut::<NativeRegistry>().0.insert(
            area.entity_id(),
            RegistryEntry {
                panels: panels.clone(),
            },
        );
        register_panel(cx, PANEL_NAME, |context, _, cx| {
            let key = context.dock_area().entity_id();
            let id = match context.info() {
                PanelInfo::Panel(value) => value["id"].as_str().unwrap_or_default(),
                _ => "",
            };
            // restore() resolves and validates all native IDs before load().
            let panel = cx
                .global::<NativeRegistry>()
                .0
                .get(&key)
                .and_then(|entry| entry.panels.borrow().get(id).cloned())
                .expect("native restore preflight registered pane");
            // Apply every saved leaf even when its content entity was retained.
            // Older layouts have no mode and restore the original Chat default.
            let watch_mode = match context.info() {
                PanelInfo::Panel(value) => value["watchMode"].as_bool().unwrap_or(false),
                _ => false,
            };
            panel.update(cx, |panel, _| {
                if panel.spec.id.starts_with("chat:") { panel.saved_watch_mode = Some(watch_mode); }
            });
            let content = panel.read(cx).spec.content.clone();
            if let Ok(chat) = content.downcast::<crate::chat::ChatPane>() {
                chat.update(cx, |chat, cx| chat.restore_watch_mode(watch_mode, cx));
            }
            panel_handle(panel)
        });
        let changed = cx.subscribe_in(&area, window, |workspace, _, event, window, cx| {
            match event {
                DockEvent::ManuallyResized => workspace.set_auto_size_panes(false, window, cx),
                DockEvent::LayoutChanged => {
                    let count = workspace.visible_group_count(cx);
                    let panels = workspace.visible_panel_count(cx);
                    let panes_changed = count != workspace.live_group_count || panels != workspace.live_panel_count;
                    workspace.live_group_count = count;
                    workspace.live_panel_count = panels;
                    if panes_changed && !workspace.restoring_layout { workspace.schedule_auto_size(window, cx); }
                    cx.emit(WorkspaceEvent::LayoutChanged);
                    cx.notify();
                }
                _ => {}
            }
        });
        let key = area.entity_id();
        let released = cx.on_release(move |_, cx| {
            cx.global_mut::<NativeRegistry>().0.remove(&key);
        });
        Self {
            area,
            skin,
            panels,
            paint,
            factory,
            appearance: Rc::new(Cell::new(None)),
            auto_hide_tabs: Rc::new(Cell::new(false)),
            retaining_layout: Rc::new(Cell::new(false)),
            empty_panes: HashMap::new(),
            empty_subscriptions: HashMap::new(),
            _subscriptions: vec![changed, released],
            equalize_motion: None,
            equalize_frame_pending: false,
            auto_size_panes: true,
            auto_size_pending: false,
            auto_size_generation: 0,
            restoring_layout: false,
            restore_generation: 0,
            live_group_count: 0,
            live_panel_count: 0,
            host_generation: 0,
        }
    }
    pub fn auto_size_panes(&self) -> bool { self.auto_size_panes }
    pub fn set_auto_size_panes(&mut self, enabled: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.auto_size_panes == enabled { return; }
        self.auto_size_panes = enabled;
        if enabled { self.schedule_auto_size(window, cx); }
        else { self.cancel_layout_motion(); }
        cx.emit(WorkspaceEvent::AutoSizePanesChanged(enabled));
        cx.notify();
    }
    fn visible_group_count(&self, cx: &App) -> usize {
        let area = self.area.read(cx);
        [DockPlacement::Center, DockPlacement::Left, DockPlacement::Right, DockPlacement::Bottom]
            .into_iter().filter(|placement| *placement == DockPlacement::Center || area.is_dock_open(*placement))
            .filter_map(|placement| area.layout(placement))
            .flat_map(|tree| groups_in(tree.root()))
            .filter(|group| !group.panels.is_empty()).count()
    }
    fn visible_panel_count(&self, cx: &App) -> usize {
        let area = self.area.read(cx);
        [DockPlacement::Center, DockPlacement::Left, DockPlacement::Right, DockPlacement::Bottom]
            .into_iter().filter(|placement| *placement == DockPlacement::Center || area.is_dock_open(*placement))
            .filter_map(|placement| area.layout(placement))
            .map(|tree| tree.panels().count()).sum()
    }
    fn schedule_auto_size(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.auto_size_panes || self.auto_size_pending || self.restoring_layout { return; }
        self.auto_size_pending = true;
        self.auto_size_generation = self.auto_size_generation.wrapping_add(1);
        let generation = self.auto_size_generation;
        let weak = cx.weak_entity();
        // Sample after all inserts/moves and the first layout pass, never an
        // intermediate tree while a new native pane is replacing its landing.
        window.on_next_frame(move |window, cx| {
            let _ = weak.update(cx, |workspace, cx| {
                if !workspace.auto_size_pending || workspace.auto_size_generation != generation { return; }
                workspace.auto_size_pending = false;
                if workspace.auto_size_panes { workspace.equalize_workspace(window, cx); }
            });
        });
        cx.notify();
    }
    fn cancel_layout_motion(&mut self) {
        self.auto_size_pending = false;
        self.auto_size_generation = self.auto_size_generation.wrapping_add(1);
        self.equalize_motion = None;
    }
    /// Keep every tab in the current saved layout while releasing cache-only
    /// panes from previously selected named layouts. Content/session owners
    /// retain those views independently; this never invokes removal lifecycle.
    pub fn retain_displayed_panels(&mut self, cx: &mut Context<Self>) -> Result<(), String> {
        let saved = self.serialize(cx)?;
        let ids: HashSet<String> = Self::saved_ids(&saved)?.into_iter().collect();
        // Preflight before dropping anything: every saved ID must still point
        // at the exact retained panel registered with this live DockArea.
        {
            let panels = self.panels.borrow();
            let area = self.area.read(cx);
            for id in &ids {
                let panel = panels
                    .get(id)
                    .ok_or_else(|| format!("Layout pane {id} is not retained."))?;
                let key = PanelId::from(panel.entity_id());
                if panel.read(cx).removed || area.panel(key).is_none() {
                    return Err(format!("Layout pane {id} is unavailable."));
                }
            }
        }
        self.panels.borrow_mut().retain(|id, _| ids.contains(id));
        self.empty_panes.retain(|id, _| ids.contains(id));
        self.empty_subscriptions.retain(|id, _| ids.contains(id));
        let mut paint = self.paint.borrow_mut();
        paint.painted.retain(|id| ids.contains(id));
        paint.visible.retain(|id| ids.contains(id));
        // A prior layout's queued cleanup must not destroy its content after
        // this dock has been moved and a different host owns that workspace.
        paint.removed.retain(|(id, _)| ids.contains(id));
        Ok(())
    }

    /// Stable retained contents, including inactive panes that are absent from
    /// the currently displayed layout. Content owners can rehost their private
    /// children without rebuilding this dock or replacing a native session.
    pub fn retained_content_views(&self, cx: &App) -> Vec<(String, AnyView)> {
        let mut contents: Vec<_> = self
            .panels
            .borrow()
            .iter()
            .map(|(id, panel)| (id.clone(), panel.read(cx).spec.content.clone()))
            .collect();
        contents.sort_by(|a, b| a.0.cmp(&b.0));
        contents
    }
    pub(crate) fn is_pane_visible(&self,id:&str)->bool {self.paint.borrow().visible.contains(id)}
    pub fn retained_browser_ids(&self) -> Vec<u64> {
        let mut ids: Vec<_> = self
            .panels
            .borrow()
            .keys()
            .filter_map(|id| crate::browser_workspace::parse_browser_pane_id(id))
            .collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    }
    /// The dock's retained children, excluding this DockWorkspace entity itself.
    /// Private children of arbitrary content views remain their owner's concern.
    pub fn retained_entity_ids(&self, cx: &App) -> Vec<EntityId> {
        let mut ids = self.area.read(cx).retained_entity_ids();
        for panel in self.panels.borrow().values() {
            ids.push(panel.entity_id());
            let panel = panel.read(cx);
            ids.push(panel.spec.content.entity_id());
            if let Some(background) = &panel.background {
                ids.push(background.entity_id());
            }
        }
        for pane in self.empty_panes.values() {
            ids.push(pane.entity_id());
            ids.extend(pane.read(cx).child_entity_ids());
        }
        ids.sort_by_key(|id| id.as_u64());
        ids.dedup();
        ids
    }
    /// Move the intact retained dock to a new native host. The caller must stop
    /// mounting it in its previous host first. No panel/group/lifecycle is reset.
    pub fn rehost_to_window(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let mut ids = self.retained_entity_ids(cx);
        ids.push(cx.entity_id());
        for id in ids {
            cx.rehost_entity_window(id, window)
                .map_err(|error| format!("Cannot rehost dock entity {}: {error}", id.as_u64()))?;
        }
        // on_next_frame closures belong to the old native window, independent
        // of entity callback routing. Invalidate those probes/animations; the
        // first destination render schedules a fresh visibility probe.
        self.host_generation = self.host_generation.wrapping_add(1);
        self.cancel_layout_motion();
        self.equalize_frame_pending = false;
        let mut paint = self.paint.borrow_mut();
        paint.frame_callback_pending = false;
        paint.painted.clear();
        drop(paint);
        self.area.update(cx, |_, cx| cx.notify());
        cx.notify();
        Ok(())
    }

    pub fn area(&self) -> Entity<DockArea> {
        self.area.clone()
    }
    pub fn refresh_activity(&self, cx: &mut App) {
        for panel in self.panels.borrow().values() {
            panel.update(cx, |_, cx| cx.notify());
        }
        self.area.update(cx, |_, cx| cx.notify());
    }
    pub fn set_close_buttons_visible(&self, visible: bool, cx: &mut App) {
        self.skin.set_close_button_visible(visible, cx);
    }
    pub fn set_pane_appearance(
        &self,
        settings: &Value,
        palette: crate::design::Palette,
        cx: &mut App,
    ) {
        let next = Some(crate::pane_appearance::PaneAppearance::from_settings(
            settings, palette,
        ));
        let appearance = next.expect("pane appearance constructed above");
        self.skin.set_tile_style(match appearance.tile_theme {
            NativeTileTheme::Monitter => DockTileStyle::Contiguous,
            NativeTileTheme::Tern => DockTileStyle::Rounded {
                gutter: px(crate::design::tern_spacing(settings).gutter),
                radius: px(crate::pane_appearance::PANE_RADIUS),
                // Expose the same native window finish as the workspace bar.
                backdrop: rgba(0x00000000).into(),
                border_color: crate::design::tile_border_color(palette).into(),
            },
        }, cx);
        let previous = self.appearance.replace(next);
        if previous != next {
            let theme_changed = previous.is_none_or(|old| old.tile_theme != appearance.tile_theme);
            for panel in self.panels.borrow().values() {
                panel.update(cx, |panel, cx| {
                    if theme_changed {
                        // Retained content keeps its entity and layout, but its
                        // cached header must repaint even when colors are unchanged.
                        App::notify(cx, panel.spec.content.entity_id());
                    }
                    cx.notify();
                });
            }
        }
    }
    pub fn set_tab_appearance(&self, settings: &Value, scale: u16, cx: &mut App) {
        let hidden = settings["autoHideTabs"].as_bool().unwrap_or(false);
        let tern = crate::design::tile_theme() == NativeTileTheme::Tern;
        let hidden_changed = self.auto_hide_tabs.replace(hidden) != hidden;
        if hidden_changed {
            for panel in self.panels.borrow().values() {
                panel.update(cx, |panel, cx| { if panel.is_git_pane() { cx.notify(); } });
            }
        }
        self.skin.set_panel_style(if tern {
            PanelStyle::Hidden
        } else if hidden {
            PanelStyle::Auto
        } else {
            PanelStyle::TabBar
        }, cx);
        let modern = settings["tabStyle"].as_str() == Some("modern");
        // Mirror Tauri's native tabbar CSS: compensate its native header height
        // for zoom, then reduce modern workspace strips by four design pixels.
        let height = if tern {
            f32::from(crate::design::ui_px(28., scale))
        } else {
            crate::design::workspace_tabbar_height_points(settings, scale)
        };
        self.skin.set_tab_bar_style(
            if tern {
                TabBarStyle::Pill
            } else if modern {
                TabBarStyle::Modern
            } else {
                TabBarStyle::Classic
            },
            gpui_kit::px(height),
            px(if tern || modern { 9. } else { 12. }),
            cx,
        );
        let interface_font_size = settings["interfaceFontSize"]
            .as_f64()
            .unwrap_or(14.)
            .clamp(8., 32.) as f32;
        self.skin.set_tab_label_geometry(
            crate::design::ui_px(210., scale),
            crate::design::ui_px(11.5 * interface_font_size / 14., scale),
            cx,
        );
    }
    fn retain(
        &mut self,
        spec: PaneSpec,
        cx: &mut Context<Self>,
    ) -> Result<Entity<NativePanel>, String> {
        if spec.id.is_empty() || spec.id.len() > 128 {
            return Err("Pane ID must contain 1–128 bytes.".into());
        }
        if let Some(panel) = self.panels.borrow().get(&spec.id).cloned() {
            return Ok(panel);
        }
        let id = spec.id.clone();
        let paint = self.paint.clone();
        let panel = cx.new(|cx: &mut Context<NativePanel>| {
            let chat = spec.content.clone().downcast::<crate::chat::ChatPane>().ok();
            let browser = spec.content.clone().downcast::<crate::browser::BrowserTab>().ok();
            let working = chat.as_ref().is_some_and(|chat| chat.read(cx).is_working())
                || browser.as_ref().is_some_and(|browser| browser.read(cx).is_loading());
            let activity_subscription = if let Some(chat) = chat {
                Some(cx.observe(&chat, |panel, chat, cx| {
                    let working = chat.read(cx).is_working();
                    panel.set_working(working, cx);
                }))
            } else { browser.map(|browser| cx.observe(&browser, |panel, browser, cx| {
                let working = browser.read(cx).is_loading();
                panel.set_working(working, cx);
            })) };
            NativePanel {
                spec, lazy_factory: None, saved_watch_mode: None, active: false, paint, removed: false,
                retaining_layout: self.retaining_layout.clone(),
                appearance: self.appearance.clone(), auto_hide_tabs: self.auto_hide_tabs.clone(),
                background: None, working, _activity_subscription: activity_subscription, dock_group: None,
            }
        });
        self.panels.borrow_mut().insert(id, panel.clone());
        Ok(panel)
    }
    /// Preflight a real retained pane without changing the displayed layout.
    pub fn register(&mut self, spec: PaneSpec, cx: &mut Context<Self>) -> Result<(), String> {
        self.retain(spec, cx).map(|_| ())
    }
    /// Retain tab identity without constructing its hidden service/UI subtree.
    pub fn register_lazy(&mut self, id: String, title: SharedString, factory: LazyPaneFactory, cx: &mut Context<Self>) -> Result<(), String> {
        if self.contains(&id) { return Ok(()); }
        let content = cx.new(|_| LazyPanePlaceholder { label: "Loading pane…".into() });
        let spec = PaneSpec { id, title, content: content.into(), focus: cx.focus_handle(), lifecycle: None, closable: true };
        let panel = self.retain(spec, cx)?;
        panel.update(cx, |panel, _| panel.lazy_factory = Some(factory));
        Ok(())
    }
    pub fn set_lazy_working(&mut self, id: &str, working: bool, cx: &mut Context<Self>) {
        if let Some(panel) = self.panels.borrow().get(id).cloned() {
            panel.update(cx, |panel, cx| {
                if panel.lazy_factory.is_some()
                    || panel.spec.content.clone().downcast::<LazyPanePlaceholder>().is_ok()
                    || panel.spec.content.clone().downcast::<LazyPaneError>().is_ok()
                { panel.set_working(working, cx); }
            });
        }
    }
    pub fn has_panes(&self) -> bool {
        !self.panels.borrow().is_empty()
    }
    fn has_layout_panes(&self, cx: &App) -> bool {
        let area = self.area.read(cx);
        [DockPlacement::Center, DockPlacement::Left, DockPlacement::Right, DockPlacement::Bottom]
            .into_iter().filter_map(|placement| area.layout(placement))
            .any(|tree| tree.panels().next().is_some())
    }
    pub fn contains(&self, id: &str) -> bool {
        self.panels.borrow().contains_key(id)
    }
    pub fn panel_key(&self, id: &str) -> Option<PanelId> {
        self.panels
            .borrow()
            .get(id)
            .map(|panel| PanelId::from(panel.entity_id()))
    }
    pub fn pane_id_for_key(&self, key: PanelId) -> Option<String> {
        self.panels
            .borrow()
            .iter()
            .find_map(|(id, panel)| (PanelId::from(panel.entity_id()) == key).then(|| id.clone()))
    }
    pub fn is_closable(&self, id: &str, cx: &App) -> bool {
        self.panels
            .borrow()
            .get(id)
            .is_some_and(|panel| panel.read(cx).spec.closable)
    }
    /// Resolve the dock pane that contains the current keyboard focus.
    pub fn focused_pane_id(&self, window: &Window, cx: &App) -> Option<String> {
        self.panels.borrow().iter().find_map(|(id, panel)| {
            panel
                .read(cx)
                .spec
                .focus
                .contains_focused(window, cx)
                .then(|| id.clone())
        })
    }
    /// Find the tab group containing the dock pane that owns keyboard focus.
    /// Create the first landing pane when the final tab group has been removed.
    pub fn add_focused_empty_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Result<String, String> {
        if let Some(group) = self.focused_group_node(window, cx) { return self.add_empty_tab(group, window, cx); }
        let occupied = {
            let area = self.area.read(cx);
            [DockPlacement::Center, DockPlacement::Left, DockPlacement::Right, DockPlacement::Bottom]
                .into_iter().filter_map(|placement| area.layout(placement))
                .any(|tree| tree.panels().next().is_some())
        };
        if occupied { return Err("Focus a workspace pane before opening a new tab.".into()); }
        let mut index = 1_u64;
        let id = loop {
            let id = format!("{EMPTY_TAB_PREFIX}{index}");
            if !self.contains(&id) { break id; }
            index += 1;
        };
        let spec = self.empty_spec(&id, window, cx)?;
        self.add(spec, DockPlacement::Center, None, window, cx)?;
        self.focus(&id, window, cx);
        Ok(id)
    }
    pub fn add_focused_empty_split(&mut self, vertical: bool, window: &mut Window, cx: &mut Context<Self>) -> Result<String, String> {
        if let Some(group) = self.focused_group_node(window, cx) { self.add_empty_split(group, vertical, window, cx) }
        else { self.add_focused_empty_tab(window, cx) }
    }
    pub fn focused_group_node(&self, window: &Window, cx: &App) -> Option<NodeId> {
        let focused_id = self.focused_pane_id(window, cx)?;
        self.group_for_pane(&focused_id, cx)
    }
    pub fn group_for_pane(&self, id: &str, cx: &App) -> Option<NodeId> {
        let focused_key = self.panel_key(id)?;
        let area = self.area().read(cx);
        for placement in [
            DockPlacement::Center,
            DockPlacement::Left,
            DockPlacement::Right,
            DockPlacement::Bottom,
        ] {
            let Some(tree) = area.layout(placement) else {
                continue;
            };
            let mut found = None;
            tree.root().walk(&mut |node| {
                if let PaneRef::Tabs { panels, .. } = node.kind()
                    && panels.contains(&focused_key)
                {
                    found = Some(node.id());
                }
            });
            if found.is_some() {
                return found;
            }
        }
        None
    }

    /// Focus the spatially nearest pane in the requested direction. Geometry is
    /// derived from the retained split tree, with unconstrained children given
    /// equal shares; this keeps navigation deterministic before first layout.
    pub fn focus_adjacent_pane(
        &mut self,
        direction: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(current_id) = self.focused_pane_id(window, cx) else {
            return false;
        };
        let Some(current_key) = self.panel_key(&current_id) else {
            return false;
        };
        let area = self.area.read(cx);
        let Some(current) = collect_workspace_frames(&area, cx)
            .into_iter()
            .find(|frame| frame.panels.contains(&current_key))
        else {
            return false;
        };
        let target = collect_workspace_frames(&area, cx)
            .into_iter()
            .filter(|frame| !frame.panels.contains(&current_key))
            .filter_map(|frame| {
                directional_score(current.rect, frame.rect, direction).map(|score| (score, frame))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, frame)| frame);
        drop(area);
        let Some(target) = target else {
            return false;
        };
        target
            .panels
            .get(target.active_ix)
            .and_then(|key| self.pane_id_for_key(*key))
            .is_some_and(|id| self.focus(&id, window, cx))
    }

    /// Focus the first/last pane or the next/previous pane in retained layout order.
    pub fn focus_relative_pane(
        &mut self,
        direction: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let frames = {
            let area = self.area.read(cx);
            collect_workspace_frames(&area, cx)
        };
        let mut ids = Vec::new();
        for frame in frames {
            let key = frame.panels.get(frame.active_ix).copied();
            if let Some(id) = key.and_then(|key| self.pane_id_for_key(key))
                && !ids.contains(&id)
            {
                ids.push(id);
            }
        }
        if ids.is_empty() {
            return false;
        }
        let current = self.focused_pane_id(window, cx);
        let target = match direction {
            "first" => ids.first(),
            "last" => ids.last(),
            "next" | "previous" => {
                let Some(index) = current
                    .as_ref()
                    .and_then(|id| ids.iter().position(|item| item == id))
                else {
                    return false;
                };
                let next = if direction == "next" {
                    (index + 1) % ids.len()
                } else {
                    (index + ids.len() - 1) % ids.len()
                };
                ids.get(next)
            }
            _ => None,
        };
        target.is_some_and(|id| self.focus(id, window, cx))
    }

    /// Cycle tabs in the focused group, crossing to the nearest horizontal
    /// neighbor at a group boundary just as the Tauri workspace does.
    pub fn focus_relative_tab(
        &mut self,
        direction: i8,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(id) = self.focused_pane_id(window, cx) else {
            return false;
        };
        let Some(key) = self.panel_key(&id) else {
            return false;
        };
        let area = self.area.read(cx);
        let group = [
            DockPlacement::Center,
            DockPlacement::Left,
            DockPlacement::Right,
            DockPlacement::Bottom,
        ]
        .into_iter()
        .filter_map(|placement| area.layout(placement))
        .find_map(|tree| {
            let node_id = tree.find_panel_node(key)?;
            let node = tree.find_node(node_id)?;
            match node.kind() {
                PaneRef::Tabs { panels, active_ix } => Some((panels.to_vec(), active_ix)),
                PaneRef::Split { .. } => None,
            }
        });
        drop(area);
        let Some((panels, active_ix)) = group else {
            return false;
        };
        let next_ix = active_ix as isize + direction.signum() as isize;
        if let Some(next) = usize::try_from(next_ix)
            .ok()
            .and_then(|ix| panels.get(ix))
            .copied()
        {
            return self
                .pane_id_for_key(next)
                .is_some_and(|id| self.focus(&id, window, cx));
        }
        if !self.focus_adjacent_pane(if direction < 0 { "left" } else { "right" }, window, cx) {
            return false;
        }
        let Some(adjacent_id) = self.focused_pane_id(window, cx) else {
            return false;
        };
        let Some(adjacent_key) = self.panel_key(&adjacent_id) else {
            return false;
        };
        let area = self.area.read(cx);
        let boundary = [
            DockPlacement::Center,
            DockPlacement::Left,
            DockPlacement::Right,
            DockPlacement::Bottom,
        ]
        .into_iter()
        .filter_map(|placement| area.layout(placement))
        .find_map(|tree| {
            let node = tree.find_node(tree.find_panel_node(adjacent_key)?)?;
            match node.kind() {
                PaneRef::Tabs { panels, .. } => Some(
                    if direction < 0 {
                        panels.last()
                    } else {
                        panels.first()
                    }
                    .copied(),
                ),
                PaneRef::Split { .. } => None,
            }
        })
        .flatten();
        drop(area);
        boundary
            .and_then(|key| self.pane_id_for_key(key))
            .is_some_and(|id| self.focus(&id, window, cx))
    }

    /// Extract a tab beside its group, or relocate a standalone pane beyond
    /// its directional neighbor. Native insertion keeps same-axis siblings flat.
    pub fn move_focused_tab(
        &mut self,
        direction: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let placement = match direction {
            "left" => gpui_kit::component::Placement::Left,
            "right" => gpui_kit::component::Placement::Right,
            "up" => gpui_kit::component::Placement::Top,
            "down" => gpui_kit::component::Placement::Bottom,
            _ => return false,
        };
        let Some(id) = self.focused_pane_id(window, cx) else { return false; };
        let Some(key) = self.panel_key(&id) else { return false; };
        let Some((region, group, panels, shape)) = focused_group_location(self, window, cx) else { return false; };
        // Resolve every retained panel before editing so failure can restore
        // the original topology without creating or removing pane entities.
        let rollback = {
            let area = self.area.read(cx);
            shape_to_layout(&shape, &area, cx)
        };
        let Some(rollback) = rollback else { return false; };
        let (target, allocation, weights) = {
            let area = self.area.read(cx);
            let Some(tree) = area.layout(region) else { return false; };
            let snapshots = area.split_geometry(cx);
            if panels.len() > 1 {
                (group, Some(split_allocation::SplitAllocation::capture(region, tree.root(), &snapshots)), HashMap::new())
            } else {
                let frames = collect_workspace_frames(&area, cx);
                let Some(source) = frames.iter().find(|frame| frame.panels.contains(&key)) else { return false; };
                let neighbor = frames.iter()
                    .filter(|frame| !frame.panels.contains(&key))
                    .filter_map(|frame| {
                        let target_key = *frame.panels.get(frame.active_ix)?;
                        let node = tree.find_panel_node(target_key)?;
                        directional_score(source.rect, frame.rect, direction).map(|score| (score, node))
                    })
                    .min_by(|a, b| a.0.total_cmp(&b.0));
                let Some((_, target)) = neighbor else { return false; };
                let prior = split_allocation::leaf_weights(tree.root(), &snapshots);
                let weights = groups_in(tree.root()).into_iter()
                    .filter_map(|group| Some((*group.panels.first()?, *prior.get(&group.node)?)))
                    .collect();
                (target, None, weights)
            }
        };
        self.area.update(cx, |area, cx| area.move_panel(key,
            InsertTarget::Split { node: target, placement, size: None }, window, cx));
        let mut snapshots = self.area.read(cx).split_geometry(cx);
        let prepared = {
            let area = self.area.read(cx);
            area.layout(region).is_some_and(|tree| {
                if let Some(allocation) = allocation {
                    tree.find_panel_node(key).is_some_and(|leaf| allocation.targets(tree.root(), leaf, &mut snapshots))
                } else {
                    movement_weight_targets(tree.root(), &weights, &mut snapshots).is_some()
                }
            })
        };
        snapshots.retain(|snapshot| self.area.read(cx).layout(region)
            .is_some_and(|tree| tree.find_node(snapshot.node).is_some()));
        self.equalize_motion = None;
        let applied = prepared && self.area.update(cx, |area, cx|
            area.equalize_split_geometry(&snapshots, 1., true, cx));
        if !applied {
            self.retaining_layout.set(true);
            self.area.update(cx, |area, cx| {
                if region == DockPlacement::Center { area.set_center(rollback, window, cx); }
                else { area.set_dock(region, rollback, window, cx); }
            });
            self.retaining_layout.set(false);
            self.focus(&id, window, cx);
            return false;
        }
        self.focus(&id, window, cx)
    }

    pub fn select_focused_tab_target(
        &mut self,
        target: &crate::vim_command::VimTabTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some((_, _, panels, shape)) = focused_group_location(self, window, cx) else {
            return false;
        };
        let group = shape_groups(&shape)
            .into_iter()
            .find(|group| group.panels == panels);
        let Some(group) = group else {
            return false;
        };
        if panels.is_empty() {
            return false;
        }
        let current = group.active_ix.min(panels.len() - 1);
        let target_ix = match target {
            crate::vim_command::VimTabTarget::Index(index) => index.saturating_sub(1),
            crate::vim_command::VimTabTarget::Last => panels.len() - 1,
            crate::vim_command::VimTabTarget::Relative(offset) => {
                (current as isize + offset).rem_euclid(panels.len() as isize) as usize
            }
        };
        panels
            .get(target_ix)
            .and_then(|key| self.pane_id_for_key(*key))
            .is_some_and(|id| self.focus(&id, window, cx))
    }

    pub fn keep_focused_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some((_, _, panels, shape)) = focused_group_location(self, window, cx) else {
            return false;
        };
        let group = shape_groups(&shape)
            .into_iter()
            .find(|group| group.panels == panels);
        let Some(group) = group else {
            return false;
        };
        let Some(active) = panels.get(group.active_ix).copied() else {
            return false;
        };
        let others = panels
            .into_iter()
            .filter(|key| *key != active)
            .collect::<Vec<_>>();
        if others.is_empty() {
            return true;
        }
        let panels_to_remove = others
            .iter()
            .filter_map(|key| {
                self.pane_id_for_key(*key)
                    .and_then(|id| self.panels.borrow().get(&id).cloned())
            })
            .collect::<Vec<_>>();
        self.area.update(cx, |area, cx| {
            for panel in panels_to_remove {
                area.remove_panel(panel, window, cx);
            }
        });
        self.pane_id_for_key(active)
            .is_some_and(|id| self.focus(&id, window, cx))
    }

    pub fn move_focused_tab_order(
        &mut self,
        target: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some((_, group_node, panels, shape)) = focused_group_location(self, window, cx) else {
            return false;
        };
        let Some(group) = shape_groups(&shape)
            .into_iter()
            .find(|group| group.node == group_node)
        else {
            return false;
        };
        let Some(key) = panels.get(group.active_ix).copied() else {
            return false;
        };
        let len = panels.len();
        if len < 2 {
            return false;
        }
        let destination = match target {
            "first" | "0" => 0,
            "$" | "last" => len - 1,
            raw if raw.starts_with('+') => {
                (group.active_ix + raw[1..].parse::<usize>().unwrap_or(1).max(1)) % len
            }
            raw if raw.starts_with('-') => {
                (group.active_ix + len - raw[1..].parse::<usize>().unwrap_or(1).max(1) % len) % len
            }
            raw => raw
                .parse::<usize>()
                .ok()
                .filter(|index| *index > 0)
                .map(|index| index.min(len) - 1)
                .unwrap_or(group.active_ix),
        };
        if group.active_ix == destination {
            return true;
        }
        self.area.update(cx, |area, cx| {
            area.move_panel(
                key,
                InsertTarget::Tabs {
                    node: group_node,
                    ix: Some(destination),
                    activate: true,
                },
                window,
                cx,
            );
        });
        self.pane_id_for_key(key)
            .is_some_and(|id| self.focus(&id, window, cx))
    }

    pub fn resize_focused_group(
        &mut self,
        axis: &str,
        amount: Option<isize>,
        maximize: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some((placement, group, _, mut shape)) = focused_group_location(self, window, cx)
        else {
            return false;
        };
        if !resize_shape_for_group(&mut shape, group, axis, amount, maximize) {
            return false;
        }
        let resized = self.install_shape(placement, shape, window, cx);
        if resized { self.set_auto_size_panes(false, window, cx); }
        resized
    }

    /// Split the focused tab group and put a retained empty pane beside it.
    pub fn split_focused_group(
        &mut self,
        direction: &str,
        size: Option<u32>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<bool, String> {
        let Some(group) = self.focused_group_node(window, cx) else {
            return Ok(false);
        };
        let placement = match direction {
            "left" => gpui_kit::component::Placement::Left,
            "right" => gpui_kit::component::Placement::Right,
            "up" => gpui_kit::component::Placement::Top,
            "down" => gpui_kit::component::Placement::Bottom,
            _ => return Ok(false),
        };
        self.add_empty_at(InsertTarget::Split {
            node: group, placement, size: size.map(|size| px(size as f32)),
        }, window, cx)?;
        Ok(true)
    }

    /// Close a tab group by moving all of its retained panels into another
    /// group in the same dock region. This removes only workspace references;
    /// task/session state remains owned by its existing pane entity/backend.
    pub fn close_focused_group(
        &mut self,
        target_index: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some((placement, focused_group, _, shape)) = focused_group_location(self, window, cx)
        else {
            return false;
        };
        let groups = shape_groups(&shape);
        let group = target_index
            .and_then(|index| groups.get(index.saturating_sub(1)))
            .map(|group| group.node)
            .unwrap_or(focused_group);
        let Some(source) = groups.iter().find(|candidate| candidate.node == group) else {
            return false;
        };
        let panels = source.panels.clone();
        let target = {
            let area = self.area.read(cx);
            let Some(tree) = area.layout(placement) else {
                return false;
            };
            groups_in(tree.root())
                .into_iter()
                .find(|candidate| candidate.node != group)
        };
        let Some(target) = target else {
            return false;
        };
        let target_active = target.panels.get(target.active_ix).copied();
        self.area.update(cx, |area, cx| {
            for key in panels {
                area.move_panel(
                    key,
                    InsertTarget::Tabs {
                        node: target.node,
                        ix: None,
                        activate: false,
                    },
                    window,
                    cx,
                );
            }
        });
        target_active
            .and_then(|key| self.pane_id_for_key(key))
            .is_some_and(|id| self.focus(&id, window, cx))
    }

    /// Keep the focused tab group and remove the other visible pane groups.
    /// The panels remain registered with DockWorkspace, so a later restore can
    /// rebuild them without recreating or stopping their underlying tasks.
    pub fn keep_focused_group(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some((_, _, focused_panels, shape)) = focused_group_location(self, window, cx) else {
            return false;
        };
        let Some(shape) = focused_group_shape(&shape, &focused_panels) else {
            return false;
        };
        let layout = {
            let area = self.area.read(cx);
            shape_to_layout(&shape, &area, cx)
        };
        let Some(layout) = layout else {
            return false;
        };
        let focus_key = first_active_panel(&shape);
        let area_entity = self.area();
        area_entity.update(cx, |area, cx| {
            area.set_center(layout, window, cx);
            for placement in [
                DockPlacement::Left,
                DockPlacement::Right,
                DockPlacement::Bottom,
            ] {
                area.remove_dock(placement, window, cx);
            }
        });
        focus_key
            .and_then(|key| self.pane_id_for_key(key))
            .is_some_and(|id| self.focus(&id, window, cx))
    }

    /// Exchange or rotate whole tab groups in the focused dock region.
    pub fn rearrange_groups(
        &mut self,
        target: Option<usize>,
        rotate: i8,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some((placement, focused, _, mut shape)) = focused_group_location(self, window, cx)
        else {
            return false;
        };
        let groups = shape_groups(&shape);
        if groups.len() < 2 {
            return false;
        }
        let Some(current_ix) = groups.iter().position(|group| group.node == focused) else {
            return false;
        };
        let next = if rotate != 0 {
            rotate_shape_groups(&mut shape, rotate.signum())
        } else {
            let target_ix = target
                .map(|one_based| one_based.saturating_sub(1))
                .unwrap_or((current_ix + 1) % groups.len());
            if target_ix >= groups.len() || target_ix == current_ix {
                return false;
            }
            exchange_shape_groups(&mut shape, focused, groups[target_ix].node)
        };
        if !next {
            return false;
        }
        let active_key = shape_groups(&shape)
            .into_iter()
            .find(|group| group.node == focused)
            .and_then(|group| group.panels.get(group.active_ix).copied());
        if !self.install_shape(placement, shape, window, cx) {
            return false;
        }
        active_key
            .and_then(|key| self.pane_id_for_key(key))
            .is_some_and(|id| self.focus(&id, window, cx))
    }

    /// Relocate the focused pane group as one unit to the requested outer edge
    /// of its current dock region; all child tabs and split ratios are kept.
    pub fn move_focused_group(
        &mut self,
        direction: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some((placement, focused, _, mut shape)) = focused_group_location(self, window, cx)
        else {
            return false;
        };
        if shape_group_count(&shape) < 2 {
            return false;
        }
        let Some(moved) = remove_shape_group(&mut shape, focused) else {
            return false;
        };
        let active_key = first_active_panel(&moved);
        if shape_group_count(&shape) == 0 {
            return false;
        }
        let (axis, moved_first) = match direction {
            "left" => (Axis::Horizontal, true),
            "right" => (Axis::Horizontal, false),
            "up" => (Axis::Vertical, true),
            "down" => (Axis::Vertical, false),
            _ => return false,
        };
        let children = if moved_first {
            vec![moved, shape]
        } else {
            vec![shape, moved]
        };
        let shape = WorkspaceShape::Split {
            node: None,
            axis,
            children,
            sizes: vec![None, None],
        };
        if !self.install_shape(placement, shape, window, cx) {
            return false;
        }
        active_key
            .and_then(|key| self.pane_id_for_key(key))
            .is_some_and(|id| self.focus(&id, window, cx))
    }

    fn install_shape(
        &mut self,
        placement: DockPlacement,
        shape: WorkspaceShape,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let layout = {
            let area = self.area.read(cx);
            shape_to_layout(&shape, &area, cx)
        };
        let Some(layout) = layout else {
            return false;
        };
        self.area.update(cx, |area, cx| {
            if placement == DockPlacement::Center {
                area.set_center(layout, window, cx);
            } else {
                area.set_dock(placement, layout, window, cx);
            }
        });
        true
    }

    /// Arrange retained panes into a legacy layout preset without closing them.
    pub fn apply_layout_preset(
        &mut self,
        preset: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let (columns, rows) = match preset {
            "single" => (1, 1), "columns" => (2, 1), "columns-3" => (3, 1),
            "columns-4" => (4, 1), "grid" => (2, 2), "grid-3x2" => (3, 2),
            "grid-4x2" => (4, 2), _ => return Err("Unknown pane layout preset.".into()),
        };
        let focused = self.focused_pane_id(window, cx);
        let mut groups = {
            let area = self.area.read(cx);
            [DockPlacement::Center, DockPlacement::Left, DockPlacement::Right, DockPlacement::Bottom]
                .into_iter().filter_map(|placement| area.layout(placement))
                .flat_map(|tree| groups_in(tree.root()))
                .filter(|group| !group.panels.is_empty())
                .map(|group| (group.panels, group.active_ix)).collect::<Vec<_>>()
        };
        let count = columns * rows;
        // Match the legacy presets: excess panes become tabs in the first
        // group. Retain every existing panel and its task/terminal lifecycle.
        if groups.len() > count {
            let overflow = groups.split_off(count);
            groups[0].0.extend(overflow.into_iter().flat_map(|group| group.0));
        }
        while groups.len() < count {
            let mut n = 1;
            let id = loop {
                let id = format!("{EMPTY_TAB_PREFIX}preset-{n}");
                if !self.contains(&id) { break id; }
                n += 1;
            };
            let spec = self.empty_spec(&id, window, cx)?;
            let panel = self.retain(spec, cx)?;
            groups.push((vec![PanelId::from(panel.entity_id())], 0));
        }
        let retained = self.panels.borrow().values().map(|panel|
            (PanelId::from(panel.entity_id()), panel.clone())).collect::<HashMap<_, _>>();
        let mut cells = Vec::new();
        for (keys, active) in groups {
            let mut tabs = DockLayout::tabs();
            for key in keys {
                let panel = retained.get(&key).ok_or("A pane is no longer available.")?;
                tabs = tabs.panel_view(panel_handle(panel.clone()), cx);
            }
            cells.push(tabs.active_index(active));
        }
        let mut row_layouts = Vec::new();
        let mut cells = cells.into_iter();
        for _ in 0..rows {
            let row_cells = cells.by_ref().take(columns).collect::<Vec<_>>();
            let row = if columns == 1 { row_cells.into_iter().next().unwrap() }
                else { row_cells.into_iter().fold(DockLayout::h_split(), |row, cell| row.child(cell, None)) };
            row_layouts.push(row);
        }
        let layout = if rows == 1 { row_layouts.into_iter().next().unwrap() }
            else { row_layouts.into_iter().fold(DockLayout::v_split(), |layout, row| layout.child(row, None)) };
        self.retaining_layout.set(true);
        self.area.update(cx, |area, cx| {
            area.set_center(layout, window, cx);
            for placement in [DockPlacement::Left, DockPlacement::Right, DockPlacement::Bottom] {
                area.remove_dock(placement, window, cx);
            }
        });
        self.retaining_layout.set(false);
        if let Some(id) = focused { self.focus(&id, window, cx); }
        cx.notify();
        Ok(())
    }

    /// Equalize nested split shares in place, keeping all native entities,
    /// active tabs, focus, terminals and composers throughout the transition.
    pub fn equalize_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let snapshots = self.area.read(cx).split_geometry(cx);
        if snapshots.is_empty() { self.equalize_motion = None; return false; }
        if snapshots.iter().all(|split| split.shares.iter().zip(&split.target_shares).all(|(share, target)| (share - target).abs() < 0.0005)) {
            if self.equalize_motion.take().is_some() {
                return self.area.update(cx, |area, cx| area.equalize_split_geometry(&snapshots, 1., true, cx));
            }
            return false;
        }
        if !crate::native_motion::motion_allowed() || !self.paint.borrow().shown || self.area.read(cx).is_zoomed() || !snapshots.iter().any(|split| split.animated) {
            self.equalize_motion = None;
            return self.area.update(cx, |area, cx| area.equalize_split_geometry(&snapshots, 1., true, cx));
        }
        // A repeated invocation samples the currently displayed shares.
        self.equalize_motion = Some((std::time::Instant::now(), snapshots));
        self.schedule_equalize_frame(window, cx);
        true
    }
    fn schedule_equalize_frame(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.equalize_frame_pending || self.equalize_motion.is_none() { return; }
        self.equalize_frame_pending = true;
        let weak = cx.weak_entity();
        let host_generation = self.host_generation;
        window.on_next_frame(move |window, cx| {
            let _ = weak.update(cx, |workspace, cx| {
                if workspace.host_generation != host_generation { return; }
                workspace.equalize_frame_pending = false;
                let Some((started, snapshots)) = workspace.equalize_motion.as_ref() else { return; };
                let progress = if !crate::native_motion::motion_allowed() || !workspace.paint.borrow().shown || crate::session_lock::locked(cx) || !workspace.area.read(cx).has_visible_split_geometry(snapshots, cx) {
                    1.
                } else {
                    crate::layout_motion::layout_progress(started.elapsed())
                };
                let settled = progress >= 1.;
                let applied = workspace.area.update(cx, |area, cx| area.equalize_split_geometry(snapshots, progress, settled, cx));
                if !applied || settled { workspace.equalize_motion = None; }
                else { workspace.schedule_equalize_frame(window, cx); }
            });
        });
    }
    /// Add and focus a native empty tab. Its stable prefix lets restore rebuild
    /// the pane without creating or changing any task/session state.
    pub fn add_empty_tab(
        &mut self,
        group: NodeId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<String, String> {
        if self.appearance.get().is_some_and(|appearance| appearance.tile_theme == NativeTileTheme::Tern) {
            return self.add_empty_split(group, true, window, cx);
        }
        self.add_empty_at(InsertTarget::Tabs { node: group, ix: None, activate: true }, window, cx)
    }

    /// Open a blank retained pane beside the owning group, without creating a task.
    pub fn add_empty_split(
        &mut self,
        group: NodeId,
        vertical: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<String, String> {
        self.add_empty_at(InsertTarget::Split {
            node: group,
            placement: if vertical { gpui_kit::component::Placement::Right } else { gpui_kit::component::Placement::Bottom },
            size: None,
        }, window, cx)
    }

    fn add_empty_at(
        &mut self,
        target: InsertTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<String, String> {
        let allocation = if let InsertTarget::Split { node, size: None, .. } = &target {
            let area = self.area.read(cx);
            let snapshots = area.split_geometry(cx);
            [DockPlacement::Center, DockPlacement::Left, DockPlacement::Right, DockPlacement::Bottom]
                .into_iter().find_map(|placement| {
                    let tree = area.layout(placement)?;
                    tree.find_node(*node)?;
                    Some(split_allocation::SplitAllocation::capture(placement, tree.root(), &snapshots))
                })
        } else { None };
        let mut index = 1_u64;
        let id = loop {
            let candidate = format!("{EMPTY_TAB_PREFIX}{index}");
            if !self.contains(&candidate) {
                break candidate;
            }
            index = index.saturating_add(1);
        };
        let spec = self.empty_spec(&id, window, cx)?;
        self.add(spec, DockPlacement::Center, None, window, cx)?;
        if !self.move_pane(
            &id,
            target,
            window,
            cx,
        ) {
            self.remove(&id, window, cx);
            return Err("The new pane could not be placed beside the focused pane.".into());
        }
        if let Some(allocation) = allocation {
            let mut snapshots = self.area.read(cx).split_geometry(cx);
            let prepared = {
                let area = self.area.read(cx);
                let key = self.panel_key(&id).expect("new pane retained");
                area.layout(allocation.placement).and_then(|tree| tree.find_panel_node(key)
                    .map(|new_leaf| allocation.targets(tree.root(), new_leaf, &mut snapshots)))
                    .unwrap_or(false)
            };
            snapshots.retain(|snapshot| self.area.read(cx).layout(allocation.placement)
                .is_some_and(|tree| tree.find_node(snapshot.node).is_some())
                && snapshot.shares.iter().zip(&snapshot.target_shares)
                    .any(|(share, target)| (share - target).abs() > 0.000001));
            self.equalize_motion = None;
            if !prepared || !self.area.update(cx, |area, cx| area.equalize_split_geometry(&snapshots, 1., true, cx)) {
                self.remove(&id, window, cx);
                return Err("The new pane's proportional split allocation could not be applied.".into());
            }
        }
        self.focus(&id, window, cx);
        Ok(id)
    }
    pub fn saved_ids(value: &Value) -> Result<Vec<String>, String> {
        let state: DockAreaState =
            serde_json::from_value(value.clone()).map_err(|error| error.to_string())?;
        if state.version != Some(VERSION) {
            return Err("Unsupported native workspace layout version.".into());
        }
        let mut ids = HashSet::new();
        collect_ids(&state.center, &mut ids, 0)?;
        for dock in [&state.left_dock, &state.right_dock, &state.bottom_dock]
            .into_iter()
            .flatten()
        {
            collect_ids(dock.panel(), &mut ids, 0)?;
        }
        if ids.len() > 64 {
            return Err("Native workspace supports at most 64 panes.".into());
        }
        let mut ids: Vec<_> = ids.into_iter().collect();
        ids.sort();
        Ok(ids)
    }
    pub fn add(
        &mut self,
        spec: PaneSpec,
        placement: DockPlacement,
        size: Option<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let id = spec.id.clone();
        let panel = self.retain(spec, cx)?;
        let key = PanelId::from(panel.entity_id());
        // `retain` also serves restore preflight, where the pane exists in
        // this workspace's registry but has not entered the DockArea yet.
        // Only skip insertion when the same retained panel is already live in
        // the area; repeated route/chat opens then focus it without creating a
        // second serialized leaf.
        if self.area.read(cx).panel(key).is_some() {
            self.focus(&id, window, cx);
            cx.notify();
            return Ok(());
        }
        self.area.update(cx, |area, cx| {
            area.add_panel_view(panel_handle(panel), placement, size, window, cx)
        });
        // A new tab or landing replacement need not increase the group count.
        // Coalesce explicit insertions with DockArea's structural notifications;
        // the deferred pass samples the completed tree and resizes every split.
        self.schedule_auto_size(window, cx);
        cx.notify();
        Ok(())
    }
    /// Browser links create a separate split; the originating pane stays visible.
    pub fn add_browser_beside(&mut self, spec: PaneSpec, origin: Option<NodeId>, window: &mut Window, cx: &mut Context<Self>) -> Result<(), String> {
        let id = spec.id.clone();
        if self.contains(&id) { return Err("The browser pane identity is already retained.".into()); }
        self.add(spec, DockPlacement::Center, None, window, cx)?;
        if let Some(node) = origin {
            if !self.move_pane(&id, InsertTarget::Split { node, placement: gpui_kit::component::Placement::Right, size: None }, window, cx) {
                self.remove(&id, window, cx);
                return Err("The browser pane could not be placed beside its source.".into());
            }
        }
        self.focus(&id, window, cx);
        Ok(())
    }

    /// Create a separate pane below the named origin, regardless of focus.
    pub fn add_below(&mut self, spec: PaneSpec, origin: &str, window: &mut Window, cx: &mut Context<Self>) -> Result<(), String> {
        let node = self.group_for_pane(origin, cx).ok_or("The originating chat pane is unavailable.")?;
        let id = spec.id.clone();
        if self.contains(&id) { return Err("The new pane identity is already retained.".into()); }
        self.add(spec, DockPlacement::Center, None, window, cx)?;
        if !self.move_pane(&id, InsertTarget::Split { node, placement: gpui_kit::component::Placement::Bottom, size: None }, window, cx) {
            self.remove(&id, window, cx);
            return Err("The new pane could not be placed below its chat.".into());
        }
        self.focus(&id, window, cx);
        cx.notify();
        Ok(())
    }

    pub fn set_title(&mut self, id: &str, title: SharedString, cx: &mut Context<Self>) {
        let panel = self.panels.borrow().get(id).cloned();
        if let Some(panel) = panel {
            panel.update(cx, |panel, cx| {
                if panel.spec.title != title {
                    panel.spec.title = title;
                    cx.notify();
                }
            });
        }
    }
    pub fn pane_title(&self, id: &str, cx: &App) -> Option<SharedString> {
        self.panels.borrow().get(id).map(|panel| panel.read(cx).spec.title.clone())
    }
    /// Focus a pane already in this layout, opening its edge dock if needed.
    /// Hidden retained workspace panels never qualify and are never inserted.
    pub fn reveal_and_focus(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(panel) = self.panels.borrow().get(id).cloned() else { return false; };
        let key = PanelId::from(panel.entity_id());
        let placement = {
            let area = self.area.read(cx);
            [DockPlacement::Center, DockPlacement::Left, DockPlacement::Right, DockPlacement::Bottom]
                .into_iter().find(|placement| area.layout(*placement)
                    .is_some_and(|tree| tree.panels().any(|panel| panel == key)))
        };
        let Some(placement) = placement else { return false; };
        if placement != DockPlacement::Center && !self.area.read(cx).is_dock_open(placement) {
            self.area.update(cx, |area, cx| area.toggle_dock(placement, window, cx));
        }
        self.focus(id, window, cx)
    }
    pub fn focus(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(panel) = self.panels.borrow().get(id).cloned() else {
            return false;
        };
        let key = PanelId::from(panel.entity_id());
        if self.area.read(cx).panel(key).is_none() {
            return false;
        }
        self.area
            .update(cx, |area, cx| area.select_panel(key, window, cx));
        let content = panel.read(cx).spec.content.clone();
        let focus = content.downcast::<crate::chat::ChatPane>().ok()
            .map(|chat| chat.read(cx).composer.read(cx).focus_handle(cx))
            .unwrap_or_else(|| panel.read(cx).spec.focus.clone());
        focus.focus(window, cx);
        true
    }
    /// GPUI handles drop geometry, tab order, resizing and split normalization.
    pub fn move_pane(
        &mut self,
        id: &str,
        target: InsertTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(panel) = self.panels.borrow().get(id).cloned() else {
            return false;
        };
        self.area.update(cx, |area, cx| {
            area.move_panel(PanelId::from(panel.entity_id()), target, window, cx)
        });
        self.schedule_auto_size(window, cx);
        true
    }
    /// Landing panes may be closed even when they are the last docked pane.
    pub fn close_empty(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !id.starts_with(EMPTY_TAB_PREFIX) || !self.empty_panes.contains_key(id) { return false; }
        self.remove(id, window, cx)
    }
    pub fn remove(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(panel) = self.panels.borrow().get(id).cloned() else {
            return false;
        };
        self.area
            .update(cx, |area, cx| area.remove_panel(panel.clone(), window, cx));
        if self
            .area
            .read(cx)
            .panel(PanelId::from(panel.entity_id()))
            .is_some()
        {
            return false;
        }
        self.panels.borrow_mut().remove(id);
        self.empty_subscriptions.remove(id);
        self.empty_panes.remove(id);
        cx.notify();
        true
    }
    /// Closing native children happens via Removed hooks. The host must then
    /// render a blank frame before destroying the GPUI window, to release its
    /// frame-arena view clones (especially native webview hosts).
    pub fn close_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_shown(false, window, cx);
        self.area.update(cx, |area, cx| {
            area.set_center(DockLayout::tabs(), window, cx);
            for placement in [
                DockPlacement::Left,
                DockPlacement::Right,
                DockPlacement::Bottom,
            ] {
                area.remove_dock(placement, window, cx);
            }
        });
        self.panels.borrow_mut().clear();
        self.empty_subscriptions.clear();
        self.empty_panes.clear();
        cx.notify();
    }
    pub fn serialize(&self, cx: &App) -> Result<Value, String> {
        serde_json::to_value(self.area.read(cx).dump(cx)).map_err(|error| error.to_string())
    }
    /// Layout switching changes visibility without closing retained pane content.
    pub fn restore_retaining(&mut self, value: Value, window: &mut Window, cx: &mut Context<Self>) -> Result<(), String> {
        self.retaining_layout.set(true);
        let result = self.restore(value, window, cx);
        self.retaining_layout.set(false);
        result
    }
    pub fn restore(
        &mut self,
        value: Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let state: DockAreaState =
            serde_json::from_value(value).map_err(|error| error.to_string())?;
        if state.version != Some(VERSION) {
            return Err("Unsupported native workspace layout version.".into());
        }
        let mut ids = HashSet::new();
        collect_ids(&state.center, &mut ids, 0)?;
        for dock in [&state.left_dock, &state.right_dock, &state.bottom_dock]
            .into_iter()
            .flatten()
        {
            collect_ids(dock.panel(), &mut ids, 0)?;
        }
        if ids.len() > 64 {
            return Err("Native workspace supports at most 64 panes.".into());
        }
        for id in ids {
            if !self.panels.borrow().contains_key(&id) {
                let spec = match (self.factory)(&id, window, cx) {
                    Some(spec) => spec,
                    None if id.starts_with(EMPTY_TAB_PREFIX) => self.empty_spec(&id, window, cx)?,
                    None if crate::markdown_pane::path_from_pane_id(&id).is_some() => {
                        let path = crate::markdown_pane::path_from_pane_id(&id).expect("validated above");
                        let pane = cx.new(|cx| crate::markdown_pane::MarkdownPane::new(path, window, cx));
                        PaneSpec {
                            id: id.clone(),
                            title: pane.read(cx).title().to_owned().into(),
                            content: pane.clone().into(),
                            focus: pane.focus_handle(cx),
                            lifecycle: None,
                            closable: true,
                        }
                    }
                    None => {
                        return Err(format!(
                            "Pane {id} is unavailable; existing layout retained."
                        ));
                    }
                };
                if spec.id != id {
                    return Err("Pane factory returned a different identity.".into());
                }
                self.retain(spec, cx)?;
            }
        }
        self.cancel_layout_motion();
        self.restoring_layout = true;
        self.restore_generation = self.restore_generation.wrapping_add(1);
        let restore_generation = self.restore_generation;
        let loaded = self.area.update(cx, |area, cx| area.load(state, window, cx));
        // Dock events are delivered after the enclosing entity update. Keep
        // the restore guard until those effects have been consumed; a load is
        // never an inserted-pane gesture.
        cx.defer_in(window, move |workspace, _, _| {
            if workspace.restore_generation == restore_generation {
                workspace.restoring_layout = false;
            }
        });
        loaded.map_err(|error| error.to_string())?;
        // Restoring/switching workspaces must preserve the saved proportions.
        self.live_group_count = self.visible_group_count(cx);
        self.live_panel_count = self.visible_panel_count(cx);
        // A single restored group has an unambiguous active pane. Give it
        // actual keyboard focus so shortcuts work before the first click.
        // Multiple split groups require an explicit user focus choice.
        let active = {
            let area = self.area.read(cx);
            let mut groups = Vec::new();
            for placement in [
                DockPlacement::Center,
                DockPlacement::Left,
                DockPlacement::Right,
                DockPlacement::Bottom,
            ] {
                if let Some(tree) = area.layout(placement) {
                    tree.root().walk(&mut |node| {
                        if let PaneRef::Tabs { panels, active_ix } = node.kind() {
                            if let Some(panel) = panels.get(active_ix) {
                                groups.push(*panel);
                            }
                        }
                    });
                }
            }
            (groups.len() == 1).then(|| groups[0])
        };
        if let Some(id) = active.and_then(|key| self.pane_id_for_key(key)) {
            self.focus(&id, window, cx);
        }
        cx.notify();
        Ok(())
    }
    fn empty_spec(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) -> Result<PaneSpec, String> {
        if !id.starts_with(EMPTY_TAB_PREFIX) || id.len() <= EMPTY_TAB_PREFIX.len() {
            return Err("Invalid empty pane identity.".into());
        }
        let pane = if let Some(pane) = self.empty_panes.get(id) {
            pane.clone()
        } else {
            let pane = cx.new(|cx| EmptyPane::new(id, window, cx));
            let event_id = id.to_owned();
            let subscription = cx.subscribe(
                &pane,
                move |workspace, source, event: &EmptyPaneEvent, cx| {
                    let is_current = workspace
                        .empty_panes
                        .get(&event_id)
                        .is_some_and(|current| current.entity_id() == source.entity_id());
                    let is_retained = workspace
                        .panels
                        .borrow()
                        .get(&event_id)
                        .is_some_and(|panel| !panel.read(cx).removed);
                    if is_current && is_retained {
                        match event {
                            EmptyPaneEvent::Close => cx.emit(WorkspaceEvent::EmptyClose { pane_id: event_id.clone() }),
                            EmptyPaneEvent::Action(action) => cx.emit(WorkspaceEvent::EmptyAction { pane_id: event_id.clone(), action: *action }),
                            EmptyPaneEvent::OpenSession(task_id) => cx.emit(WorkspaceEvent::EmptySession { pane_id: event_id.clone(), task_id: task_id.clone() }),
                        }
                    }
                },
            );
            self.empty_panes.insert(id.to_owned(), pane.clone());
            self.empty_subscriptions.insert(id.to_owned(), subscription);
            pane
        };
        Ok(PaneSpec {
            id: id.to_owned(),
            title: "New tab".into(),
            content: pane.clone().into(),
            focus: pane.read(cx).focus_handle(cx),
            lifecycle: None,
            closable: true,
        })
    }
    pub fn set_empty_presentation(
        &mut self,
        palette: crate::design::Palette,
        interface_font_ratio: f32,
        busy: bool,
        browser_available: bool,
        cx: &mut Context<Self>,
    ) {
        let panes: Vec<_> = self.empty_panes.values().cloned().collect();
        for pane in panes {
            pane.update(cx, |pane, cx| {
                pane.set_presentation(palette, interface_font_ratio, busy, browser_available, cx)
            });
        }
    }
    pub fn set_empty_sessions(&mut self, sessions: &monitter_gpui_desktop::session_picker::PickerGroups, cx: &mut Context<Self>) {
        for pane in self.empty_panes.values() {
            pane.update(cx, |pane, cx| pane.set_sessions(sessions, cx));
        }
    }
    /// Move a retained chat into the exact group containing the landing tab.
    /// The existing chat entity is reused, including a running native session.
    pub fn replace_empty_with_chat(&mut self, empty: &str, chat: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !empty.starts_with(EMPTY_TAB_PREFIX) || !chat.starts_with("chat:") { return false; }
        self.replace_pane(empty, chat, window, cx)
    }
    /// Replace only the source tab, preserving its group and tab position.
    pub fn replace_pane(&mut self, empty: &str, chat: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(empty_key) = self.panel_key(empty) else { return false; };
        let Some(chat_key) = self.panel_key(chat) else { return false; };
        let destination = {
            let area = self.area.read(cx);
            [DockPlacement::Center, DockPlacement::Left, DockPlacement::Right, DockPlacement::Bottom]
                .into_iter().filter_map(|placement| area.layout(placement))
                .find_map(|tree| {
                    let node = tree.find_panel_node(empty_key)?;
                    let PaneRef::Tabs { panels, .. } = tree.find_node(node)?.kind() else { return None; };
                    let index = panels.iter().position(|key| *key == empty_key)?;
                    Some((node, index))
                })
        };
        let Some((node, index)) = destination else { return false; };
        self.area.update(cx, |area, cx| area.move_panel(chat_key, InsertTarget::Tabs { node, ix: Some(index), activate: true }, window, cx));
        // Verify the insertion before retiring the landing pane.
        let moved = {
            let area = self.area.read(cx);
            [DockPlacement::Center, DockPlacement::Left, DockPlacement::Right, DockPlacement::Bottom]
                .into_iter().filter_map(|placement| area.layout(placement))
                .any(|tree| tree.find_panel_node(chat_key) == Some(node))
        };
        if !moved { return false; }
        self.remove(empty, window, cx);
        self.focus(chat, window, cx);
        self.schedule_auto_size(window, cx);
        true
    }
    /// Call before hiding a route/window. Browser hooks must hide native child
    /// views here; before window removal the host must close and blank-draw them.
    pub fn set_shown(&mut self, shown: bool, window: &mut Window, cx: &mut Context<Self>) {
        let shown = shown && !crate::session_lock::locked(cx);
        if !shown {
            crate::native_pane_highlight::clear(window);
        }
        let changed = self.paint.borrow().shown != shown;
        self.paint.borrow_mut().shown = shown;
        if !shown || window.has_active_dialog(cx) {
            self.apply_visibility(HashSet::new(), window, cx);
        }
        if changed {cx.notify();}
    }
    fn apply_visibility(
        &mut self,
        next: HashSet<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        crate::native_pane_highlight::retain_panes(&next.iter().cloned().collect::<Vec<_>>(), window);
        let prior = std::mem::replace(&mut self.paint.borrow_mut().visible, next.clone());
        for id in prior.symmetric_difference(&next) {
            let visible = next.contains(id);
            if let Some(panel) = self.panels.borrow().get(id) {
                let background = panel.read(cx).background.clone();
                if let Some(background) = background { background.update(cx, |background, cx| background.set_visible(visible, cx)); }
                let hook = panel.read(cx).spec.lifecycle.clone();
                if let Some(hook) = hook {
                    hook(PaneLifecycle::Visible(visible), window, cx);
                }
            }
            cx.emit(WorkspaceEvent::VisibilityChanged {
                id: id.clone(),
                visible,
            });
        }
    }
}
impl Render for DockWorkspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // WKWebViews are native children above the GPU surface. Park them
        // before drawing a native dialog while retaining the desired route.
        if window.has_active_dialog(cx) {
            self.apply_visibility(HashSet::new(), window, cx);
        }
        let schedule_callback = {
            let mut paint = self.paint.borrow_mut();
            paint.painted.clear();
            if paint.frame_callback_pending {
                false
            } else {
                paint.frame_callback_pending = true;
                true
            }
        };
        if schedule_callback {
            let weak = cx.weak_entity();
            let host_generation = self.host_generation;
            window.on_next_frame(move |window, cx| {
                let _ = weak.update(cx, |workspace, cx| {
                    if workspace.host_generation != host_generation { return; }
                    let next = {
                        let mut paint = workspace.paint.borrow_mut();
                        paint.frame_callback_pending = false;
                        if paint.shown && !window.has_active_dialog(cx) {
                            let next = std::mem::take(&mut paint.painted);
                            #[cfg(feature = "performance-profiling")]
                            {
                                PAINTED_PANE_OBSERVATIONS.fetch_add(next.len() as u64, Ordering::Relaxed);
                                let missing = paint.visible.difference(&next).count() as u64;
                                MISSING_VISIBLE_PANE_OBSERVATIONS.fetch_add(missing, Ordering::Relaxed);
                            }
                            next
                        } else {
                            HashSet::new()
                        }
                    };
                    workspace.apply_visibility(next, window, cx);
                    let removed = std::mem::take(&mut workspace.paint.borrow_mut().removed);
                    for (id, was_visible) in removed {
                        let still_present = workspace
                            .panels
                            .borrow()
                            .get(&id)
                            .is_some_and(|panel| !panel.read(cx).removed);
                        if still_present {
                            continue;
                        }
                        workspace.panels.borrow_mut().remove(&id);
                        workspace.empty_subscriptions.remove(&id);
                        workspace.empty_panes.remove(&id);
                        if was_visible {
                            cx.emit(WorkspaceEvent::VisibilityChanged {
                                id: id.clone(),
                                visible: false,
                            });
                        }
                        cx.emit(WorkspaceEvent::Removed { id });
                    }
                });
            });
        }
        let empty = !self.has_layout_panes(cx);
        div().size_full().relative().child(self.area.clone())
            .when(empty, |surface| surface.child(
                div().id("empty-workspace").absolute().inset_0()
                    .flex().flex_col().items_center().justify_center().gap(px(16.))
                    .text_color(cx.theme().foreground)
                    .child(div().text_size(px(16.)).child("Nothing to see here."))
                    .child(div().flex().flex_wrap().justify_center().gap(px(8.))
                        .children([
                            ("empty-workspace-chat", "New Chat", EmptyPaneAction::Chat),
                            ("empty-workspace-terminal", "Terminal", EmptyPaneAction::Terminal),
                            ("empty-workspace-neovim", "Neovim", EmptyPaneAction::Neovim),
                            ("empty-workspace-browser", "Browser", EmptyPaneAction::Browser),
                        ].into_iter().map(|(id, label, action)| {
                            gpui_kit::component::button::Button::new(id).label(label).small()
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    cx.emit(WorkspaceEvent::EmptyWorkspaceAction(action));
                                }))
                        })))))
    }
}

#[derive(Clone, Copy, Debug)]
struct NavRect {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

impl NavRect {
    fn center(self) -> (f32, f32) {
        (self.x + self.width * 0.5, self.y + self.height * 0.5)
    }
}

#[derive(Clone)]
struct NavFrame {
    rect: NavRect,
    panels: Vec<PanelId>,
    active_ix: usize,
}

#[derive(Clone, Debug)]
enum WorkspaceShape {
    Split {
        node: Option<NodeId>,
        axis: Axis,
        children: Vec<WorkspaceShape>,
        sizes: Vec<Option<Pixels>>,
    },
    Tabs {
        node: Option<NodeId>,
        panels: Vec<PanelId>,
        active_ix: usize,
    },
}

#[derive(Clone, Debug)]
struct WorkspaceGroup {
    node: NodeId,
    panels: Vec<PanelId>,
    active_ix: usize,
}

/// Retarget geometry after relocation using stable panel keys, since native
/// detach/insert can replace leaf and ancestor node IDs.
fn movement_weight_targets(
    node: &PaneNode,
    weights: &HashMap<PanelId, f32>,
    snapshots: &mut [DockSplitSnapshot],
) -> Option<f32> {
    match node.kind() {
        PaneRef::Tabs { panels, .. } => panels.iter().find_map(|panel| weights.get(panel).copied()),
        PaneRef::Split { children, .. } => {
            let weights = children.iter().map(|child| movement_weight_targets(child, weights, snapshots))
                .collect::<Option<Vec<_>>>()?;
            let total: f32 = weights.iter().sum();
            if !total.is_finite() || total <= 0. { return None; }
            if children.len() > 1 {
                let snapshot = snapshots.iter_mut().find(|snapshot| snapshot.node == node.id())?;
                snapshot.target_shares = weights.iter().map(|weight| weight / total).collect();
            }
            Some(total)
        }
    }
}

fn capture_shape(node: &PaneNode) -> WorkspaceShape {
    match node.kind() {
        PaneRef::Tabs { panels, active_ix } => WorkspaceShape::Tabs {
            node: Some(node.id()),
            panels: panels.to_vec(),
            active_ix,
        },
        PaneRef::Split {
            axis,
            children,
            sizes,
        } => WorkspaceShape::Split {
            node: Some(node.id()),
            axis,
            children: children.iter().map(capture_shape).collect(),
            sizes: sizes.to_vec(),
        },
    }
}

fn collect_shape_groups(shape: &WorkspaceShape, groups: &mut Vec<WorkspaceGroup>) {
    match shape {
        WorkspaceShape::Tabs {
            node: Some(node),
            panels,
            active_ix,
        } => groups.push(WorkspaceGroup {
            node: *node,
            panels: panels.clone(),
            active_ix: *active_ix,
        }),
        WorkspaceShape::Split { children, .. } => {
            for child in children {
                collect_shape_groups(child, groups);
            }
        }
        WorkspaceShape::Tabs { node: None, .. } => {}
    }
}

fn groups_in(node: &PaneNode) -> Vec<WorkspaceGroup> {
    let shape = capture_shape(node);
    let mut groups = Vec::new();
    collect_shape_groups(&shape, &mut groups);
    groups
}

fn shape_groups(shape: &WorkspaceShape) -> Vec<WorkspaceGroup> {
    let mut groups = Vec::new();
    collect_shape_groups(shape, &mut groups);
    groups
}

fn focused_group_shape(shape: &WorkspaceShape, panels: &[PanelId]) -> Option<WorkspaceShape> {
    match shape {
        WorkspaceShape::Tabs {
            node,
            panels: candidate,
            active_ix,
        } if candidate == panels => Some(WorkspaceShape::Tabs {
            node: *node,
            panels: candidate.clone(),
            active_ix: *active_ix,
        }),
        WorkspaceShape::Split { children, .. } => children
            .iter()
            .find_map(|child| focused_group_shape(child, panels)),
        _ => None,
    }
}

fn shape_group_count(shape: &WorkspaceShape) -> usize {
    shape_groups(shape).len()
}

fn find_shape_group_mut(shape: &mut WorkspaceShape, target: NodeId) -> Option<&mut WorkspaceShape> {
    match shape {
        WorkspaceShape::Tabs {
            node: Some(node), ..
        } if *node == target => Some(shape),
        WorkspaceShape::Split { children, .. } => children
            .iter_mut()
            .find_map(|child| find_shape_group_mut(child, target)),
        _ => None,
    }
}

fn exchange_shape_groups(shape: &mut WorkspaceShape, left: NodeId, right: NodeId) -> bool {
    let mut changed = shape.clone();
    let left_group = shape_groups(&changed)
        .into_iter()
        .find(|group| group.node == left);
    let right_group = shape_groups(&changed)
        .into_iter()
        .find(|group| group.node == right);
    let (Some(left_group), Some(right_group)) = (left_group, right_group) else {
        return false;
    };
    if let Some(WorkspaceShape::Tabs {
        panels, active_ix, ..
    }) = find_shape_group_mut(&mut changed, left)
    {
        *panels = right_group.panels;
        *active_ix = right_group.active_ix;
    }
    if let Some(WorkspaceShape::Tabs {
        panels, active_ix, ..
    }) = find_shape_group_mut(&mut changed, right)
    {
        *panels = left_group.panels;
        *active_ix = left_group.active_ix;
    }
    *shape = changed;
    true
}

fn rotate_shape_groups(shape: &mut WorkspaceShape, direction: i8) -> bool {
    let groups = shape_groups(shape);
    if groups.len() < 2 {
        return false;
    }
    let payloads = groups
        .iter()
        .map(|group| (group.panels.clone(), group.active_ix))
        .collect::<Vec<_>>();
    for (index, group) in groups.iter().enumerate() {
        let source = if direction > 0 {
            (index + groups.len() - 1) % groups.len()
        } else {
            (index + 1) % groups.len()
        };
        if let Some(WorkspaceShape::Tabs {
            panels, active_ix, ..
        }) = find_shape_group_mut(shape, group.node)
        {
            *panels = payloads[source].0.clone();
            *active_ix = payloads[source].1;
        }
    }
    true
}

fn remove_shape_group(shape: &mut WorkspaceShape, target: NodeId) -> Option<WorkspaceShape> {
    if matches!(shape, WorkspaceShape::Tabs { node: Some(node), .. } if *node == target) {
        return Some(shape.clone());
    }
    let WorkspaceShape::Split {
        children, sizes, ..
    } = shape
    else {
        return None;
    };
    for index in 0..children.len() {
        if let Some(removed) = remove_shape_group(&mut children[index], target) {
            children.remove(index);
            if index < sizes.len() {
                sizes.remove(index);
            }
            if children.len() == 1 {
                let replacement = children[0].clone();
                *shape = replacement;
            }
            return Some(removed);
        }
    }
    None
}

fn first_active_panel(shape: &WorkspaceShape) -> Option<PanelId> {
    match shape {
        WorkspaceShape::Tabs {
            panels, active_ix, ..
        } => panels.get(*active_ix).copied(),
        WorkspaceShape::Split { children, .. } => children.iter().find_map(first_active_panel),
    }
}

fn shape_to_layout(shape: &WorkspaceShape, area: &DockArea, cx: &App) -> Option<DockLayout> {
    match shape {
        WorkspaceShape::Tabs {
            panels, active_ix, ..
        } => {
            let mut layout = DockLayout::tabs();
            for key in panels {
                let panel = area.panel(*key)?.clone();
                layout = layout.panel_view(panel, cx);
            }
            Some(layout.active_index(*active_ix))
        }
        WorkspaceShape::Split {
            axis,
            children,
            sizes,
            ..
        } => {
            let mut layout = if *axis == Axis::Horizontal {
                DockLayout::h_split()
            } else {
                DockLayout::v_split()
            };
            for (index, child) in children.iter().enumerate() {
                layout = layout.child(
                    shape_to_layout(child, area, cx)?,
                    sizes.get(index).copied().flatten(),
                );
            }
            Some(layout)
        }
    }
}

fn shape_contains_group(shape: &WorkspaceShape, target: NodeId) -> bool {
    match shape {
        WorkspaceShape::Tabs {
            node: Some(node), ..
        } => *node == target,
        WorkspaceShape::Split { children, .. } => children
            .iter()
            .any(|child| shape_contains_group(child, target)),
        _ => false,
    }
}

fn resize_shape_for_group(
    shape: &mut WorkspaceShape,
    target: NodeId,
    axis: &str,
    amount: Option<isize>,
    maximize: bool,
) -> bool {
    let WorkspaceShape::Split {
        axis: current_axis,
        children,
        sizes,
        ..
    } = shape
    else {
        return false;
    };
    let expected = if axis == "horizontal" {
        Axis::Horizontal
    } else {
        Axis::Vertical
    };
    let child_ix = children
        .iter()
        .position(|child| shape_contains_group(child, target));
    let Some(child_ix) = child_ix else {
        return false;
    };
    if *current_axis == expected {
        if sizes.len() != children.len() {
            return false;
        }
        let selected = sizes[child_ix]
            .map(|size| size / px(1.))
            .unwrap_or(100.0)
            .max(1.0);
        if maximize {
            for (index, slot) in sizes.iter_mut().enumerate() {
                *slot = Some(px(if index == child_ix { 10_000.0 } else { 1.0 }));
            }
            return true;
        }
        let target_size = match amount {
            Some(value) if value < 0 => (selected + value as f32).max(1.0),
            Some(value) => (value as f32).max(1.0),
            None => return false,
        };
        let delta = target_size - selected;
        let Some(donor_ix) = sizes
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != child_ix)
            .max_by(|a, b| {
                let a = a.1.map(|size| size / px(1.)).unwrap_or(100.0);
                let b = b.1.map(|size| size / px(1.)).unwrap_or(100.0);
                a.total_cmp(&b)
            })
            .map(|(index, _)| index)
        else {
            return false;
        };
        let donor = sizes[donor_ix].map(|size| size / px(1.)).unwrap_or(100.0);
        if donor - delta < 1.0 {
            return false;
        }
        sizes[child_ix] = Some(px(target_size));
        sizes[donor_ix] = Some(px(donor - delta));
        return true;
    }
    for child in children {
        if resize_shape_for_group(child, target, axis, amount, maximize) {
            return true;
        }
    }
    false
}

fn focused_group_location(
    dock: &DockWorkspace,
    window: &Window,
    cx: &App,
) -> Option<(DockPlacement, NodeId, Vec<PanelId>, WorkspaceShape)> {
    let focused = dock.focused_pane_id(window, cx)?;
    let key = dock.panel_key(&focused)?;
    let area = dock.area().read(cx);
    for placement in [
        DockPlacement::Center,
        DockPlacement::Left,
        DockPlacement::Right,
        DockPlacement::Bottom,
    ] {
        let Some(tree) = area.layout(placement) else {
            continue;
        };
        let shape = capture_shape(tree.root());
        for group in shape_groups(&shape) {
            if group.panels.contains(&key) {
                return Some((placement, group.node, group.panels, shape));
            }
        }
    }
    None
}

fn collect_workspace_frames(area: &DockArea, cx: &App) -> Vec<NavFrame> {
    let mut frames = Vec::new();
    for (placement, rect) in [
        (
            DockPlacement::Center,
            NavRect {
                x: 0.,
                y: 0.,
                width: 1.,
                height: 1.,
            },
        ),
        (
            DockPlacement::Left,
            NavRect {
                x: -0.32,
                y: 0.,
                width: 0.32,
                height: 1.,
            },
        ),
        (
            DockPlacement::Right,
            NavRect {
                x: 1.,
                y: 0.,
                width: 0.32,
                height: 1.,
            },
        ),
        (
            DockPlacement::Bottom,
            NavRect {
                x: 0.,
                y: 1.,
                width: 1.,
                height: 0.32,
            },
        ),
    ] {
        if let Some(tree) = area.layout(placement) {
            collect_nav_frames(tree.root(), rect, &mut frames);
        }
    }
    let _ = cx;
    frames
}

fn collect_nav_frames(node: &PaneNode, rect: NavRect, frames: &mut Vec<NavFrame>) {
    match node.kind() {
        PaneRef::Tabs { panels, active_ix } => frames.push(NavFrame {
            rect,
            panels: panels.to_vec(),
            active_ix,
        }),
        PaneRef::Split {
            axis,
            children,
            sizes,
        } => {
            if children.is_empty() {
                return;
            }
            let mut weights: Vec<f32> = children
                .iter()
                .enumerate()
                .map(|(ix, _)| {
                    sizes
                        .get(ix)
                        .copied()
                        .flatten()
                        .map(|size| (size / gpui_kit::px(1.)).max(0.001))
                        .unwrap_or(1.)
                })
                .collect();
            let total: f32 = weights.iter().sum();
            if !total.is_finite() || total <= 0. {
                weights.fill(1.);
            }
            let total: f32 = weights.iter().sum();
            let mut offset = 0.;
            for (child, weight) in children.iter().zip(weights) {
                let share = weight / total;
                let child_rect = if axis == Axis::Horizontal {
                    NavRect {
                        x: rect.x + rect.width * offset,
                        y: rect.y,
                        width: rect.width * share,
                        height: rect.height,
                    }
                } else {
                    NavRect {
                        x: rect.x,
                        y: rect.y + rect.height * offset,
                        width: rect.width,
                        height: rect.height * share,
                    }
                };
                collect_nav_frames(child, child_rect, frames);
                offset += share;
            }
        }
    }
}

fn directional_score(from: NavRect, to: NavRect, direction: &str) -> Option<f32> {
    let (fx, fy) = from.center();
    let (tx, ty) = to.center();
    let (primary, cross) = match direction {
        "left" if tx < fx => (fx - tx, (fy - ty).abs()),
        "right" if tx > fx => (tx - fx, (fy - ty).abs()),
        "up" if ty < fy => (fy - ty, (fx - tx).abs()),
        "down" if ty > fy => (ty - fy, (fx - tx).abs()),
        _ => return None,
    };
    Some(primary + cross * 1.5)
}


#[cfg(test)]
mod keyboard_navigation_tests {
    use super::{Axis, NavRect, WorkspaceShape, directional_score, focused_group_shape};
    use gpui_kit::component::dock::PanelId;

    #[test]
    fn directional_focus_chooses_only_panes_in_requested_half_plane() {
        let origin = NavRect {
            x: 0.,
            y: 0.,
            width: 0.5,
            height: 1.,
        };
        let right = NavRect {
            x: 0.5,
            y: 0.,
            width: 0.5,
            height: 1.,
        };
        let above = NavRect {
            x: 0.,
            y: -1.,
            width: 0.5,
            height: 1.,
        };
        assert!(directional_score(origin, right, "right").is_some());
        assert!(directional_score(origin, right, "left").is_none());
        assert!(directional_score(origin, above, "up").is_some());
        assert!(directional_score(origin, above, "down").is_none());
    }

    #[test]
    fn directional_focus_prefers_nearer_primary_axis_before_cross_axis_alignment() {
        let origin = NavRect {
            x: 0.,
            y: 0.,
            width: 0.2,
            height: 0.2,
        };
        let near = NavRect {
            x: 0.4,
            y: 0.8,
            width: 0.2,
            height: 0.2,
        };
        let far = NavRect {
            x: 0.8,
            y: 0.,
            width: 0.2,
            height: 0.2,
        };
        assert!(
            directional_score(origin, far, "right").unwrap()
                < directional_score(origin, near, "right").unwrap()
        );
    }

    #[test]
    fn only_pane_shape_retains_only_the_focused_group_and_active_tab() {
        let first = PanelId::from_u64(1);
        let second = PanelId::from_u64(2);
        let third = PanelId::from_u64(3);
        let shape = WorkspaceShape::Split {
            node: None,
            axis: Axis::Horizontal,
            children: vec![
                WorkspaceShape::Tabs {
                    node: None,
                    panels: vec![first, second],
                    active_ix: 1,
                },
                WorkspaceShape::Tabs {
                    node: None,
                    panels: vec![third],
                    active_ix: 0,
                },
            ],
            sizes: vec![None, None],
        };
        assert!(matches!(focused_group_shape(&shape, &[first, second]), Some(WorkspaceShape::Tabs { panels, active_ix: 1, .. }) if panels == vec![first, second]));
        assert!(focused_group_shape(&shape, &[PanelId::from_u64(99)]).is_none());
    }
}

fn collect_ids(state: &PanelState, ids: &mut HashSet<String>, depth: usize) -> Result<(), String> {
    if depth > 32 {
        return Err("Native layout nesting is too deep.".into());
    }
    if state.panel_name == PANEL_NAME {
        let PanelInfo::Panel(value) = &state.info else {
            return Err("Invalid native pane payload.".into());
        };
        let id = value["id"]
            .as_str()
            .filter(|id| !id.is_empty() && id.len() <= 128)
            .ok_or("Invalid native pane identity.")?;
        if !ids.insert(id.into()) {
            return Err("Duplicate native pane identity in layout.".into());
        }
    }
    if state.children.len() > 64 {
        return Err("Native layout contains too many groups.".into());
    }
    for child in &state.children {
        collect_ids(child, ids, depth + 1)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pane(id: &str) -> PanelState {
        PanelState {
            panel_name: PANEL_NAME.into(),
            children: vec![],
            info: PanelInfo::panel(json!({"id":id})),
        }
    }
    #[::core::prelude::v1::test]
    fn persisted_ids_reject_duplicates_and_empty_names() {
        let state = PanelState {
            panel_name: "TabPanel".into(),
            children: vec![pane("chat-a"), pane("chat-a")],
            info: PanelInfo::tabs(0),
        };
        assert!(collect_ids(&state, &mut HashSet::new(), 0).is_err());
        assert!(collect_ids(&pane(""), &mut HashSet::new(), 0).is_err());
    }
    #[::core::prelude::v1::test]
    fn unknown_panels_survive_native_identity_preflight() {
        let state = PanelState {
            panel_name: "FuturePane".into(),
            children: vec![],
            info: PanelInfo::panel(json!({"future":true})),
        };
        assert!(collect_ids(&state, &mut HashSet::new(), 0).is_ok());
    }
    #[::core::prelude::v1::test]
    fn saved_layout_preflight_reports_real_ids_and_rejects_versions() {
        let center = PanelState {
            panel_name: "TabPanel".into(),
            children: vec![pane("route:terminal"), pane("chat:task")],
            info: PanelInfo::tabs(0),
        };
        let layout = json!({"version":1,"center":center});
        assert_eq!(
            DockWorkspace::saved_ids(&layout).unwrap(),
            vec!["chat:task", "route:terminal"]
        );
        let mut invalid = layout;
        invalid["version"] = json!(2);
        assert!(DockWorkspace::saved_ids(&invalid).is_err());
    }
}

#[cfg(all(test, feature = "visual-export"))]
mod workspace_rehost_tests {
    use super::*;
    use ::core::prelude::v1::test;
    use gpui_kit::{TestAppContext, WindowId};

    struct Content;
    impl EventEmitter<()> for Content {}
    impl Render for Content {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full()
        }
    }
    struct Host(Rc<RefCell<Option<Entity<DockWorkspace>>>>);
    impl Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().children(self.0.borrow().clone())
        }
    }

    #[gpui_kit::test]
    fn intact_dock_move_preserves_layout_inactive_contents_and_callback_host(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let events = Rc::new(RefCell::new(Vec::<PaneLifecycle>::new()));
        let callbacks = Rc::new(RefCell::new(Vec::<WindowId>::new()));
        let source_mount = Rc::new(RefCell::new(None));
        let destination_mount = Rc::new(RefCell::new(None));
        let mut retained_dock = None;
        let mut retained_content = None;
        let mut retained_source_host = None;
        let (_, source) = cx.add_window_view(|window, cx| {
            let dock =
                cx.new(|cx| DockWorkspace::new("rehost", Rc::new(|_, _, _| None), window, cx));
            let content = cx.new(|_| Content);
            let inactive = cx.new(|_| Content);
            let capture = events.clone();
            let lifecycle: LifecycleHook =
                Rc::new(move |event, _, _| capture.borrow_mut().push(event));
            dock.update(cx, |dock, cx| {
                dock.set_auto_size_panes(false, window, cx);
                let callback_windows = callbacks.clone();
                cx.subscribe_in(&content, window, move |_, _, _, window, _| {
                    callback_windows
                        .borrow_mut()
                        .push(window.window_handle().window_id());
                })
                .detach();
                let active = PaneSpec {
                    id: "retained:active".into(),
                    title: "active".into(),
                    content: content.clone().into(),
                    focus: cx.focus_handle(),
                    lifecycle: Some(lifecycle.clone()),
                    closable: true,
                };
                dock.add(active, DockPlacement::Center, None, window, cx)
                    .unwrap();
                let inactive = PaneSpec {
                    id: "retained:inactive".into(),
                    title: "inactive".into(),
                    content: inactive.clone().into(),
                    focus: cx.focus_handle(),
                    lifecycle: Some(lifecycle),
                    closable: true,
                };
                dock.add(inactive, DockPlacement::Center, None, window, cx)
                    .unwrap();
                assert!(dock.focus("retained:inactive", window, cx));
                assert_eq!(dock.focused_pane_id(window, cx).as_deref(), Some("retained:inactive"));
                dock.add_focused_empty_tab(window, cx).unwrap();
                let stale = PaneSpec {
                    id: "retained:stale".into(),
                    title: "stale".into(),
                    content: cx.new(|_| Content).into(),
                    focus: cx.focus_handle(),
                    lifecycle: None,
                    closable: true,
                };
                dock.register(stale, cx).unwrap();
                let stale_empty = dock.empty_spec("empty:stale", window, cx).unwrap();
                dock.register(stale_empty, cx).unwrap();
            });
            *source_mount.borrow_mut() = Some(dock.clone());
            retained_dock = Some(dock);
            retained_content = Some(content);
            let host = cx.new(|_| Host(source_mount.clone()));
            retained_source_host = Some(host.clone());
            gpui_kit::component::Root::new(host, window, cx)
        });
        let dock = retained_dock.unwrap();
        let content = retained_content.unwrap();
        let source_host = retained_source_host.unwrap();
        let mut original = None;
        let mut ids = vec![];
        let mut panel_keys = vec![];
        source.update(|_, cx| {
            dock.update(cx, |workspace, cx| {
                let before = workspace.serialize(cx).unwrap();
                workspace.retain_displayed_panels(cx).unwrap();
                assert_eq!(workspace.serialize(cx).unwrap(), before);
                assert!(!workspace.contains("retained:stale"));
                assert!(!workspace.contains("empty:stale"));
                assert!(!workspace.empty_panes.contains_key("empty:stale"));
                assert!(!workspace.empty_subscriptions.contains_key("empty:stale"));
                assert!(workspace.contains("retained:inactive"));
            });
            let workspace = dock.read(cx);
            original = Some(workspace.serialize(cx).unwrap());
            ids = workspace.retained_entity_ids(cx);
            ids.push(dock.entity_id());
            assert_eq!(workspace.retained_content_views(cx).len(), 3);
            panel_keys = workspace
                .panels
                .borrow()
                .iter()
                .map(|(id, panel)| (id.clone(), panel.entity_id()))
                .collect();
            *source_mount.borrow_mut() = None;
            source_host.update(cx, |_, cx| cx.notify());
        });
        events.borrow_mut().clear();
        let mut destination_id = None;
        let (_, destination) = cx.add_window_view(|window, cx| {
            destination_id = Some(window.window_handle().window_id());
            dock.update(cx, |workspace, cx| {
                workspace.rehost_to_window(window, cx).unwrap();
                assert_eq!(workspace.serialize(cx).unwrap(), original.clone().unwrap());
                for (id, key) in &panel_keys {
                    assert_eq!(workspace.panels.borrow()[id].entity_id(), *key);
                }
            });
            *destination_mount.borrow_mut() = Some(dock.clone());
            let host = cx.new(|_| Host(destination_mount.clone()));
            gpui_kit::component::Root::new(host, window, cx)
        });
        destination.update(|_, cx| content.update(cx, |_, cx| cx.emit(())));
        destination.run_until_parked();
        assert_eq!(&*callbacks.borrow(), &[destination_id.unwrap()]);
        assert!(
            !events
                .borrow()
                .iter()
                .any(|event| matches!(event, PaneLifecycle::Removed))
        );
        cx.update(|cx| {
            for id in ids {
                assert_eq!(
                    cx.with_window(id, |window, _| window.window_handle().window_id()),
                    destination_id
                );
            }
        });
    }
}
