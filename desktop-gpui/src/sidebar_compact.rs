//! Compact, accessible icon rail for the collapsed workspace sidebar.
use super::*;
use gpui_kit::component::tooltip::ManagedTooltipExt as _;
use gpui_kit::{
    assets::__private as icons_assets,
    component::{Icon, Sizable, button::*, hover_card::HoverCard, tooltip::Tooltip},
};
use std::rc::Rc;

fn icon_button(
    id: impl Into<SharedString>,
    label: &'static str,
    icon: &'static [u8],
    active: bool,
    palette: design::Palette,
    click: impl Fn(&mut Desktop, &mut Window, &mut Context<Desktop>) + 'static,
    cx: &mut Context<Desktop>,
) -> AnyElement {
    Button::new(id.into())
        .when(
            design::tile_theme() == monitter_lib::experimental_desktop::NativeTileTheme::Tern,
            |button| button.ghost(),
        )
        .compact()
        .icon(
            Icon::default()
                .data(icon)
                .with_size(design::ui_scaled_px(16.)),
        )
        .tooltip(label)
        .accessibility_label(label)
        .when(active, |button| button.outline())
        .on_click(cx.listener(move |desktop, _, window, cx| click(desktop, window, cx)))
        .into_any_element()
}

pub(super) fn render(
    desktop: &Desktop,
    agents: &[Value],
    tasks: &[Value],
    palette: design::Palette,
    window: &mut Window,
    cx: &mut Context<Desktop>,
) -> AnyElement {
    let tern = design::tile_theme() == monitter_lib::experimental_desktop::NativeTileTheme::Tern;
    let rail_width = gpui_kit::px(design::COMPACT_SIDEBAR_WIDTH_POINTS);
    let width = if tern {
        rail_width - gpui_kit::px(12.)
    } else {
        rail_width
    };
    let mut views = div().flex().flex_col().items_center().gap(px(5.));
    for tab in VIEWS {
        let next = tab.view;
        let active = desktop.sidebar_view == tab.view;
        views = views.child(icon_button(
            SharedString::from(format!(
                "compact-sidebar-{}",
                tab.label.to_ascii_lowercase()
            )),
            tab.label,
            tab.icon,
            active,
            palette,
            move |view, _, cx| {
                view.sidebar_view = next;
                cx.notify();
            },
            cx,
        ));
    }

    let ordered = running_tasks(tasks);
    let chat_scroll = window
        .use_keyed_state("compact-chat-overflow", cx, |_, _| ScrollHandle::new())
        .read(cx)
        .clone();
    let mut chat_icons = div()
        .id("compact-chat-icons")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .track_scroll(&chat_scroll)
        .flex()
        .flex_col()
        .items_center()
        .gap(px(4.));
    if desktop.sidebar_view == SidebarView::Activity {
        for task in &ordered {
            let id = field(task, "id");
            let title = field(task, "title");
            let agent = agents
                .iter()
                .find(|agent| field(agent, "id") == field(task, "agentId"));
            let provider = field(task, "provider");
            let model = field(task, "model");
            let active = desktop.selected.as_ref() == Some(&id);
            let hover_key = format!("compact-chat-{id}");
            let (hover_state, background_opacity) =
                sidebar_hover::state(&hover_key, active, window, cx);
            let task_id = id.clone();
            let icon = crate::branding::provider_for_ink(&provider, &model, palette.ink)
                .map(|image| img(image).w(px(17.)).h(px(17.)).into_any_element())
                .unwrap_or_else(|| {
                    Icon::default()
                        .data(icons_assets::MessageSquare.1)
                        .with_size(px(16.))
                        .into_any_element()
                });
            let status = field(task, "status");
            let avatar_color = agent
                .map(|agent| design::parse_color(&field(agent, "color"), palette.accent))
                .unwrap_or(palette.muted);
            let tooltip_label = format!("{title} · {provider} · {status}");
            // Managed tooltip owns an on_hover slot. Keep it on a separate
            // full-size wrapper so the retained background animation has its own.
            chat_icons = chat_icons.child(
                div()
                    .id(SharedString::from(format!("{hover_key}-tooltip")))
                    .w(width - px(8.))
                    .h(px(34.))
                    .managed_tooltip(move |window, cx| {
                        Tooltip::new(tooltip_label.clone()).build(window, cx)
                    })
                    .child(
                        div()
                            .id(SharedString::from(hover_key))
                            .w(width - px(8.))
                            .h(px(34.))
                            .rounded_md()
                            .cursor_pointer()
                            .flex()
                            .items_center()
                            .justify_center()
                            .relative()
                            .bg(crate::design::surface_color(palette.soft)
                                .opacity(background_opacity))
                            .overflow_hidden()
                            .children(crate::activity_effects::for_task(
                                &task_id,
                                "minibar",
                                crate::activity_sparkles::SparkleLayout::Compact,
                                false,
                                cx,
                            ))
                            .child(icon)
                            .child(
                                div()
                                    .absolute()
                                    .right_1()
                                    .bottom_1()
                                    .w(px(6.))
                                    .h(px(6.))
                                    .rounded_full()
                                    .bg(rgb(if status == "running" {
                                        design::status_color(&status, palette)
                                    } else {
                                        avatar_color
                                    })),
                            )
                            .aria_label(format!("Open chat {}", field(task, "title")))
                            .on_click(cx.listener(move |view, _, window, cx| {
                                view.navigate_sidebar_task(task_id.clone(), window, cx);
                            }))
                            .on_hover(cx.listener(move |_, hovered, _, cx| {
                                sidebar_hover::set(&hover_state, *hovered, cx);
                            })),
                    ),
            );
        }
        if ordered.is_empty() {
            chat_icons = chat_icons.child(note_compact(
                "No activity",
                palette,
                sidebar_text_px(8., &desktop.snapshot["settings"]),
            ));
        }
    } else {
        let projects = rows(&desktop.snapshot, "projects");
        let groups = compact_groups(desktop.sidebar_view, agents, &projects, tasks);
        let owner = cx.weak_entity();
        let select: SelectChat = Rc::new(move |id, window, cx| {
            let _ = owner.update(cx, |view, cx| {
                view.navigate_sidebar_task(id.to_string(), window, cx);
            });
        });
        for group in groups {
            let selected = group
                .tasks
                .iter()
                .any(|task| desktop.selected.as_deref() == Some(field(task, "id").as_str()));
            chat_icons = chat_icons.child(group_menu(
                group,
                selected,
                palette,
                select.clone(),
                sidebar_text_px(13., &desktop.snapshot["settings"]),
            ));
        }
    }

    let footer_actions = [
        (
            Sidebar::Chats,
            "New chat",
            icons_assets::MessageSquarePlus.1,
        ),
        (Sidebar::Terminal, "New terminal", icons_assets::Terminal.1),
        (Sidebar::Subagents, "Companion mode", icons_assets::Bot.1),
        (Sidebar::Settings, "Settings", icons_assets::Settings.1),
    ];
    let mut footer = div().flex().flex_col().items_center().gap(px(4.));
    for (surface, label, icon) in footer_actions {
        footer = footer.child(icon_button(
            SharedString::from(format!(
                "compact-footer-{}",
                label.to_ascii_lowercase().replace(' ', "-")
            )),
            label,
            icon,
            desktop.sidebar == surface,
            palette,
            move |view, window, cx| {
                if surface == Sidebar::Chats {
                    view.open_chat_draft(None, window, cx);
                } else {
                    view.switch_surface(surface, window, cx);
                }
                cx.notify();
            },
            cx,
        ));
    }
    if tern {
        let interface_scale = design::interface_scale() as f32 / 100.;
        let usage_width = (design::COMPACT_SIDEBAR_WIDTH_POINTS - 12.) / interface_scale;
        let usage_stats = desktop.sidebar_usage.as_ref().map(|usage| {
            usage.update(cx, |usage, cx| {
                usage.set_composed_palette(palette, usage_width, cx)
            });
            usage.clone()
        });
        return div()
            .id("sidebar-compact-rail")
            .w(rail_width)
            .h_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .p(px(design::tern_spacing(&desktop.snapshot["settings"])
                .gutter
                / 2.))
            .gap(px(
                design::tern_spacing(&desktop.snapshot["settings"]).gutter
            ))
            .child(
                div()
                    .id("compact-navigation")
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(7.))
                    .pb(px(7.))
                    // Preserve native traffic-light access above the controls.
                    .pt(gpui_kit::px(38.))
                    .child(icon_button(
                        "compact-sidebar-hide",
                        "Hide sidebar",
                        icons_assets::ChevronLeft.1,
                        false,
                        palette,
                        |view, window, cx| view.toggle_sidebar(window, cx),
                        cx,
                    ))
                    .child(views)
                    .child(div().h(px(1.)).w(px(28.)).bg(rgb(palette.line)))
                    .child(ScrollEdgeFade::new(
                        chat_icons,
                        chat_scroll,
                        Axis::Vertical,
                        gpui_kit::px(20.),
                    ))
                    .child(div().h(px(1.)).w(px(28.)).bg(rgb(palette.line)))
                    .child(footer),
            )
            .child(
                div()
                    .id("compact-usage-footer")
                    .w_full()
                    .flex_shrink_0()
                    .flex()
                    .flex_col()
                    .children(usage_stats),
            )
            .into_any_element();
    }

    let _ = window;
    div()
        .id("sidebar-compact-rail")
        .w(width)
        .h_full()
        .flex_shrink_0()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(7.))
        .pb(px(7.))
        // Native traffic lights occupy the top 44 physical pixels.
        .pt(gpui_kit::px(44.))
        .border_color(rgb(palette.line))
        .bg(crate::surface_blend::background(
            crate::surface_blend::Area::Sidebar,
            palette.sidebar,
        ))
        .child(icon_button(
            "compact-sidebar-expand",
            "Hide sidebar",
            icons_assets::ChevronLeft.1,
            false,
            palette,
            |view, window, cx| view.toggle_sidebar(window, cx),
            cx,
        ))
        .child(views)
        .child(div().h(px(1.)).w(px(28.)).bg(rgb(palette.line)))
        .child(ScrollEdgeFade::new(
            chat_icons,
            chat_scroll,
            Axis::Vertical,
            gpui_kit::px(20.),
        ))
        .child(div().h(px(1.)).w(px(28.)).bg(rgb(palette.line)))
        .child(footer)
        .into_any_element()
}

