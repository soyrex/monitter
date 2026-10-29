//! Flattened, retained sidebar metadata. Only visible rows become GPUI elements.
use super::*;
use gpui_kit::{ContentMask, Element, GlobalElementId, InspectorElementId, LayoutId};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::rc::Rc;
use std::time::{Duration, Instant};

/// The virtual list owns its scroll geometry; a Div ScrollHandle would not
/// observe it. Read the list's measured offset after prepaint for edge cues.
struct VirtualListEdgeFade {
    child: AnyElement,
    scroll: ListState,
}
impl IntoElement for VirtualListEdgeFade {
    type Element = Self;
    fn into_element(self) -> Self { self }
}
impl Element for VirtualListEdgeFade {
    type RequestLayoutState = ();
    type PrepaintState = ();
    fn id(&self) -> Option<ElementId> { None }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> { None }
    fn request_layout(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, window: &mut Window, cx: &mut App) -> (LayoutId, ()) {
        (self.child.request_layout(window, cx), ())
    }
    fn prepaint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, bounds: Bounds<Pixels>, _: &mut (), window: &mut Window, cx: &mut App) {
        #[cfg(target_os = "macos")]
        {
            let distance = gpui_kit::px(20.);
            let mask = ContentMask::vertical_fade_edges(bounds, distance, distance);
            window.with_content_mask(Some(mask), |window| self.child.prepaint(window, cx));
        }
        #[cfg(not(target_os = "macos"))]
        self.child.prepaint(window, cx);
    }
    fn paint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, bounds: Bounds<Pixels>, _: &mut (), _: &mut (), window: &mut Window, cx: &mut App) {
        #[cfg(target_os = "macos")]
        {
            let maximum = self.scroll.max_offset_for_scrollbar().y.max(Pixels::ZERO);
            let traveled = (-self.scroll.scroll_px_offset_for_scrollbar().y).max(Pixels::ZERO).min(maximum);
            let start = if traveled > gpui_kit::px(0.5) { gpui_kit::px(20.) } else { Pixels::ZERO };
            let end = if maximum - traveled > gpui_kit::px(0.5) { gpui_kit::px(20.) } else { Pixels::ZERO };
            let mask = ContentMask::vertical_fade_edges(bounds, start, end);
            window.with_content_mask(Some(mask), |window| self.child.paint(window, cx));
        }
        #[cfg(not(target_os = "macos"))]
        self.child.paint(window, cx);
    }
}

/// Sidebar/menu records deliberately exclude message bodies and tool events.
pub(super) fn metadata(record: &Value) -> Value {
    const FIELDS: &[&str] = &[
        "id",
        "name",
        "title",
        "provider",
        "model",
        "status",
        "agentId",
        "projectId",
        "hostId",
        "parentTaskId",
        "createdAt",
        "updatedAt",
        "lastActivityAt",
        "completedAt",
        "archived",
        "internal",
        "color",
        "icon",
    ];
    Value::Object(
        FIELDS
            .iter()
            .filter_map(|key| {
                record
                    .get(*key)
                    .map(|value| ((*key).to_owned(), value.clone()))
            })
            .collect(),
    )
}

