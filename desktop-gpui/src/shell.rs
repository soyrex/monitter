//! Native sidebar with agent, project and activity views plus route controls.
use crate::workspace_windows::DesktopContextExt as _;
use super::*;
use crate::design::ui_scaled_px as px;

// Keep sidebar text in step with the interface font setting while preserving
// the compact hierarchy used by secondary labels and metadata.
fn sidebar_text_px(value: f32, settings: &Value) -> Pixels {
    let font_ratio = settings["interfaceFontSize"]
        .as_f64()
        .unwrap_or(14.)
        .clamp(8., 32.) as f32
        / 14.;
    px(value * 0.9 * font_ratio)
}

fn sidebar_primary_text_px(font_size: Pixels) -> Pixels {
    ((font_size - px(4.)).max(px(8.))) * 1.1
}
use gpui_kit::{
    assets::{__private as icons_assets, IconName},
    component::menu::{DropdownMenu, PopupMenuItem},
    component::{ActiveTheme as _, Icon, Sizable, button::*},
};

#[path = "sidebar_compact.rs"]
mod sidebar_compact;
#[path = "sidebar_list.rs"]
mod sidebar_list;
#[path = "sidebar_hover.rs"]
mod sidebar_hover;

use crate::collaboration::completion_feedback::CompletionMarker;

#[derive(Default)]
struct TaskMarkers {
    markers: HashMap<(String, u8), Entity<CompletionMarker>>,
    visible_view: Option<u8>,
    statuses: Option<Arc<HashMap<String, String>>>,
}
impl Global for TaskMarkers {}
fn marker_view(view: SidebarView) -> u8 {
    match view {
        SidebarView::Agents => 0,
        SidebarView::Projects => 1,
        SidebarView::Activity => 2,
    }
}
fn ensure_markers(cx: &mut App) {
    if cx.try_global::<TaskMarkers>().is_none() {
        cx.set_global(TaskMarkers::default());
    }
}
pub(crate) fn set_task_markers_visible(visible: bool, view: SidebarView, cx: &mut App) {
    ensure_markers(cx);
    let next = visible.then_some(marker_view(view));
    if cx.global::<TaskMarkers>().visible_view == next {
        return;
    }
    cx.global_mut::<TaskMarkers>().visible_view = next;
    let markers = cx.global::<TaskMarkers>().markers.clone();
    for ((_, view), marker) in markers {
        marker.update(cx, |marker, cx| marker.set_visible(next == Some(view), cx));
    }
}
pub(crate) fn sync_task_markers(
    index: &monitter_gpui_desktop::native_publication::PublicationIndex,
    reset: bool,
    cx: &mut App,
) {
    ensure_markers(cx);
    let statuses = index.task_statuses();
    if !reset
        && cx
            .global::<TaskMarkers>()
            .statuses
            .as_ref()
            .is_some_and(|previous| Arc::ptr_eq(previous, &statuses))
    {
        return;
    }
    cx.global_mut::<TaskMarkers>().statuses = Some(statuses);
    if cx.global::<TaskMarkers>().markers.is_empty() {
        return;
    }
    cx.global_mut::<TaskMarkers>()
        .markers
        .retain(|(id, _), _| index.task_status(id).is_some());
    let registry = cx.global::<TaskMarkers>();
    let visible_view = registry.visible_view;
    let markers = registry.markers.clone();
    for ((id, view), marker) in markers {
        let Some(status) = index.task_status(&id) else {
            continue;
        };
        marker.update(cx, |marker, cx| {
            if reset {
                marker.suppress(cx);
            }
            marker.set_status(&id, status, !reset && visible_view == Some(view), cx);
        });
    }
}
fn task_marker(
    task: &Value,
    view: SidebarView,
    palette: design::Palette,
    cx: &mut App,
) -> Entity<CompletionMarker> {
    ensure_markers(cx);
    let id = field(task, "id");
    let status = field(task, "status");
    let key = (id.clone(), marker_view(view));
    let marker = cx
        .global::<TaskMarkers>()
        .markers
        .get(&key)
        .cloned()
        .unwrap_or_else(|| {
            let marker = cx.new(|_| CompletionMarker::new(&id, &status));
            cx.global_mut::<TaskMarkers>()
                .markers
                .insert(key.clone(), marker.clone());
            marker
        });
    let visible = cx.global::<TaskMarkers>().visible_view == Some(key.1);
    let icon = match status.as_str() {
        "running" => "◌",
        "completed" => "✓",
        "error" => "!",
        "interrupted" => "■",
        _ => "",
    };
    marker.update(cx, |marker, cx| {
        marker.set_visible(visible, cx);
        marker.set_appearance(icon, design::status_color(&status, palette), 14., cx);
    });
    marker
}

