//! Native sidebar with agent, project and activity views plus route controls.
use super::*;
use crate::design::ui_scaled_px as px;
use gpui_kit::{
    assets::{__private as icons_assets, IconName},
    component::menu::{DropdownMenu, PopupMenuItem},
    component::{button::*, ActiveTheme as _, Icon, Sizable},
};

#[path = "sidebar_compact.rs"]
mod sidebar_compact;
#[path = "sidebar_hover.rs"]
mod sidebar_hover;

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

impl Desktop {
    /// Narrow icon rail used when the workspace sidebar is collapsed.
    pub(super) fn render_compact_sidebar(
        &self,
        agents: &[Value],
        tasks: &[Value],
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
        let active = self.selected.as_ref() == Some(&task_id);
        let status = field(task, "status");
        let provider = provider_label(&self.snapshot, task);
        let model = field(task, "model");
        let project_name = context.filter(|name| !name.is_empty());
        let updated_at = task.get("updatedAt").and_then(Value::as_i64).filter(|time| *time > 0);
        let relative_time = updated_at.map(relative_task_time);
        let title = field(task, "title");
        let hover_key = format!("{id_prefix}-{task_id}");
        let (hover_state, background_opacity) =
            sidebar_hover::state(&hover_key, active, window, cx);
        let provider_icon = crate::branding::provider_for_ink(&field(task, "provider"), &model, palette.ink)
            .map(|image| img(image).w(px(17.)).h(px(17.)).into_any_element())
            .unwrap_or_else(|| Icon::new(IconName::MessageSquare).size(px(16.)).into_any_element());
        let meta = if let Some(project_name) = project_name.clone() {
            if provider.is_empty() {
                div().flex_1().min_w_0().truncate().child(project_name)
            } else {
                div().flex_1().min_w_0().flex().items_center().gap(px(3.))
                    .child(div().min_w_0().truncate().child(provider.clone()))
                    .child("·")
                    .child(div().min_w_0().truncate().child(project_name))
            }
        } else {
            div().flex_1().min_w_0().truncate().child(provider.clone())
        };
        let mut accessible_parts = vec![title.clone()];
        if !provider.is_empty() { accessible_parts.push(provider.clone()); }
        if let Some(project) = project_name.as_ref() { accessible_parts.push(project.clone()); }
        if !status.is_empty() { accessible_parts.push(status.clone()); }
        if let Some(time) = relative_time.as_ref() { accessible_parts.push(time.clone()); }
        let accessible_label = accessible_parts.join(" · ");
        div()
            .id(SharedString::from(hover_key))
            .role(Role::Button)
            .aria_label(accessible_label)
            .flex_shrink_0()
            .ml(px(design::space::GROUP))
            .px(px(design::space::INSET))
            .py(px(8.))
            .rounded_md()
            .cursor_pointer()
            .relative().overflow_hidden()
            .bg(crate::design::surface_color(palette.soft).opacity(background_opacity))
            .flex()
            .items_center()
            .gap(px(design::space::TIGHT))
            .children(crate::activity_effects::for_task(&task_id, id_prefix, crate::activity_sparkles::SparkleLayout::Compact, false, cx))
            .child(
                div().w(px(30.)).h(px(30.)).flex_shrink_0().rounded_md().relative().flex().items_center().justify_center()
                    .bg(crate::design::surface_color(palette.panel))
                    .child(provider_icon)
                    .child(div().absolute().right(px(1.)).bottom(px(1.))
                        .w(px(design::STATUS_DOT)).h(px(design::STATUS_DOT)).rounded_full()
                        .bg(rgb(if status == "running" { palette.accent } else { design::status_color(&status, palette) }))),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex().flex_col().gap(px(2.))
                    .child(div().min_w_0().truncate().text_size(px(13.))
                        .font_weight(if older { FontWeight::THIN } else { FontWeight::SEMIBOLD })
                        .text_color(rgb(palette.ink).opacity(if older { 0.7 } else { 1. })).child(title))
                    .child(div().min_w_0().flex().items_center().gap(px(3.))
                        .text_size(px(10.)).font_weight(if older { FontWeight::THIN } else { FontWeight::NORMAL })
                        .text_color(rgb(palette.muted).opacity(if older { 0.7 } else { 1. }))
                        .child(meta)
                        .when_some(relative_time, |row, time| row.child(div().flex_shrink_0().whitespace_nowrap().child(time))),
                    ),
            )
            .when(status == "running", |row| row.child(
                Icon::new(IconName::LoaderCircle).size(px(14.)).text_color(rgb(palette.accent)),
            ))
            .on_click(cx.listener(move |view, _, window, cx| {
                view.switch_surface(Sidebar::Chats, window, cx);
                view.selected = Some(task_id.clone());
                cx.notify();
            }))
            .on_hover(cx.listener(move |_, hovered, _, cx| {
                sidebar_hover::set(&hover_state, *hovered, cx);
            }))
            .into_any_element()
    }