fn note_compact(label: &'static str, palette: design::Palette, font_size: Pixels) -> AnyElement {
    div()
        .text_size(font_size)
        .text_color(rgb(palette.muted))
        .child(label)
        .into_any_element()
}

/// Menu contents use the same authoritative running status as expanded Activity.
fn running_tasks(tasks: &[Value]) -> Vec<Value> {
    let mut running: Vec<_> = tasks
        .iter()
        .filter(|task| field(task, "status") == "running")
        .cloned()
        .collect();
    running.sort_by_key(|task| std::cmp::Reverse(task["updatedAt"].as_i64().unwrap_or_default()));
    running
}

struct CompactGroup {
    key: String,
    label: String,
    record: Value,
    project: bool,
    tasks: Vec<Value>,
}

fn compact_groups(
    view: SidebarView,
    agents: &[Value],
    projects: &[Value],
    tasks: &[Value],
) -> Vec<CompactGroup> {
    let project = view == SidebarView::Projects;
    let records = if project { projects } else { agents };
    let owner_field = if project { "projectId" } else { "agentId" };
    let running = running_tasks(tasks);
    let mut groups: Vec<_> = records
        .iter()
        .map(|record| {
            let id = field(record, "id");
            CompactGroup {
                key: format!("compact-{}-{id}", if project { "project" } else { "agent" }),
                label: field(record, "name"),
                record: record.clone(),
                project,
                tasks: running
                    .iter()
                    .filter(|task| field(task, owner_field) == id)
                    .cloned()
                    .collect(),
            }
        })
        .collect();
    // Keep work without a current project/agent reachable through one group.
    let unassigned: Vec<_> = running
        .iter()
        .filter(|task| {
            !records
                .iter()
                .any(|record| field(record, "id") == field(task, owner_field))
        })
        .cloned()
        .collect();
    if !unassigned.is_empty() {
        groups.push(CompactGroup {
            key: format!(
                "compact-{}-unassigned",
                if project { "project" } else { "agent" }
            ),
            label: if project {
                "Unassigned"
            } else {
                "Other agents"
            }
            .into(),
            record: Value::Null,
            project,
            tasks: unassigned,
        });
    }
    groups
}