struct ProjectCollapseState {
    collapsed: bool,
}

/// Three primary sidebar views matching the production Tauri `.sidebar-tab`
/// strip. Selecting one only updates the sidebar view; it does not switch the
/// main content surface.
struct ViewTab {
    view: SidebarView,
    label: &'static str,
    icon: &'static [u8],
}

const VIEWS: &[ViewTab] = &[
    ViewTab {
        view: SidebarView::Agents,
        label: "Agents",
        icon: icons_assets::Bot.1,
    },
    ViewTab {
        view: SidebarView::Projects,
        label: "Projects",
        icon: icons_assets::Folder.1,
    },
    ViewTab {
        view: SidebarView::Activity,
        label: "Activity",
        icon: icons_assets::Activity.1,
    },
];

/// Frequent workspace destinations shown as direct footer actions.
struct BrandAction {
    surface: Sidebar,
    label: &'static str,
    icon: &'static [u8],
}

const BRAND_ACTIONS: &[BrandAction] = &[
    BrandAction {
        surface: Sidebar::Terminal,
        label: "New terminal",
        icon: icons_assets::Terminal.1,
    },
    BrandAction {
        surface: Sidebar::Chats,
        label: "New chat",
        icon: icons_assets::MessageSquarePlus.1,
    },
    BrandAction {
        surface: Sidebar::Subagents,
        label: "Companion mode",
        icon: icons_assets::Bot.1,
    },
    BrandAction {
        surface: Sidebar::Settings,
        label: "Open settings",
        icon: icons_assets::Settings.1,
    },
];

fn sidebar_task_row_frame() -> Div {
    div().px(px(design::space::INSET))
}

impl Desktop {
    /// Narrow icon rail used when the workspace sidebar is collapsed.
    pub(super) fn render_compact_sidebar(
        &self,
        agents: &[&Value],
        tasks: &[&Value],
        palette: design::Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        sidebar_compact::render(self, agents, tasks, palette, window, cx)
    }