enum Row {
    Task {
        record: Value,
        context: Option<String>,
        prefix: &'static str,
    },
    Agent {
        record: Value,
        collapsed: bool,
    },
    Project {
        record: Value,
        count: usize,
        running: bool,
        collapsed: bool,
        state: Entity<ProjectCollapseState>,
    },
    Section(String),
    Empty(&'static str),
}

struct FlatRow {
    key: String,
    row: Row,
}

struct SidebarList {
    revision: u64,
    minute: i64,
    collapsed_agents: HashSet<String>,
    collapsed_projects: Vec<(String, bool)>,
    layout: (f32, f32),
    rows: Rc<Vec<FlatRow>>,
    list: ListState,
    last_render: Instant,
    refresh_timer: Option<Task<()>>,
}

impl SidebarList {
    fn new() -> Self {
        Self {
            revision: u64::MAX,
            minute: 0,
            collapsed_agents: HashSet::new(),
            collapsed_projects: Vec::new(),
            layout: (0., 0.),
            rows: Rc::new(Vec::new()),
            list: ListState::new(0, ListAlignment::Top, px(160.)),
            last_render: Instant::now(),
            refresh_timer: None,
        }
    }
}

fn section(rows: &mut Vec<FlatRow>, scope: &str, label: &str) {
    rows.push(FlatRow {
        key: format!("section:{scope}:{label}"),
        row: Row::Section(label.to_owned()),
    });
}

fn tasks(
    rows: &mut Vec<FlatRow>,
    mut records: Vec<&Value>,
    show_project: bool,
    prefix: &'static str,
    scope: &str,
    snapshot: &Value,
    now: i64,
) {
    records.sort_by_key(|task| std::cmp::Reverse(task_activity_ms(task)));
    for older in [false, true] {
        let group: Vec<_> = records
            .iter()
            .copied()
            .filter(|task| task_is_older(task, now) == older)
            .collect();
        if group.is_empty() {
            continue;
        }
        section(rows, scope, if older { "Older" } else { "Recent" });
        for task in group {
            rows.push(FlatRow {
                key: format!("{prefix}:{}", field(task, "id")),
                row: Row::Task {
                    record: metadata(task),
                    context: show_project.then(|| project_name(snapshot, task)).flatten(),
                    prefix,
                },
            });
        }
    }
}

pub(super) fn render(
    desktop: &Desktop,
    agents: &[&Value],
    records: &[&Value],
    palette: design::Palette,
    window: &mut Window,
    cx: &mut Context<Desktop>,
) -> AnyElement {
    let view = desktop.window.sidebar_view;
    let view_key = match view {
        SidebarView::Agents => "agents",
        SidebarView::Projects => "projects",
        _ => "activity",
    };
    let cache = window.use_keyed_state(
        SharedString::from(format!("sidebar-list-{view_key}")),
        cx,
        |_, _| SidebarList::new(),
    );
    cache.update(cx, |state, cx| {
        state.last_render = Instant::now();
        if state.refresh_timer.is_none() {
            state.refresh_timer = Some(cx.spawn(async move |state, cx| {
                loop {
                    cx.background_executor()
                        .timer(Duration::from_secs(60))
                        .await;
                    let keep = state
                        .update(cx, |state, cx| {
                            // A hidden/detached sidebar stops its minute clock.
                            // Visible views refresh relative labels without token-driven root redraws.
                            if state.last_render.elapsed() > Duration::from_secs(90) {
                                state.refresh_timer = None;
                                return false;
                            }
                            cx.notify();
                            true
                        })
                        .unwrap_or(false);
                    if !keep {
                        break;
                    }
                }
            }));
        }
    });
    let projects = sidebar_rows(&desktop.snapshot, "projects");
    let project_states: Vec<_> = if view == SidebarView::Projects {
        projects
            .iter()
            .map(|project| {
                let id = field(project, "id");
                let state = window.use_keyed_state(
                    SharedString::from(format!("project-collapse-{id}")),
                    cx,
                    |_, _| ProjectCollapseState { collapsed: false },
                );
                (id, state.read(cx).collapsed, state)
            })
            .collect()
    } else {
        Vec::new()
    };
    let collapsed_projects: Vec<_> = project_states
        .iter()
        .map(|(id, collapsed, _)| (id.clone(), *collapsed))
        .collect();
    let now = sidebar_now_ms();
    let layout = (
        cx.theme().font_size.as_f32(),
        design::ui_scaled_px(1.).as_f32(),
    );
    let rebuild = {
        let previous = cache.read(cx);
        previous.revision != desktop.shell_metadata_revision
            || previous.minute != now / 60_000
            || previous.collapsed_agents != desktop.window.collapsed_agents
            || previous.collapsed_projects != collapsed_projects
    };
    if rebuild {
        let mut rows = Vec::new();
        // Group once per metadata publication, rather than scan every task for
        // every expanded owner. The row factory below only touches the viewport.
        let mut by_owner: HashMap<&str, Vec<&Value>> = HashMap::new();
        if view != SidebarView::Activity {
            let key = if view == SidebarView::Agents {
                "agentId"
            } else {
                "projectId"
            };
            for task in records {
                by_owner
                    .entry(task[key].as_str().unwrap_or(""))
                    .or_default()
                    .push(task);
            }
        }
        if view == SidebarView::Agents {
            let ids: HashSet<_> = agents.iter().map(|agent| field(agent, "id")).collect();
            for agent in agents {
                let id = field(agent, "id");
                let collapsed = desktop.window.collapsed_agents.contains(&id);
                rows.push(FlatRow {
                    key: format!("agent:{id}"),
                    row: Row::Agent {
                        record: metadata(agent),
                        collapsed,
                    },
                });
                if !collapsed {
                    tasks(
                        &mut rows,
                        by_owner.remove(id.as_str()).unwrap_or_default(),
                        true,
                        "agent-task",
                        &id,
                        &desktop.snapshot,
                        now,
                    );
                }
            }
            tasks(
                &mut rows,
                records
                    .iter()
                    .copied()
                    .filter(|task| !ids.contains(&field(task, "agentId")))
                    .collect(),
                true,
                "orphan",
                "orphan",
                &desktop.snapshot,
                now,
            );
        } else if view == SidebarView::Projects {
            let ids: HashSet<_> = project_states.iter().map(|(id, _, _)| id.clone()).collect();
            for (project, (id, collapsed, state)) in projects.iter().zip(project_states) {
                let owned = by_owner.remove(id.as_str()).unwrap_or_default();
                let running = owned.iter().any(|task| field(task, "status") == "running");
                rows.push(FlatRow {
                    key: format!("project:{id}"),
                    row: Row::Project {
                        record: metadata(project),
                        count: owned.len(),
                        running,
                        collapsed,
                        state,
                    },
                });
                if !collapsed {
                    tasks(
                        &mut rows,
                        owned,
                        false,
                        "project-task",
                        &id,
                        &desktop.snapshot,
                        now,
                    );
                }
            }
            let missing: BTreeSet<_> = records
                .iter()
                .map(|task| field(task, "projectId"))
                .filter(|id| !id.is_empty() && !ids.contains(id))
                .collect();
            for id in missing {
                section(&mut rows, &id, &format!("Other project · {id}"));
                tasks(
                    &mut rows,
                    by_owner.remove(id.as_str()).unwrap_or_default(),
                    false,
                    "unlisted-project-task",
                    &id,
                    &desktop.snapshot,
                    now,
                );
            }
            let unassigned = by_owner.remove("").unwrap_or_default();
            if !unassigned.is_empty() {
                section(&mut rows, "unassigned", "No project");
                tasks(
                    &mut rows,
                    unassigned,
                    false,
                    "unassigned-task",
                    "unassigned",
                    &desktop.snapshot,
                    now,
                );
            }
        } else {
            let mut running: Vec<_> = records
                .iter()
                .copied()
                .filter(|task| field(task, "status") == "running")
                .collect();
            running.sort_by_key(|task| std::cmp::Reverse(task["updatedAt"].as_i64().unwrap_or(0)));
            if !running.is_empty() {
                section(&mut rows, "activity", "Running");
                for task in running {
                    rows.push(FlatRow {
                        key: format!("activity-running:{}", field(task, "id")),
                        row: Row::Task {
                            record: metadata(task),
                            context: project_name(&desktop.snapshot, task),
                            prefix: "activity-running",
                        },
                    });
                }
            }
            tasks(
                &mut rows,
                records
                    .iter()
                    .copied()
                    .filter(|task| field(task, "status") != "running")
                    .collect(),
                true,
                "activity-recent",
                "activity-recent",
                &desktop.snapshot,
                now,
            );
        }
        if records.is_empty() {
            rows.push(FlatRow {
                key: "empty".into(),
                row: Row::Empty(match view {
                    SidebarView::Agents => "No chats yet. Use + beside an agent to start one.",
                    SidebarView::Projects => "No chats yet.",
                    _ => "No recent activity.",
                }),
            });
        }
        cache.update(cx, |cache, _| {
            let offset = cache.list.logical_scroll_top();
            let anchor = cache
                .rows
                .get(offset.item_ix)
                .and_then(|old| rows.iter().position(|row| row.key == old.key));
            let same_keys = cache.rows.len() == rows.len()
                && cache.rows.iter().zip(&rows).all(|(a, b)| a.key == b.key);
            if same_keys {
                cache.list.remeasure();
            } else {
                cache.list.reset(rows.len());
                if let Some(item_ix) = anchor {
                    cache.list.scroll_to(ListOffset {
                        item_ix,
                        offset_in_item: offset.offset_in_item,
                    });
                }
            }
            cache.rows = Rc::new(rows);
            cache.revision = desktop.shell_metadata_revision;
            cache.minute = now / 60_000;
            cache.collapsed_agents = desktop.window.collapsed_agents.clone();
            cache.collapsed_projects = collapsed_projects;
            cache.layout = layout;
        });
    } else if cache.read(cx).layout != layout {
        cache.update(cx, |cache, _| {
            cache.layout = layout;
            cache.list.remeasure();
        });
    }
    let rows = cache.read(cx).rows.clone();
    let state = cache.read(cx).list.clone();
    let owner = cx.weak_entity();
    let list = gpui_kit::list(state.clone(), move |index, window, cx| {
        let Some(row) = rows.get(index) else {
            return div().into_any_element();
        };
        owner
            .update(cx, |desktop, cx| {
                desktop.with_workspace_window(window, cx, |desktop, window, cx| {
                    let element = match &row.row {
                        Row::Task {
                            record,
                            context,
                            prefix,
                        } => desktop.task_row(record, context.clone(), palette, prefix, window, cx),
                        Row::Agent { record, collapsed } => {
                            desktop.sidebar_agent_row(record, *collapsed, palette, window, cx)
                        }
                        Row::Project {
                            record,
                            count,
                            running,
                            collapsed,
                            state,
                        } => Desktop::project_row(
                            record,
                            *count,
                            *running,
                            *collapsed,
                            state.clone(),
                            palette,
                            sidebar_text_px(11., &desktop.snapshot["settings"]),
                            cx,
                        )
                        .into_any_element(),
                        Row::Section(label) => {
                            Desktop::section_label(label, palette, &desktop.snapshot["settings"])
                                .into_any_element()
                        }
                        Row::Empty(label) => div()
                            .text_size(sidebar_text_px(11., &desktop.snapshot["settings"]))
                            .text_color(rgb(palette.muted))
                            .p(px(design::space::INSET))
                            .child(*label)
                            .into_any_element(),
                    };
                    div()
                        .w_full()
                        .pb(px(design::space::TIGHT))
                        .child(element)
                        .into_any_element()
                })
            })
            .unwrap_or_else(|_| div().into_any_element())
    })
    .size_full();
    div()
        .id("sidebar-tree")
        .flex_1()
        .min_h_0()
        .overflow_hidden()
        .px(px(design::space::INSET))
        .py(px(design::space::INSET))
        .when(
            design::tile_theme() == monitter_lib::experimental_desktop::NativeTileTheme::Tern,
            |tree| tree.pt_0(),
        )
        .child(VirtualListEdgeFade { child: list.into_any_element(), scroll: state })
        .into_any_element()
}