type SelectChat = Rc<dyn Fn(&str, &mut Window, &mut App)>;

fn group_menu(
    group: CompactGroup,
    selected: bool,
    palette: design::Palette,
    select: SelectChat,
    font_size: Pixels,
) -> AnyElement {
    let color = design::parse_color(&field(&group.record, "color"), palette.accent);
    let icon = if group.project {
        Icon::new(project_icon(&field(&group.record, "icon")))
            .size(px(17.))
            .into_any_element()
    } else {
        div()
            .text_size(font_size)
            .font_weight(FontWeight::SEMIBOLD)
            .child(group.label.chars().next().unwrap_or('?').to_string())
            .into_any_element()
    };
    let trigger = Button::new(SharedString::from(group.key.clone()))
        .text()
        .compact()
        .w(px(34.))
        .h(px(34.))
        .p_0()
        .accessibility_label(format!(
            "{} · {} running chats",
            group.label,
            group.tasks.len()
        ))
        .when(selected, |button| button.outline())
        .child(
            div()
                .debug_selector({
                    let key = group.key.clone();
                    move || key.clone()
                })
                .w(px(30.))
                .h(px(30.))
                .rounded_md()
                .relative()
                .bg(crate::design::surface_color(color).opacity(0.16))
                .text_color(rgb(color))
                .flex()
                .items_center()
                .justify_center()
                .child(icon)
                .when(!group.tasks.is_empty(), |tile| {
                    tile.child(
                        div()
                            .absolute()
                            .right_0()
                            .bottom_0()
                            .w(px(6.))
                            .h(px(6.))
                            .rounded_full()
                            .bg(rgb(palette.accent)),
                    )
                }),
        );
    let popup_id = format!("{}-menu", group.key);
    HoverCard::new(SharedString::from(format!("{}-hover", group.key)))
        .header(group.label.clone(), "Running chats in this group.")
        .anchor(Anchor::LeftCenter)
        .open_delay(std::time::Duration::from_millis(160))
        .close_delay(std::time::Duration::from_millis(250))
        .trigger(trigger)
        .content(move |_, window, cx| {
            let scroll = window
                .use_keyed_state(
                    SharedString::from(format!("{popup_id}-scroll")),
                    cx,
                    |_, _| ScrollHandle::new(),
                )
                .read(cx)
                .clone();
            let mut menu = div()
                .id(SharedString::from(popup_id.clone()))
                .debug_selector({
                    let key = popup_id.clone();
                    move || key.clone()
                })
                .w(px(280.))
                .flex()
                .flex_col()
                .gap(px(design::space::TIGHT))
                .text_color(rgb(palette.ink))
                .font_family(cx.theme().font_family.clone())
                .text_size(cx.theme().font_size * 0.9);
            let mut list = div()
                .id(SharedString::from(format!("{popup_id}-list")))
                .max_h(px(360.))
                .overflow_y_scroll()
                .track_scroll(&scroll)
                .flex()
                .flex_col()
                .gap(px(2.));
            for task in &group.tasks {
                let id = field(task, "id");
                let select = select.clone();
                let row_id = format!("{popup_id}-task-{id}");
                list = list.child(
                    Button::new(SharedString::from(row_id.clone()))
                        .text()
                        .compact()
                        .w_full()
                        .justify_start()
                        .icon(
                            Icon::default()
                                .data(icons_assets::Activity.1)
                                .size(px(14.))
                                .text_color(rgb(palette.accent)),
                        )
                        .accessibility_label(format!("Open running chat {}", field(task, "title")))
                        .child(
                            div()
                                .debug_selector(move || row_id.clone())
                                .min_w_0()
                                .truncate()
                                .child(field(task, "title")),
                        )
                        .on_click(move |_, window, cx| select(&id, window, cx)),
                );
            }
            if group.tasks.is_empty() {
                list = list.child(
                    div()
                        .p(px(6.))
                        .text_color(rgb(palette.muted))
                        .child("No running chats"),
                );
            }
            menu = menu.child(ScrollEdgeFade::new(
                list,
                scroll,
                Axis::Vertical,
                gpui_kit::px(16.),
            ));
            menu
        })
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{SidebarView, compact_groups, field, running_tasks};
    use serde_json::json;

    #[test]
    fn compact_sidebar_groups_only_running_work_under_top_level_icons() {
        let agents = vec![
            json!({"id":"a","name":"Codex"}),
            json!({"id":"b","name":"Other"}),
        ];
        let projects = vec![json!({"id":"p","name":"Monitter"})];
        let tasks = vec![
            json!({"id":"idle","agentId":"a","projectId":"p","status":"idle","updatedAt":100}),
            json!({"id":"run1","agentId":"a","projectId":"p","status":"running","updatedAt":1}),
            json!({"id":"run2","agentId":"a","projectId":"p","status":"running","updatedAt":2}),
            json!({"id":"done","agentId":"b","status":"completed","updatedAt":200}),
            json!({"id":"error","agentId":"b","status":"error","updatedAt":300}),
        ];
        let agent_groups = compact_groups(SidebarView::Agents, &agents, &projects, &tasks);
        assert_eq!(
            agent_groups.len(),
            2,
            "one icon per agent, regardless of chat count"
        );
        assert_eq!(agent_groups[0].tasks.len(), 2);
        assert!(agent_groups[1].tasks.is_empty());
        let project_groups = compact_groups(SidebarView::Projects, &agents, &projects, &tasks);
        assert_eq!(project_groups.len(), 1, "one icon per project");
        assert_eq!(project_groups[0].tasks.len(), 2);
        let activity = running_tasks(&tasks);
        assert_eq!(
            activity
                .iter()
                .map(|task| field(task, "id"))
                .collect::<Vec<_>>(),
            ["run2", "run1"]
        );
        let mut stopped = tasks.clone();
        stopped[1]["status"] = json!("completed");
        stopped[2]["status"] = json!("idle");
        assert!(
            running_tasks(&stopped).is_empty(),
            "finished work disappears on the next snapshot"
        );
    }

    #[test]
    fn compact_sidebar_keeps_unassigned_running_work_reachable() {
        let tasks = vec![json!({"id":"run","agentId":"removed","status":"running"})];
        let groups = compact_groups(SidebarView::Projects, &[], &[], &tasks);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].label, "Unassigned");
        assert_eq!(groups[0].tasks.len(), 1);
        let groups = compact_groups(SidebarView::Agents, &[], &[], &tasks);
        assert_eq!(groups[0].label, "Other agents");
    }
}