    fn task_row(
        &self,
        task: &Value,
        context: Option<String>,
        palette: design::Palette,
        id_prefix: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let task_id = field(task, "id");
        let older = task_is_older(task, sidebar_now_ms());
        let active = self.window.selected.as_ref() == Some(&task_id);
        let status = field(task, "status");
        let completion_marker = task_marker(task, self.window.sidebar_view, palette, cx);
        let provider = provider_label(&self.snapshot, task);
        let model = field(task, "model");
        let project_name = context.filter(|name| !name.is_empty());
        let updated_at = task
            .get("updatedAt")
            .and_then(Value::as_i64)
            .filter(|time| *time > 0);
        let relative_time = updated_at.map(relative_task_time);
        let title = field(task, "title");
        let hover_key = format!("{id_prefix}-{task_id}");
        let (hover_state, background_opacity) =
            sidebar_hover::state(&hover_key, active, window, cx);
        let provider_icon =
            crate::branding::provider_for_ink(&field(task, "provider"), &model, palette.ink)
                .map(|image| img(image).w(px(17.)).h(px(17.)).into_any_element())
                .unwrap_or_else(|| {
                    Icon::new(IconName::MessageSquare)
                        .size(px(16.))
                        .into_any_element()
                });
        let meta = if let Some(project_name) = project_name.clone() {
            if provider.is_empty() {
                div().flex_1().min_w_0().truncate().child(project_name)
            } else {
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(3.))
                    .child(div().min_w_0().truncate().child(provider.clone()))
                    .child("·")
                    .child(div().min_w_0().truncate().child(project_name))
            }
        } else {
            div().flex_1().min_w_0().truncate().child(provider.clone())
        };
        let mut accessible_parts = vec![title.clone()];
        if !provider.is_empty() {
            accessible_parts.push(provider.clone());
        }
        if let Some(project) = project_name.as_ref() {
            accessible_parts.push(project.clone());
        }
        if !status.is_empty() {
            accessible_parts.push(status.clone());
        }
        if let Some(time) = relative_time.as_ref() {
            accessible_parts.push(time.clone());
        }
        let accessible_label = accessible_parts.join(" · ");
        sidebar_task_row_frame()
            .id(SharedString::from(hover_key))
            .role(Role::Button)
            .aria_label(accessible_label)
            .flex_shrink_0()
            .py(px(8.))
            .rounded_md()
            .cursor_pointer()
            .relative()
            .overflow_hidden()
            .bg(crate::design::surface_color(palette.soft).opacity(background_opacity))
            .flex()
            .items_center()
            .gap(px(design::space::TIGHT))
            .children(crate::activity_effects::for_task(
                &task_id,
                id_prefix,
                crate::activity_sparkles::SparkleLayout::Compact,
                false,
                cx,
            ))
            .child(
                div()
                    .w(px(30.))
                    .h(px(30.))
                    .flex_shrink_0()
                    .rounded_md()
                    .relative()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(crate::design::surface_color(palette.panel))
                    .child(provider_icon)
                    .child(
                        div()
                            .absolute()
                            .right(px(1.))
                            .bottom(px(1.))
                            .w(px(design::STATUS_DOT))
                            .h(px(design::STATUS_DOT))
                            .rounded_full()
                            .bg(rgb(if status == "running" {
                                palette.accent
                            } else {
                                design::status_color(&status, palette)
                            })),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(sidebar_primary_text_px(cx.theme().font_size))
                            .font_weight(if older {
                                FontWeight::THIN
                            } else {
                                FontWeight::SEMIBOLD
                            })
                            .text_color(rgb(palette.ink).opacity(if older { 0.7 } else { 1. }))
                            .child(title),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap(px(3.))
                            .text_size(sidebar_text_px(10., &self.snapshot["settings"]))
                            .font_weight(if older {
                                FontWeight::THIN
                            } else {
                                FontWeight::NORMAL
                            })
                            .text_color(rgb(palette.muted).opacity(if older { 0.7 } else { 1. }))
                            .child(meta)
                            .when_some(relative_time, |row, time| {
                                row.child(div().flex_shrink_0().whitespace_nowrap().child(time))
                            }),
                    ),
            )
            .when(
                matches!(
                    status.as_str(),
                    "running" | "completed" | "error" | "interrupted"
                ),
                |row| row.child(completion_marker),
            )
            .on_click(cx.workspace_listener(move |view, _, window, cx| {
                view.navigate_sidebar_task(task_id.clone(), window, cx);
            }))
            .on_hover(cx.workspace_listener(move |_, hovered, _, cx| {
                sidebar_hover::set(&hover_state, *hovered, cx);
            }))
            .into_any_element()
    }

    fn project_row(
        project: &Value,
        chat_count: usize,
        running: bool,
        collapsed: bool,
        collapse: Entity<ProjectCollapseState>,
        palette: design::Palette,
        metadata_font_size: Pixels,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let name = field(project, "name");
        let icon = project_icon(&field(project, "icon"));
        let color = design::parse_color(&field(project, "color"), palette.accent);
        let count = format!(
            "{chat_count} {}",
            if chat_count == 1 { "chat" } else { "chats" }
        );
        let mut tile = div()
            .w(px(38.))
            .h(px(38.))
            .flex_shrink_0()
            .rounded_lg()
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .bg(crate::design::surface_color(color).opacity(0.16))
            .text_color(rgb(color))
            .child(Icon::new(icon).size(px(17.)));
        if running {
            tile = tile.child(
                div()
                    .absolute()
                    .right(px(1.))
                    .bottom(px(1.))
                    .w(px(design::STATUS_DOT))
                    .h(px(design::STATUS_DOT))
                    .rounded_full()
                    .bg(rgb(palette.accent)),
            );
        }
        let copy = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(sidebar_primary_text_px(cx.theme().font_size))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(palette.ink))
                    .child(name.clone()),
            )
            .child(
                div()
                    .text_size(metadata_font_size)
                    .text_color(rgb(palette.muted))
                    .child(count.clone()),
            );
        let project_id = field(project, "id");
        div()
            .id(SharedString::from(format!("project-group-{project_id}")))
            .flex_shrink_0()
            .w_full()
            .px(px(design::space::TIGHT))
            .py(px(design::space::TIGHT))
            .flex()
            .items_center()
            .gap(px(design::space::TIGHT))
            .child(tile)
            .child(copy)
            .child(
                Icon::new(if collapsed {
                    IconName::ChevronRight
                } else {
                    IconName::ChevronDown
                })
                .size(px(13.))
                .text_color(rgb(palette.muted)),
            )
            .role(Role::Button)
            .aria_label(format!(
                "{} project, {}. {}",
                if collapsed { "Expand" } else { "Collapse" },
                name,
                count
            ))
            .cursor_pointer()
            .on_click(cx.workspace_listener(move |_, _, _, cx| {
                collapse.update(cx, |state, _| state.collapsed = !state.collapsed);
                cx.notify();
            }))
    }

    fn section_label(label: &str, palette: design::Palette, settings: &Value) -> Div {
        div()
            .flex_shrink_0()
            .px(px(design::space::INSET))
            .pt(px(design::space::SECTION))
            .pb(px(design::space::TIGHT))
            .text_size(sidebar_text_px(10., settings))
            .text_color(rgb(palette.muted))
            .font_family(design::FONT_MONO)
            .child(
                div()
                    .debug_selector(|| "sidebar-section-copy".into())
                    .child(label.to_uppercase()),
            )
    }

    fn sidebar_agent_row(&self, agent: &Value, collapsed: bool, palette: design::Palette,
        window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let id = field(agent, "id");
        let color = design::parse_color(&field(agent, "color"), palette.accent);
        let avatar = field(agent, "name")
            .chars()
            .next()
            .unwrap_or('?')
            .to_string();
        let toggle = id.clone();
        let new_id = id.clone();
        let agent_row_id = format!("agent-row-{id}");
        let (agent_hover_state, agent_background_opacity) =
            sidebar_hover::state(&agent_row_id, false, window, cx);
        let row = div()
                .id(SharedString::from(agent_row_id))
                .flex_shrink_0()
                .rounded_md()
                .relative()
                .overflow_hidden()
                .bg(crate::design::surface_color(palette.soft)
                    .opacity(agent_background_opacity))
                .flex()
                .items_center()
                .gap(px(design::space::TIGHT))
                .py(px(design::space::INSET))
                .px(px(design::space::INSET))
                .child(
                    div()
                        .id(SharedString::from(format!("agent-toggle-{id}")))
                        .w(px(design::AGENT_AVATAR_SIZE))
                        .h(px(design::AGENT_AVATAR_SIZE))
                        .rounded_md()
                        .bg(rgb(color))
                        .text_color(rgb(0xffffff))
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .child(if collapsed { "›".into() } else { avatar })
                        .on_click(cx.workspace_listener(move |view, _, _, cx| {
                            if !view.window.collapsed_agents.remove(&toggle) {
                                view.window.collapsed_agents.insert(toggle.clone());
                            }
                            cx.notify();
                        })),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(design::space::TIGHT))
                        .child(div().text_size(sidebar_primary_text_px(cx.theme().font_size)).child(field(agent, "name")))
                        .child(
                            div()
                                .text_size(sidebar_text_px(10., &self.snapshot["settings"]))
                                .text_color(rgb(palette.muted))
                                .child(format!(
                                    "{} · {}",
                                    field(agent, "provider"),
                                    field(agent, "model")
                                )),
                        ),
                )
                .child(
                    Button::new(SharedString::from(format!("new-{id}")))
                        .compact()
                        .label("+")
                        .disabled(self.window.busy)
                        .on_click(cx.workspace_listener(move |view, _, window, cx| {
                            view.open_chat_draft(Some(new_id.clone()), window, cx);
                        })),
                )
                .on_hover(cx.workspace_listener(move |_, hovered, _, cx| {
                    sidebar_hover::set(&agent_hover_state, *hovered, cx);
                }));
        row.into_any_element()
    }

    pub(super) fn render_sidebar(
        &self,
        agents: &[&Value],
        tasks: &[&Value],
        palette: design::Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tern = design::tile_theme() == monitter_lib::experimental_desktop::NativeTileTheme::Tern;
        let tree = sidebar_list::render(self, agents, tasks, palette, window, cx);

        // Three primary sidebar tabs (Agents / Projects / Activity) — match the
        // These views change the sidebar listing while leaving the selected
        // main content surface intact.
        let active_view = self.window.sidebar_view;
        self.window.sidebar_tabs_motion
            .borrow_mut()
            .observe(match active_view {
                SidebarView::Agents => 0,
                SidebarView::Projects => 1,
                SidebarView::Activity => 2,
            });
        let tree = self.window.sidebar_tabs_motion.borrow().body(
            "primary-sidebar",
            tree.into_any_element(),
            cx.reduce_motion(),
            true,
        );
        let modern_tabs = field(&self.snapshot["settings"], "tabStyle") == "modern";
        let modern_height: f32 =
            match field(&self.snapshot["settings"], "interfaceDensity").as_str() {
                "tight" => 28.,
                "spacious" => 36.,
                _ => 30.,
            };
        let mut tabs = div()
            .id("sidebar-view-tabs")
            .role(Role::TabList)
            .flex()
            .flex_row()
            .items_end()
            .gap(px(if modern_tabs {
                0.
            } else {
                design::SIDEBAR_TABS_GAP
            }))
            .px(px(if modern_tabs {
                0.
            } else {
                design::SIDEBAR_TABS_ROW_PAD_X
            }))
            .pt(px(if modern_tabs {
                0.
            } else {
                design::SIDEBAR_TABS_ROW_PAD_TOP
            }))
            .pb_0()
            .min_h(px(if modern_tabs {
                modern_height
            } else {
                design::SIDEBAR_TABS_ROW_MIN_HEIGHT
            }))
            .h(px(if modern_tabs { modern_height } else { 35. }))
            .when(modern_tabs, |row| row.items_stretch())
            .border_color(rgb(palette.line))
            .border_blend_mode(crate::surface_blend::separator_mode());
        for (tab_index, tab) in VIEWS.iter().enumerate() {
            let active = active_view == tab.view;
            let next_view = tab.view;
            let label = SharedString::from(tab.label.to_ascii_uppercase());
            tabs = tabs.child(
                div()
                    .id(SharedString::from(format!(
                        "sidebar-view-{}",
                        tab.label.to_ascii_lowercase()
                    )))
                    .role(Role::Tab)
                    .aria_selected(active)
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(design::SIDEBAR_TAB_GAP))
                    .min_h(px(design::SIDEBAR_TAB_HEIGHT))
                    .px(px(design::SIDEBAR_TAB_ENTRY_PADDING))
                    .rounded_t(px(if modern_tabs {
                        0.
                    } else {
                        design::SIDEBAR_TAB_RADIUS
                    }))
                    .border_1()
                    .border_color(rgb(if active {
                        palette.line
                    } else {
                        palette.sidebar
                    }))
                    .when(modern_tabs, |entry| {
                        entry
                            .border_0()
                            .border_t_1()
                            .border_color(rgb(palette.line))
                            .border_blend_mode(crate::surface_blend::separator_mode())
                            .when(tab_index + 1 < VIEWS.len(), |entry| entry.border_r_1())
                            .when(!active, |entry| entry.border_b_1())
                    })
                    .when(active, |entry| entry.border_b_0())
                    .cursor_pointer()
                    .text_color(rgb(if active { palette.ink } else { palette.muted }))
                    .aria_label(tab.label)
                    .child(
                        Icon::default()
                            .data(tab.icon)
                            .text_color(rgb(if active { palette.ink } else { palette.muted }))
                            .with_size(px(design::ICON_SIZE_XS)),
                    )
                    .child(
                        div()
                            .text_size(sidebar_primary_text_px(cx.theme().font_size))
                            .font_family(design::FONT_MONO)
                            .child(label),
                    )
                    .on_click(cx.workspace_listener(move |desktop, _, _, cx| {
                        desktop.window.sidebar_view = next_view;
                        cx.notify();
                    })),
            );
        }

        // Share the workspace pill's outer height and glyph sizing, including
        // interface font changes. Button::icon would replace our glyph size.
        let tab_metrics = design::workspace_tab_metrics(cx.theme().font_size);
        // Fade the glyphs before the centred group slides underneath the
        // native traffic lights. Use physical points: AppKit controls do not
        // scale with the application's font or interface scale.
        let sidebar_points = f32::from(design::ui_px(
            self.displayed_sidebar_width(cx), self.window.sidebar_scale,
        ));
        let group_points = f32::from(tab_metrics.height) * 3. + f32::from(px(2.)) * 2.;
        let glyph_left = (sidebar_points - group_points) / 2.
            + f32::from(tab_metrics.height - tab_metrics.icon_size) / 2.;
        let navigation_opacity = ((glyph_left - 72.) / 12.).clamp(0., 1.);
        let mut tern_view_toggles = div()
            .id("tern-sidebar-view-toggles")
            .opacity(navigation_opacity)
            .flex()
            .items_center()
            .gap(px(2.));
        for tab in VIEWS {
            let next_view = tab.view;
            let active = active_view == tab.view;
            tern_view_toggles = tern_view_toggles.child(
                div()
                    .w(tab_metrics.height)
                    .h(tab_metrics.height)
                    .flex_shrink_0()
                    .rounded(px(4.))
                    .overflow_hidden()
                    .border_1()
                    .border_color(rgb(palette.accent).opacity(if active { 0.65 } else { 0. }))
                    .bg(rgb(palette.accent).opacity(if active { 0.14 } else { 0. }))
                    .child(
                        Button::new(SharedString::from(format!(
                            "tern-sidebar-view-{}",
                            tab.label.to_ascii_lowercase()
                        )))
                        .ghost()
                        .compact()
                        .w_full()
                        .h_full()
                        .p_0()
                        .rounded(px(4.))
                        .child(
                            Icon::default()
                                .data(tab.icon)
                                .with_size(tab_metrics.icon_size)
                                .text_color(rgb(if active { palette.accent } else { palette.ink })),
                        )
                        .tooltip(tab.label)
                        .accessibility_label(tab.label)
                        .toggled(active)
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.workspace_listener(
                            move |desktop, _, _, cx| {
                                desktop.window.sidebar_view = next_view;
                                cx.notify();
                            },
                        )),
                    ),
            );
        }

        let density = field(&self.snapshot["settings"], "interfaceDensity");
        let chat_header_height = match density.as_str() {
            "tight" => 26. + 2. * design::space::TIGHT,
            "spacious" => 34. + 2. * design::space::GROUP,
            _ => 30. + 2. * design::space::INSET,
        };
        // Align the combined logo + sidebar tabs with the actual dock strip
        // + chat header. Classic dock strips use native component points.
        let sidebar_tabs_height = if modern_tabs { modern_height } else { 35. };
        let interface_scale = design::interface_scale() as f32 / 100.;
        let dock_strip_height = design::workspace_tabbar_height_points(
            &self.snapshot["settings"],
            design::interface_scale(),
        ) / interface_scale;
        let brand_row_height =
            (dock_strip_height + chat_header_height - sidebar_tabs_height).max(0.);
        let mut logo_shade: Background = rgba(0x0000001a).into();
        logo_shade.blend_mode = 2;
        let brand_row = div()
            .when(!tern, |row| row.bg(logo_shade))
            .h(px(brand_row_height))
            .px(px(design::space::INSET))
            .gap_2()
            .on_mouse_down(MouseButton::Left, |event, window, _| {
                if event.click_count == 2 {
                    window.titlebar_double_click();
                } else {
                    window.start_window_move();
                }
            })
            .flex()
            .items_center()
            .justify_center()
            .border_b_1()
            .border_color(rgb(palette.line))
            .border_blend_mode(crate::surface_blend::separator_mode())
            // Optical size targets 70% of the previous rendered mark, which was header-height constrained.
            .child(
                img(branding::sidebar_wordmark(
                    91.2 * interface_scale * window.scale_factor(),
                ))
                .w(px(91.2))
                .h(px(30.4))
                .max_w_full()
                .max_h_full()
                .object_fit(ObjectFit::Contain),
            );

        let usage_width = self.window.sidebar_resize
            .read(cx)
            .sizes()
            .first()
            .map(|width| *width / design::ui_px(1., self.window.sidebar_scale))
            .filter(|width| width.is_finite() && *width > 0.)
            .unwrap_or_else(|| self.current_sidebar_width(cx));
        let usage_width = if tern {
            (usage_width - 12. / interface_scale).max(0.)
        } else {
            usage_width
        };
        let usage_stats = self.window.sidebar_usage.as_ref().map(|usage| {
            usage.update(cx, |usage, cx| {
                usage.set_composed_palette(palette, usage_width, cx)
            });
            usage.clone()
        });
        let pending = sidebar_rows(&self.snapshot, "approvalRequests")
            .iter()
            .filter(|r| field(r, "status") == "pending" && r["input"].is_null())
            .collect::<Vec<_>>();
        let approval_block = if pending.is_empty() {
            None
        } else {
            let mut block = div()
                .id("sidebar-pending-approvals")
                .flex_shrink_0()
                .max_h(px(160.))
                .overflow_y_scroll()
                .px(px(design::space::INSET))
                .py(px(design::space::TIGHT))
                .flex()
                .flex_col()
                .font_family(cx.theme().font_family.clone())
                .text_size(cx.theme().font_size * 0.9);
            for request in pending {
                let approval_id = field(&request, "id");
                let task_id = field(&request, "taskId");
                let title = tasks
                    .iter()
                    .find(|t| field(t, "id") == task_id)
                    .map(|t| field(t, "title"))
                    .unwrap_or_else(|| "Agent request".into());
                block = block.child(
                    div()
                        .id(SharedString::from(format!(
                            "sidebar-approval-{}",
                            approval_id
                        )))
                        .role(Role::Button)
                        .aria_label(format!("Review approval for {title}"))
                        .h((cx.theme().font_size * 1.4 + px(6.)).max(px(28.)))
                        .flex_shrink_0()
                        .px(px(design::space::TIGHT))
                        .rounded_md()
                        .cursor_pointer()
                        .hover(|s| s.bg(design::surface_color(palette.soft)))
                        .flex()
                        .items_center()
                        .gap(px(design::space::TIGHT))
                        .min_w_0()
                        .text_color(rgb(palette.accent))
                        .child(
                            Icon::new(IconName::TriangleAlert)
                                .size(px(14.))
                                .flex_shrink_0(),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .child(format!("Review approval · {title}")),
                        )
                        .on_click(cx.workspace_listener(move |view, _, window, cx| {
                            view.switch_surface(Sidebar::Chats, window, cx);
                            view.window.selected = Some(task_id.clone());
                            let pane = view.chat_panes.borrow().get(&task_id).cloned();
                            if let Some(pane) = pane {
                                pane.update(cx, |pane, cx| {
                                    pane.show_approval_prompt(approval_id.clone(), window, cx)
                                });
                            }
                            cx.notify();
                        })),
                );
            }
            Some(block)
        };
        let sidebar = div()
            .id("sidebar-root")
            .text_size(cx.theme().font_size * 0.9)
            .on_scroll_wheel(cx.workspace_listener(|desktop, event: &ScrollWheelEvent, _, cx| {
                let delta = event.delta.pixel_delta(gpui_kit::px(20.));
                if let Some(index) = desktop.window.sidebar_tabs_motion.borrow_mut().wheel(
                    delta.x.into(),
                    delta.y.into(),
                    3,
                    std::time::Instant::now(),
                ) {
                    desktop.window.sidebar_view = [
                        SidebarView::Agents,
                        SidebarView::Projects,
                        SidebarView::Activity,
                    ][index];
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .w_full()
            .h_full()
            .flex()
            .flex_col()
            .border_color(rgb(palette.line))
            .border_blend_mode(crate::surface_blend::separator_mode())
            .when(!tern, |sidebar| {
                sidebar.bg(crate::surface_blend::background(
                    crate::surface_blend::Area::Sidebar,
                    palette.sidebar,
                ))
            });
        let footer = div()
            .id("sidebar-footer")
            .flex_shrink_0()
            .w_full()
            .flex()
            .flex_col()
            .children(usage_stats);
        let sidebar = if tern {
            let outer_pad = design::tern_spacing(&self.snapshot["settings"]).gutter / 2.;
            let header = div()
                .id("tern-sidebar-titlebar-row")
                .absolute()
                .top(px(-outer_pad))
                .left(px(-outer_pad))
                .right(px(-outer_pad))
                .h(tab_metrics.row_height)
                .flex()
                .items_center()
                .justify_center()
                .child(tern_view_toggles)
                .on_mouse_down(MouseButton::Left, |event, window, _| {
                    if event.click_count == 2 {
                        window.titlebar_double_click();
                    } else {
                        window.start_window_move();
                    }
                });
            let sidebar_content_top = (tab_metrics.row_height + tab_metrics.height) / 2.
                + px(design::tern_spacing(&self.snapshot["settings"]).gutter - outer_pad);
            sidebar
                .p(px(
                    design::tern_spacing(&self.snapshot["settings"]).gutter / 2.
                ))
                .gap(px(design::tern_spacing(&self.snapshot["settings"]).gutter))
                .child(
                    div()
                        .id("sidebar-navigation")
                        .relative()
                        .flex_1()
                        .min_h_0()
                        .w_full()
                        .flex()
                        .flex_col()
                        .pt(sidebar_content_top)
                        .child(header)
                        .children(approval_block)
                        .child(tree),
                )
                .child(footer)
        } else {
            sidebar
                .child(brand_row)
                .child(tabs)
                .children(approval_block)
                .child(tree)
                .child(footer)
        };
        sidebar.into_any_element()
    }
}

// Sidebar rendering only reads metadata. Borrow the snapshot arrays instead of
// deep-cloning every project (including its documents) for each visible task.
fn sidebar_rows<'a>(snapshot: &'a Value, key: &str) -> &'a [Value] {
    snapshot.get(key).and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
}

