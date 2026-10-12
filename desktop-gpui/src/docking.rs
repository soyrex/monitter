//! Native DockArea adapter. Hosts retain pane entities and own filesystem IO.
//! Layout persistence contains stable pane IDs only, never snapshots or secrets.
use crate::design::ui_scaled_px as px;
use crate::empty_pane::{EmptyPane, EmptyPaneAction, EmptyPaneEvent};
use gpui_kit::assets::IconName;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable, WindowExt as _};
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

const PANEL_NAME: &str = "MonitterNativePane";
const VERSION: usize = 1;
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
    EmptySession { pane_id: String, task_id: String },
}
#[derive(Default)]
struct PaintState {
    painted: HashSet<String>,
    visible: HashSet<String>,
    shown: bool,
    frame_callback_pending: bool,
    removed: Vec<(String, bool)>,
}
struct NativePanel {
    spec: PaneSpec,
    paint: Rc<RefCell<PaintState>>,
    removed: bool,
    retaining_layout: Rc<Cell<bool>>,
    appearance: Rc<Cell<Option<crate::pane_appearance::PaneAppearance>>>,
    auto_hide_tabs: Rc<Cell<bool>>,
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
        if let Some(hook) = &self.spec.lifecycle {
            hook(PaneLifecycle::Active(active), window, cx);
        }
    }
    fn on_removed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
        if let Ok(chat) = self.spec.content.clone().downcast::<crate::chat::ChatPane>() {
            chat.update(cx, |chat, cx| chat.set_dock_group(Some(group.clone()), cx));
        }
        if let Ok(browser) = self.spec.content.clone().downcast::<crate::browser::BrowserTab>() {
            browser.update(cx, |browser, cx| browser.set_dock_group(Some(group), cx));
        }
    }
    fn dump(&self, _: &App) -> PanelState {
        PanelState {
            panel_name: PANEL_NAME.into(),
            children: vec![],
            info: PanelInfo::panel(json!({"id":self.spec.id})),
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
            crate::surface_blend::current(crate::surface_blend::Area::Pane).shader()
        } else { 0 }
    }
    fn inner_padding(&self, _: &App) -> bool { false }
    fn toolbar_in_content(&self, _: &App) -> bool {
        self.spec.id.starts_with("chat:") || self.spec.id.starts_with("draft:")
    }
    fn pane_overlay(&mut self, tab: Rc<Cell<Option<gpui::Bounds<Pixels>>>>, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let appearance = self.appearance.get();
        let candidate = appearance.as_ref().is_some_and(|appearance| appearance.show_active_border)
            && pane_visual_active(&self.spec.focus, window, cx)
            && !crate::preferences_modal::workspace_blocked(cx);
        crate::native_pane_highlight::observe_candidate(candidate, window);
        let appearance = appearance.filter(|_| candidate)?;
        let color = appearance.active_border_color;
        if appearance.tile_theme == NativeTileTheme::Tern {
            let mut color = rgb(crate::pane_appearance::quiet_active_color(color, appearance.tile_backdrop_color));
            color.a = appearance.border_opacity as f32 / 100.;
            return Some(canvas(|_, _, _| (), move |bounds, _, window, cx| {
                if crate::preferences_modal::workspace_blocked(cx) { return; }
                let contour = crate::pane_appearance::rounded_contour(bounds, px(8.));
                // Native and GPUI strokes must not stack their alpha.
                if !crate::native_pane_highlight::paint(bounds, color, &contour, window) {
                    window.paint_quad(gpui::outline(bounds, color, gpui::BorderStyle::Solid)
                        .corner_radii(px(8.)));
                }
            }).absolute().inset_0().into_any_element());
        }
        Some(canvas(|_, _, _| (), move |bounds, _, window, cx| {
            if crate::preferences_modal::workspace_blocked(cx) { return; }
            let contour = crate::pane_highlight_geometry::contour(bounds, tab.get());
            crate::native_pane_highlight::paint(bounds, rgb(color), &contour, window);
            let mut path = gpui::PathBuilder::stroke(gpui::px(1.));
            if let Some(first) = contour.first() {
                path.move_to(*first);
                for point in &contour[1..] { path.line_to(*point); }
                path.close();
                if let Ok(path) = path.build() { window.paint_path(path, rgb(color)); }
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
            .child(
                Icon::new(pane_type_icon(&self.spec.id))
                    .with_size(px(13.))
                    .flex_shrink_0(),
            )
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
        let appearance = self.appearance.get();
        let active = pane_visual_active(&self.spec.focus, window, cx);
        let focus = self.spec.focus.clone();
        let paint = self.paint.clone();
        let id = self.spec.id.clone();
        // Decorative animation is a sibling of cached chat content. Its frame
        // notifications must never invalidate the transcript or composer.
        let mut decorations = Vec::new();
        let mut working_bubble = None;
        if let Some(task) = self.spec.id.strip_prefix("chat:") {
            if let Ok(chat) = self.spec.content.clone().downcast::<crate::chat::ChatPane>() {
                let geometry = chat.read(cx).decoration_bounds.clone();
                working_bubble = Some(chat.read(cx).working_bubble.clone());
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
        // Only paint proves effective visibility. Active slots can be hidden,
        // collapsed, displaced by zoom, or outside the host's current route.
        div()
            .size_full()
            .relative()
            .track_focus(&self.spec.focus)
            .when_some(appearance, |this, appearance| {
                this.when(!active && appearance.dim_inactive, |this| {
                    this.opacity(appearance.inactive_opacity)
                })
                .when(appearance.focus_follows_mouse, |this| {
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
                    move |bounds, _, _, _| {
                        if bounds.size.width > px(0.) && bounds.size.height > px(0.) {
                            paint.borrow_mut().painted.insert(id.clone());
                        }
                    },
                )
                .absolute()
                .size_full(),
            )
            .child(
                // Cached prepaint/paint now replay AccessKit nodes and actions.
                // Keep the visibility canvas outside the cached content.
                self.spec.content.clone()
                    .cached(StyleRefinement::default().size_full())
                    .into_any_element()
            )
            .children(decorations)
            .when_some(working_bubble, |this, bubble| this.child(bubble))
    }
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
            panel_handle(panel)
        });
        let changed = cx.subscribe(&area, |_, _, event, cx| {
            if matches!(event, DockEvent::LayoutChanged) {
                cx.emit(WorkspaceEvent::LayoutChanged);
                cx.notify();
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
        }
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
                radius: px(8.),
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
        self.auto_hide_tabs.set(hidden);
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
        let panel = cx.new(|_| NativePanel {
            spec,
            paint,
            removed: false,
            retaining_layout: self.retaining_layout.clone(),
            appearance: self.appearance.clone(),
            auto_hide_tabs: self.auto_hide_tabs.clone(),
        });
        self.panels.borrow_mut().insert(id, panel.clone());
        Ok(panel)
    }
    /// Preflight a real retained pane without changing the displayed layout.
    pub fn register(&mut self, spec: PaneSpec, cx: &mut Context<Self>) -> Result<(), String> {
        self.retain(spec, cx).map(|_| ())
    }
    pub fn has_panes(&self) -> bool {
        !self.panels.borrow().is_empty()
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
    pub fn focused_group_node(&self, window: &Window, cx: &App) -> Option<NodeId> {
        let focused_id = self.focused_pane_id(window, cx)?;
        let focused_key = self.panel_key(&focused_id)?;
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

    /// Move the focused tab into the adjacent pane, or create a directional
    /// split beside its current group when no neighbor exists.
    pub fn move_focused_tab(
        &mut self,
        direction: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(id) = self.focused_pane_id(window, cx) else {
            return false;
        };
        let Some(key) = self.panel_key(&id) else {
            return false;
        };
        let Some(group) = self.focused_group_node(window, cx) else {
            return false;
        };
        let target = {
            let area = self.area.read(cx);
            let source = collect_workspace_frames(&area, cx)
                .into_iter()
                .find(|frame| frame.panels.contains(&key));
            source.and_then(|source| {
                collect_workspace_frames(&area, cx)
                    .into_iter()
                    .filter(|frame| !frame.panels.contains(&key))
                    .filter_map(|frame| {
                        directional_score(source.rect, frame.rect, direction)
                            .map(|score| (score, frame))
                    })
                    .min_by(|a, b| a.0.total_cmp(&b.0))
                    .map(|(_, frame)| frame)
            })
        };
        if let Some(frame) = target {
            if let Some(target_key) = frame.panels.get(frame.active_ix).copied() {
                if let Some(node) = {
                    let area = self.area.read(cx);
                    [
                        DockPlacement::Center,
                        DockPlacement::Left,
                        DockPlacement::Right,
                        DockPlacement::Bottom,
                    ]
                    .into_iter()
                    .filter_map(|placement| area.layout(placement))
                    .find_map(|tree| tree.find_panel_node(target_key))
                } {
                    if self.move_pane(
                        &id,
                        InsertTarget::Tabs {
                            node,
                            ix: None,
                            activate: true,
                        },
                        window,
                        cx,
                    ) {
                        return self.focus(&id, window, cx);
                    }
                }
            }
            return false;
        }
        let placement = match direction {
            "left" => gpui_kit::component::Placement::Left,
            "right" => gpui_kit::component::Placement::Right,
            "up" => gpui_kit::component::Placement::Top,
            "down" => gpui_kit::component::Placement::Bottom,
            _ => return false,
        };
        self.area.update(cx, |area, cx| {
            // split_at inserts another leaf; this shortcut relocates the
            // existing pane and must detach its source in the same edit.
            area.move_panel(key, InsertTarget::Split { node: group, placement, size: None }, window, cx)
        });
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
        self.install_shape(placement, shape, window, cx)
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
            InsertTarget::Split {
                node: group,
                placement,
                size: size.map(|size| px(size as f32)),
            },
            window,
            cx,
        ) {
            self.remove(&id, window, cx);
            return Ok(false);
        }
        self.focus(&id, window, cx);
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

    /// Equalize split proportions while keeping panel entities and active tabs.
    pub fn equalize_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let focused = self.focused_pane_id(window, cx);
        let area_entity = self.area();
        let layouts = {
            let area = area_entity.read(cx);
            [
                DockPlacement::Center,
                DockPlacement::Left,
                DockPlacement::Right,
                DockPlacement::Bottom,
            ]
            .into_iter()
            .filter_map(|placement| {
                let tree = area.layout(placement)?;
                let layout = equalized_layout(tree.root(), &area, cx)?;
                Some((placement, layout))
            })
            .collect::<Vec<_>>()
        };
        if layouts.is_empty() {
            return false;
        }
        area_entity.update(cx, |area, cx| {
            for (placement, layout) in layouts {
                if placement == DockPlacement::Center {
                    area.set_center(layout, window, cx);
                } else {
                    area.set_dock(placement, layout, window, cx);
                }
            }
        });
        focused.is_some_and(|id| self.focus(&id, window, cx))
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
        true
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
        self.area
            .update(cx, |area, cx| area.load(state, window, cx))
            .map_err(|error| error.to_string())?;
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
        let prior = std::mem::replace(&mut self.paint.borrow_mut().visible, next.clone());
        for id in prior.symmetric_difference(&next) {
            let visible = next.contains(id);
            if let Some(panel) = self.panels.borrow().get(id) {
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
        // A rendered workspace is an authoritative observation point even if
        // it currently has no active panel. A focused NativePanel may promote
        // this to `true` while its TabGroupRenderer builds the deferred outline.
        crate::native_pane_highlight::observe_candidate(false, window);
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
            window.on_next_frame(move |window, cx| {
                let _ = weak.update(cx, |workspace, cx| {
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
        div().size_full().child(self.area.clone())
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

fn equalized_layout(node: &PaneNode, area: &DockArea, cx: &App) -> Option<DockLayout> {
    match node.kind() {
        PaneRef::Tabs { panels, active_ix } => {
            let mut layout = DockLayout::tabs();
            for panel in panels {
                layout = layout.panel_view(area.panel(*panel)?.clone(), cx);
            }
            Some(layout.active_index(active_ix))
        }
        PaneRef::Split { axis, children, .. } => {
            let mut layout = if axis == Axis::Horizontal {
                DockLayout::h_split()
            } else {
                DockLayout::v_split()
            };
            for child in children {
                layout = layout.child(equalized_layout(child, area, cx)?, None);
            }
            Some(layout)
        }
    }
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