#[cfg(all(test, feature = "visual-export"))]
mod visual_tests {
    use super::{SelectChat, SidebarView, compact_groups, design, group_menu, px};
    use gpui_kit::{
        AppContext as _, Context, InteractiveElement as _, IntoElement, ParentElement as _, Render,
        Styled as _, TestAppContext, VisualTestContext, Window, div, point,
    };
    use serde_json::json;
    use std::rc::Rc;
    use std::{cell::RefCell, time::Duration};

    struct MenuFixture {
        project: bool,
        selected: Rc<RefCell<String>>,
    }

    impl Render for MenuFixture {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let agents = vec![json!({"id":"a","name":"Codex"})];
            let projects = vec![json!({"id":"p","name":"Monitter"})];
            let tasks = vec![
                json!({"id":"run","title":"Running work","agentId":"a","projectId":"p","status":"running"}),
                json!({"id":"idle","title":"Idle work","agentId":"a","projectId":"p","status":"idle"}),
            ];
            let view = if self.project {
                SidebarView::Projects
            } else {
                SidebarView::Agents
            };
            let group = compact_groups(view, &agents, &projects, &tasks).remove(0);
            let selected = self.selected.clone();
            let select: SelectChat =
                Rc::new(move |id, _, _| *selected.borrow_mut() = id.to_string());
            div()
                .w(gpui_kit::px(800.))
                .h(gpui_kit::px(600.))
                .flex()
                .flex_col()
                .items_start()
                .child(group_menu(
                    group,
                    false,
                    design::palette(&json!({}), gpui_kit::WindowAppearance::Dark),
                    select,
                    super::sidebar_text_px(13., &json!({})),
                ))
                .child(
                    div()
                        .debug_selector(|| "outside-menu".into())
                        .w(px(50.))
                        .h(px(300.)),
                )
        }
    }

    fn tick(visual: &mut VisualTestContext, milliseconds: u64) {
        visual.run_until_parked();
        visual
            .executor()
            .advance_clock(Duration::from_millis(milliseconds));
        visual.run_until_parked();
        visual.update(|window, cx| {
            window.simulate_next_frame(cx);
            window.draw(cx).clear(cx);
        });
    }

    #[gpui_kit::test]
    fn compact_sidebar_hover_menu_keeps_pointer_transfer_and_opens_running_chat(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        for project in [false, true] {
            let selected = Rc::new(RefCell::new(String::new()));
            let (_, visual) = cx.add_window_view({
                let selected = selected.clone();
                move |_, _| MenuFixture { project, selected }
            });
            visual.update(|window, _| window.activate_window());
            visual.run_until_parked();
            visual.update(|window, cx| window.draw(cx).clear(cx));
            let (trigger, menu, row, idle) = if project {
                (
                    "compact-project-p",
                    "compact-project-p-menu",
                    "compact-project-p-menu-task-run",
                    "compact-project-p-menu-task-idle",
                )
            } else {
                (
                    "compact-agent-a",
                    "compact-agent-a-menu",
                    "compact-agent-a-menu-task-run",
                    "compact-agent-a-menu-task-idle",
                )
            };
            assert!(visual.debug_bounds(menu).is_none());
            let trigger_bounds = visual.debug_bounds(trigger).expect("top-level icon");
            visual.simulate_mouse_move(trigger_bounds.center(), None, Default::default());
            tick(visual, 180);
            assert!(
                visual.debug_bounds(menu).is_some(),
                "hover opens running menu"
            );
            assert!(visual.debug_bounds(idle).is_none(), "idle work excluded");
            let row_bounds = visual.debug_bounds(row).expect("running work row");
            assert!(
                row_bounds.left() >= trigger_bounds.right(),
                "flyout opens beside the rail"
            );
            visual.simulate_mouse_move(row_bounds.center(), None, Default::default());
            tick(visual, 300);
            assert!(
                visual.debug_bounds(menu).is_some(),
                "menu survives pointer transfer"
            );
            visual.simulate_click(row_bounds.center(), Default::default());
            assert_eq!(selected.borrow().as_str(), "run");
            visual.simulate_mouse_move(point(px(600.), px(500.)), None, Default::default());
            tick(visual, 300);
            assert!(visual.debug_bounds(menu).is_none(), "leaving closes menu");
        }
    }
}