fn project_name(snapshot: &Value, task: &Value) -> Option<String> {
    let project_id = field(task, "projectId");
    if project_id.is_empty() {
        return None;
    }
    sidebar_rows(snapshot, "projects")
        .iter()
        .find(|project| project["id"].as_str() == Some(project_id.as_str()))
        .map(|project| field(project, "name"))
        .filter(|name| !name.is_empty())
}

fn provider_label(snapshot: &Value, task: &Value) -> String {
    let agent_id = field(task, "agentId");
    let agent = sidebar_rows(snapshot, "agents")
        .iter()
        .find(|agent| agent["id"].as_str() == Some(agent_id.as_str()));
    let provider = {
        let task_provider = field(task, "provider");
        if task_provider.is_empty() {
            agent
                .as_ref()
                .map(|agent| field(agent, "provider"))
                .unwrap_or_default()
        } else {
            task_provider
        }
    };
    match provider.to_ascii_lowercase().as_str() {
        "codex" => "Codex".into(),
        "claude" => "Claude".into(),
        "opencode-go" | "opencode" => "OpenCode".into(),
        "minimax" => "MiniMax".into(),
        "openai" => "OpenAI".into(),
        "deepseek" => "DeepSeek".into(),
        "gemini" => "Gemini".into(),
        "glm" => "GLM".into(),
        "grok" => "Grok".into(),
        "kimi" => "Kimi".into(),
        "mistral" => "Mistral".into(),
        "qwen" => "Qwen".into(),
        "acp" => agent
            .as_ref()
            .map(|agent| field(agent, "name"))
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "ACP".into()),
        _ => provider,
    }
}