    fn grouped_task_rows(
        &self,
        tasks: Vec<&Value>,
        show_project: bool,
        palette: design::Palette,
        id_prefix: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let now = sidebar_now_ms();
        let mut ordered = tasks;
        ordered.sort_by_key(|task| std::cmp::Reverse(task_activity_ms(task)));
        let mut group = div().flex_shrink_0().flex().flex_col().gap(px(design::space::TIGHT));
        for older in [false, true] {
            let rows: Vec<_> = ordered.iter().copied().filter(|task| task_is_older(task, now) == older).collect();
            if rows.is_empty() { continue; }
            group = group.child(Self::section_label(if older { "Older" } else { "Recent" }, palette));
            for task in rows {
                let context = if show_project { project_name(&self.snapshot, task) } else { None };
                group = group.child(self.task_row(task, context, palette, id_prefix, window, cx));
            }
        }
        group.into_any_element()
    }

    fn project_row(
        project: &Value,
        chat_count: usize,
        running: bool,
        collapsed: bool,
        collapse: Entity<ProjectCollapseState>,
        palette: design::Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let name = field(project, "name");
        let icon = project_icon(&field(project, "icon"));
        let color = design::parse_color(&field(project, "color"), palette.accent);
        let count = format!("{chat_count} {}", if chat_count == 1 { "chat" } else { "chats" });
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
                    .text_size(px(14.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(palette.ink))
                    .child(name.clone()),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(palette.muted))
                    .child(count.clone()),
            );
        let project_id = field(project, "id");
        div().id(SharedString::from(format!("project-group-{project_id}")))
            .flex_shrink_0()
            .w_full().px(px(design::space::TIGHT)).py(px(design::space::TIGHT))
            .flex().items_center().gap(px(design::space::TIGHT))
            .child(tile)
            .child(copy)
            .child(Icon::new(if collapsed { IconName::ChevronRight } else { IconName::ChevronDown }).size(px(13.)).text_color(rgb(palette.muted)))
            .role(Role::Button)
            .aria_label(format!("{} project, {}. {}", if collapsed { "Expand" } else { "Collapse" }, name, count))
            .cursor_pointer()
            .on_click(cx.listener(move |_, _, _, cx| {
                collapse.update(cx, |state, _| state.collapsed = !state.collapsed);
                cx.notify();
            }))
    }

    fn section_label(label: &str, palette: design::Palette) -> Div {
        div()
            .flex_shrink_0()
            .px(px(design::space::INSET))
            .pt(px(design::space::SECTION))
            .pb(px(design::space::TIGHT))
            .text_size(px(10.))
            .text_color(rgb(palette.muted))
            .font_family(design::FONT_MONO)
            .child(label.to_uppercase())
    }

    pub(super) fn render_sidebar(
        &self,
        agents: &[Value],
        tasks: &[Value],
        palette: design::Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tern = design::tile_theme()
            == monitter_lib::experimental_desktop::NativeTileTheme::Tern;
        let tree_scroll = window.use_keyed_state("sidebar-tree-overflow", cx,
            |_, _| ScrollHandle::new()).read(cx).clone();
        let mut tree = div()
            .id("sidebar-tree")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&tree_scroll)
            .flex()
            .flex_col()
            .gap(px(design::space::TIGHT))
            .px(px(design::space::INSET))
            .py(px(design::space::INSET))
            .when(tern, |tree| tree.pt_0());
        if self.sidebar_view == SidebarView::Agents {
            for agent in agents {
                let id = field(agent, "id");
                let collapsed = self.collapsed_agents.contains(&id);
                let color = design::parse_color(&field(agent, "color"), palette.accent);
                let owned: Vec<_> = tasks
                    .iter()
                    .filter(|task| field(task, "agentId") == id)
                    .collect();
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
                tree = tree.child(
                    div()
                        .id(SharedString::from(agent_row_id))
                        .flex_shrink_0()
                        .rounded_md()
                        .relative()
                        .overflow_hidden()
                        .bg(crate::design::surface_color(palette.soft).opacity(agent_background_opacity))
                        .flex()
                        .items_center()
                        .gap(px(design::space::TIGHT))
                        .py(px(design::space::INSET))
                        .px(px(design::space::TIGHT))
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
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    if !view.collapsed_agents.remove(&toggle) {
                                        view.collapsed_agents.insert(toggle.clone());
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
                                .child(div().text_size(px(13.)).child(field(agent, "name")))
                                .child(
                                    div()
                                        .text_size(px(10.))
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
                                .disabled(self.busy)
                                .on_click(cx.listener(move |view, _, window, cx| {
                                    view.open_chat_draft(Some(new_id.clone()), window, cx);
                                })),
                        )
                        .on_hover(cx.listener(move |_, hovered, _, cx| {
                            sidebar_hover::set(&agent_hover_state, *hovered, cx);
                        })),
                );
                if !collapsed {
                    tree = tree.child(self.grouped_task_rows(owned, true, palette, "agent-task", window, cx));
                }
            }
            // A removed agent may own an unarchived chat in older stores; keep it reachable.
            let orphaned = tasks.iter().filter(|task| {
                !agents.iter().any(|agent| field(agent, "id") == field(task, "agentId"))
            }).collect();
            tree = tree.child(self.grouped_task_rows(orphaned, true, palette, "orphan", window, cx));
            if tasks.is_empty() {
                tree = tree.child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(palette.muted))
                        .p(px(design::space::INSET))
                        .child("No chats yet. Use + beside an agent to start one."),
                );
            }
        } else if self.sidebar_view == SidebarView::Projects {
            let projects = rows(&self.snapshot, "projects");
            let mut seen_project_ids = std::collections::HashSet::new();
            for project in &projects {
                let project_id = field(project, "id");
                seen_project_ids.insert(project_id.clone());
                let project_tasks: Vec<_> = tasks
                    .iter()
                    .filter(|task| field(task, "projectId") == project_id)
                    .collect();
                let running = project_tasks.iter().any(|task| field(task, "status") == "running");
                let collapse = window.use_keyed_state(
                    SharedString::from(format!("project-collapse-{project_id}")),
                    cx,
                    |_, _| ProjectCollapseState { collapsed: false },
                );
                let collapsed = collapse.read(cx).collapsed;
                tree = tree.child(Self::project_row(project, project_tasks.len(), running, collapsed, collapse, palette, cx));
                if !collapsed {
                    tree = tree.child(self.grouped_task_rows(project_tasks, false, palette, "project-task", window, cx));
                }
            }
            let missing_project_ids: std::collections::BTreeSet<_> = tasks
                .iter()
                .map(|task| field(task, "projectId"))
                .filter(|id| !id.is_empty() && !seen_project_ids.contains(id))
                .collect();
            for project_id in missing_project_ids {
                tree = tree.child(Self::section_label(
                    &format!("Other project · {project_id}"),
                    palette,
                ));
                let project_tasks = tasks.iter().filter(|task| field(task, "projectId") == project_id).collect();
                tree = tree.child(self.grouped_task_rows(project_tasks, false, palette, "unlisted-project-task", window, cx));
            }
            let unassigned: Vec<_> = tasks
                .iter()
                .filter(|task| field(task, "projectId").is_empty())
                .collect();
            if !unassigned.is_empty() {
                tree = tree.child(Self::section_label("No project", palette));
                tree = tree.child(self.grouped_task_rows(unassigned, false, palette, "unassigned-task", window, cx));
            }
            if tasks.is_empty() {
                tree = tree.child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(palette.muted))
                        .p(px(design::space::INSET))
                        .child("No chats yet."),
                );
            }
        } else {
            let mut running: Vec<_> = tasks
                .iter()
                .filter(|task| field(task, "status") == "running")
                .collect();
            let mut recent: Vec<_> = tasks
                .iter()
                .filter(|task| field(task, "status") != "running")
                .collect();
            let by_updated = |left: &&Value, right: &&Value| {
                let left = left.get("updatedAt").and_then(Value::as_i64).unwrap_or(0);
                let right = right.get("updatedAt").and_then(Value::as_i64).unwrap_or(0);
                right.cmp(&left)
            };
            running.sort_by(by_updated);
            recent.sort_by(by_updated);
            if !running.is_empty() {
                tree = tree.child(Self::section_label("Running", palette));
                for task in running {
                    tree = tree.child(self.task_row(task, project_name(&self.snapshot, task), palette, "activity-running", window, cx));
                }
            }
            if !recent.is_empty() {
                tree = tree.child(self.grouped_task_rows(recent, true, palette, "activity-recent", window, cx));
            }
            if tasks.is_empty() {
                tree = tree.child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(palette.muted))
                        .p(px(design::space::INSET))
                        .child("No recent activity."),
                );
            }
        }

        // Three primary sidebar tabs (Agents / Projects / Activity) — match the
        // These views change the sidebar listing while leaving the selected
        // main content surface intact.
        let active_view = self.sidebar_view;
        self.sidebar_tabs_motion.borrow_mut().observe(match active_view {
            SidebarView::Agents => 0, SidebarView::Projects => 1, SidebarView::Activity => 2,
        });
        let tree = self.sidebar_tabs_motion.borrow().body("primary-sidebar", tree.into_any_element(), cx.reduce_motion(), true);
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
                            .text_size(px(design::SIDEBAR_TAB_FONT_SIZE))
                            .font_family(design::FONT_MONO)
                            .child(label),
                    )
                    .on_click(cx.listener(move |desktop, _, _, cx| {
                        desktop.sidebar_view = next_view;
                        cx.notify();
                    })),
            );
        }

        // Tern keeps its view navigation in the native titlebar control row.
        // The controls sit at the same 16px size as the compact sidebar reveal.
        let mut tern_view_toggles = div()
            .id("tern-sidebar-view-toggles")
            .flex()
            .items_center()
            .gap(px(2.));
        for tab in VIEWS {
            let next_view = tab.view;
            let active = active_view == tab.view;
            tern_view_toggles = tern_view_toggles.child(
                div().rounded(px(4.)).overflow_hidden().border_1()
                    .border_color(rgb(palette.accent).opacity(if active { 0.65 } else { 0. }))
                    .bg(rgb(palette.accent).opacity(if active { 0.14 } else { 0. }))
                    .child(Button::new(SharedString::from(format!(
                    "tern-sidebar-view-{}",
                    tab.label.to_ascii_lowercase()
                )))
                    .ghost()
                    .compact()
                    .icon(Icon::default().data(tab.icon).with_size(design::ui_scaled_px(16.))
                        .text_color(rgb(if active { palette.accent } else { palette.ink })))
                    .tooltip(tab.label)
                    .accessibility_label(tab.label)
                    .toggled(active)
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |desktop, _, _, cx| {
                        desktop.sidebar_view = next_view;
                        cx.notify();
                    }))),
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
        let sidebar_tabs_height = if modern_tabs {
            modern_height
        } else {
            35.
        };
        let interface_scale = design::interface_scale() as f32 / 100.;
        let dock_strip_height = design::workspace_tabbar_height_points(&self.snapshot["settings"], design::interface_scale()) / interface_scale;
        let brand_row_height = (dock_strip_height + chat_header_height - sidebar_tabs_height).max(0.);
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
            .child(img(branding::sidebar_wordmark(91.2 * interface_scale * window.scale_factor())).w(px(91.2)).h(px(30.4)).max_w_full().max_h_full().object_fit(ObjectFit::Contain));

        let usage_width = self.sidebar_resize.read(cx).sizes().first()
            .map(|width| *width / design::ui_px(1., self.sidebar_scale))
            .filter(|width| width.is_finite() && *width > 0.)
            .unwrap_or_else(|| self.current_sidebar_width(cx));
        let usage_width = if tern {
            (usage_width - 12. / interface_scale).max(0.)
        } else {
            usage_width
        };
        let usage_stats = self.sidebar_usage.as_ref().map(|usage| {
            usage.update(cx, |usage, cx| usage.set_composed_palette(palette, usage_width, cx));
            usage.clone()
        });
        let pending = rows(&self.snapshot, "approvalRequests").into_iter()
            .filter(|r| field(r,"status") == "pending" && r["input"].is_null()).collect::<Vec<_>>();
        let approval_block = if pending.is_empty() { None } else {
            let mut block = div().id("sidebar-pending-approvals").flex_shrink_0().max_h(px(160.)).overflow_y_scroll()
                .px(px(design::space::INSET)).py(px(design::space::TIGHT)).flex().flex_col()
                .font_family(cx.theme().font_family.clone()).text_size(cx.theme().font_size);
            for request in pending {
                let approval_id = field(&request,"id");
                let task_id = field(&request,"taskId");
                let title = tasks.iter().find(|t| field(t,"id") == task_id).map(|t| field(t,"title")).unwrap_or_else(|| "Agent request".into());
                block = block.child(div().id(SharedString::from(format!("sidebar-approval-{}",approval_id)))
                    .role(Role::Button).aria_label(format!("Review approval for {title}"))
                    .h((cx.theme().font_size * 1.4 + px(6.)).max(px(28.))).flex_shrink_0().px(px(design::space::TIGHT)).rounded_md().cursor_pointer()
                    .hover(|s| s.bg(design::surface_color(palette.soft)))
                    .flex().items_center().gap(px(design::space::TIGHT)).min_w_0().text_color(rgb(palette.accent))
                    .child(Icon::new(IconName::TriangleAlert).size(px(14.)).flex_shrink_0())
                    .child(div().min_w_0().truncate().child(format!("Review approval · {title}")))
                    .on_click(cx.listener(move |view,_,window,cx| {
                        view.switch_surface(Sidebar::Chats,window,cx);
                        view.selected = Some(task_id.clone());
                        let pane = view.chat_panes.borrow().get(&task_id).cloned();
                        if let Some(pane) = pane { pane.update(cx, |pane,cx| pane.show_approval_prompt(approval_id.clone(),window,cx)); }
                        cx.notify();
                    })));
            }
            Some(block)
        };
        let sidebar = div()
            .id("sidebar-root")
            .on_scroll_wheel(cx.listener(|desktop, event: &ScrollWheelEvent, _, cx| {
                let delta = event.delta.pixel_delta(gpui_kit::px(20.));
                if let Some(index) = desktop.sidebar_tabs_motion.borrow_mut().wheel(
                    delta.x.into(), delta.y.into(), 3, std::time::Instant::now(),
                ) {
                    desktop.sidebar_view = [SidebarView::Agents, SidebarView::Projects, SidebarView::Activity][index];
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
            .when(!tern, |sidebar| sidebar.bg(crate::surface_blend::background(crate::surface_blend::Area::Sidebar, palette.sidebar)));
        let footer = div()
            .id("sidebar-footer")
            .flex_shrink_0()
            .w_full()
            .flex()
            .flex_col()
            .children(usage_stats);
        let tree = ScrollEdgeFade::new(tree, tree_scroll, Axis::Vertical, gpui_kit::px(20.));
        let sidebar = if tern {
            let outer_pad = design::tern_spacing(&self.snapshot["settings"]).gutter / 2.;
            let header = div()
                .id("tern-sidebar-titlebar-row")
                .absolute()
                .top(px(-outer_pad))
                .left(px(-outer_pad))
                .right(px(-outer_pad))
                .h(gpui_kit::px(58.))
                .pl(gpui_kit::px(82.))
                .flex()
                .items_center()
                .child(tern_view_toggles)
                .on_mouse_down(MouseButton::Left, |event, window, _| {
                    if event.click_count == 2 {
                        window.titlebar_double_click();
                    } else {
                        window.start_window_move();
                    }
                });
            let sidebar_content_top = gpui_kit::px(29.)
                + px(16. + design::tern_spacing(&self.snapshot["settings"]).gutter - outer_pad);
            sidebar
                .p(px(design::tern_spacing(&self.snapshot["settings"]).gutter / 2.))
                .gap(px(design::tern_spacing(&self.snapshot["settings"]).gutter))
                .child(div()
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
                    .child(tree))
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

fn project_name(snapshot: &Value, task: &Value) -> Option<String> {
    let project_id = field(task, "projectId");
    if project_id.is_empty() {
        return None;
    }
    rows(snapshot, "projects")
        .into_iter()
        .find(|project| field(project, "id") == project_id)
        .map(|project| field(&project, "name"))
        .filter(|name| !name.is_empty())
}

fn provider_label(snapshot: &Value, task: &Value) -> String {
    let agent_id = field(task, "agentId");
    let agent = rows(snapshot, "agents")
        .into_iter()
        .find(|agent| field(agent, "id") == agent_id);
    let provider = {
        let task_provider = field(task, "provider");
        if task_provider.is_empty() {
            agent.as_ref().map(|agent| field(agent, "provider")).unwrap_or_default()
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

fn project_icon(id: &str) -> IconName {
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
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64).unwrap_or(0)
}

fn task_activity_ms(task: &Value) -> i64 {
    task.get("updatedAt").and_then(Value::as_i64).filter(|time| *time > 0)
        .or_else(|| task.get("createdAt").and_then(Value::as_i64)).unwrap_or(0)
}

fn task_is_older(task: &Value, now_ms: i64) -> bool {
    now_ms.saturating_sub(task_activity_ms(task)) > OLDER_TASK_AGE_MS
}

fn relative_task_time(timestamp_ms: i64) -> String {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(timestamp_ms);
    let minutes = now_ms.saturating_sub(timestamp_ms).max(0).saturating_add(30_000) / 60_000;
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