pub(crate) fn project_icon(id: &str) -> IconName {
    match id {
        "briefcase" => IconName::Briefcase,
        "code" => IconName::Code,
        "rocket" => IconName::Rocket,
        "globe" => IconName::Globe,
        "palette" => IconName::Palette,
        "database" => IconName::Database,
        "wrench" => IconName::Wrench,
        "layers" => IconName::Layers,
        _ => IconName::Folder,
    }
}

const OLDER_TASK_AGE_MS: i64 = 4 * 60 * 60 * 1000;

fn sidebar_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

fn task_activity_ms(task: &Value) -> i64 {
    task.get("updatedAt")
        .and_then(Value::as_i64)
        .filter(|time| *time > 0)
        .or_else(|| task.get("createdAt").and_then(Value::as_i64))
        .unwrap_or(0)
}

fn task_is_older(task: &Value, now_ms: i64) -> bool {
    now_ms.saturating_sub(task_activity_ms(task)) > OLDER_TASK_AGE_MS
}

fn relative_task_time(timestamp_ms: i64) -> String {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(timestamp_ms);
    let minutes = now_ms
        .saturating_sub(timestamp_ms)
        .max(0)
        .saturating_add(30_000)
        / 60_000;
    if minutes < 1 {
        "now".into()
    } else if minutes < 60 {
        format!("{minutes}m ago")
    } else {
        format!("{}h ago", minutes.saturating_add(30) / 60)
    }
}

// Surface enum has no `Display`, so derive a stable slug for button IDs.
fn surface_slug(surface: Sidebar) -> &'static str {
    match surface {
        Sidebar::Chats => "chats",
        Sidebar::Archived => "archives",
        Sidebar::Agents => "agents",
        Sidebar::Projects => "projects",
        Sidebar::Browser => "browser",
        Sidebar::Settings => "settings",
        Sidebar::Mail => "mail",
        Sidebar::Channels => "channels",
        Sidebar::Terminal => "terminal",
        Sidebar::Usage => "usage",
        Sidebar::Subagents => "companion",
        Sidebar::Schedules => "schedules",
        Sidebar::Git => "git",
    }
}

#[cfg(all(test, feature = "visual-export"))]
#[path = "sidebar_hover_tests.rs"]
mod sidebar_hover_tests;

#[cfg(all(test, feature = "visual-export"))]
mod sidebar_alignment_tests {
    use super::sidebar_task_row_frame;
    use crate::{Desktop, design};
    use gpui::InteractiveElement as _;
    use gpui_kit::{
        AppContext as _, Context, IntoElement, ParentElement as _, Render, Styled as _,
        TestAppContext, Window, div,
    };
    struct AlignmentFixture;
    impl Render for AlignmentFixture {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let palette = design::PRESETS[0].1;
            let settings = serde_json::json!({"interfaceFontSize": 14});
            div()
                .w(gpui_kit::px(240.))
                .flex()
                .flex_col()
                .child(Desktop::section_label("Recent", palette, &settings))
                .child(
                    sidebar_task_row_frame().child(
                        div()
                            .debug_selector(|| "sidebar-agent-avatar".into())
                            .w(gpui_kit::px(30.))
                            .h(gpui_kit::px(30.)),
                    ),
                )
        }
    }
    #[gpui_kit::test]
    fn sidebar_agent_row_avatar_is_flush_with_recent_heading(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let (_, mut visual) = cx.add_window_view(|_, _| AlignmentFixture);
        let heading = visual
            .debug_bounds("sidebar-section-copy")
            .expect("heading renders");
        let avatar = visual
            .debug_bounds("sidebar-agent-avatar")
            .expect("row avatar renders");
        assert!((f32::from(heading.origin.x) - f32::from(avatar.origin.x)).abs() < 0.01);
    }
}
