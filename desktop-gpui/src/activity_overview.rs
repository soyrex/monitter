//! A retained, read-only map of the workspace's real chats and project teams.
use crate::design::{self, Palette, ui_scaled_px as px};
use gpui_kit::{
    component::{
        ActiveTheme as _, Sizable,
        button::*,
        input::{Input, InputEvent, InputState},
    },
    prelude::*,
    *,
};
use serde_json::Value;
use monitter_lib::experimental_desktop::NativeActivityGrouping;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::Path,
    rc::Rc,
    cell::{Cell, RefCell},
    time::Duration,
};

#[path = "activity_communications.rs"]
mod communications;

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or_default()
}
fn rows<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value[key].as_array().map(Vec::as_slice).unwrap_or(&[])
}
fn active(status: &str) -> bool {
    matches!(
        status,
        "running" | "queued" | "pending" | "starting" | "in_progress"
    )
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Working,
    NeedsYou,
    Failed,
    Done,
    Idle,
}
impl State {
    fn label(self) -> &'static str {
        match self {
            Self::Working => "Working",
            Self::NeedsYou => "Needs you",
            Self::Failed => "Failed",
            Self::Done => "Done",
            Self::Idle => "Idle",
        }
    }
    fn color(self, palette: Palette) -> u32 {
        design::status_color(
            match self {
                Self::Working => "running",
                Self::NeedsYou => "interrupted",
                Self::Failed => "error",
                Self::Done => "completed",
                Self::Idle => "idle",
            },
            palette,
        )
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Filter {
    All,
    Working,
    NeedsYou,
    Failed,
    Idle,
}
impl Filter {
    const ALL: [Self; 5] = [
        Self::All,
        Self::Working,
        Self::NeedsYou,
        Self::Failed,
        Self::Idle,
    ];
    fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Working => "Working",
            Self::NeedsYou => "Needs you",
            Self::Failed => "Failed",
            Self::Idle => "Idle",
        }
    }
    fn matches(self, state: State) -> bool {
        match self {
            Self::All => true,
            Self::Working => state == State::Working,
            Self::NeedsYou => state == State::NeedsYou,
            Self::Failed => state == State::Failed,
            Self::Idle => state == State::Idle,
        }
    }
}
#[derive(Clone, Debug)]
struct Card {
    task_id: String,
    title: String,
    agent: String,
    model: String,
    provider: String,
    project_id: String,
    project_name: String,
    workspace_id: String,
    workspace_name: String,
    workspace_path: String,
    state: State,
    activity: String,
    /// Individual plan item states, never an inferred completion percentage.
    steps: Vec<State>,
    children: usize,
    attention: usize,
    updated_at: i64,
}
impl Card {
    fn group_id(&self, grouping: NativeActivityGrouping) -> &str {
        match grouping {
            NativeActivityGrouping::Project => &self.project_id,
            NativeActivityGrouping::Workspace => &self.workspace_id,
        }
    }
    fn group_name(&self, grouping: NativeActivityGrouping) -> &str {
        match grouping {
            NativeActivityGrouping::Project => &self.project_name,
            NativeActivityGrouping::Workspace => &self.workspace_name,
        }
    }
}

fn workspace_identity(task: &Value) -> (String, String, String) {
    let path = text(task, "cwd").trim();
    let host = text(task, "hostId").trim();
    let host = if host.is_empty() { "local" } else { host };
    if path.is_empty() {
        return (
            format!("workspace:{host}:unknown"),
            "Unknown workspace".into(),
            String::new(),
        );
    }
    let name = Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or(path);
    (format!("workspace:{host}:{path}"), name.into(), path.into())
}
#[derive(Clone, Default)]
struct OverviewModel {
    cards: Vec<Card>,
    search_text: Vec<String>,
    filter_counts: [usize; 5],
    group_counts: [usize; 2],
}
impl OverviewModel {
    #[cfg(test)]
    fn from_snapshot(snapshot: &Value) -> Self {
        Self::from_snapshot_at(snapshot, 0)
    }
    #[cfg(test)]
    fn from_snapshot_at(snapshot: &Value, now: i64) -> Self {
        Self::from_indexed_snapshot_at(snapshot, &child_counts(snapshot), now)
    }
    fn from_indexed_snapshot(snapshot: &Value, child_counts: &HashMap<String, usize>) -> Self {
        Self::from_indexed_snapshot_at(
            snapshot,
            child_counts,
            chrono::Utc::now().timestamp_millis(),
        )
    }
    fn from_indexed_snapshot_at(
        snapshot: &Value,
        child_counts: &HashMap<String, usize>,
        now: i64,
    ) -> Self {
        let agents: HashMap<_, _> = rows(snapshot, "agents")
            .iter()
            .map(|row| (text(row, "id"), row))
            .collect();
        let projects: HashMap<_, _> = rows(snapshot, "projects")
            .iter()
            .map(|row| (text(row, "id"), row))
            .collect();
        let mut pending_by_task = HashMap::<&str, Vec<&Value>>::new();
        for request in rows(snapshot, "approvalRequests")
            .iter()
            .filter(|r| text(r, "status") == "pending")
        {
            pending_by_task
                .entry(text(request, "taskId"))
                .or_default()
                .push(request);
        }
        // Compact native publications carry the turn boundary without retaining transcripts.
        let mut user_at = HashMap::<&str, i64>::new();
        if let Some(contexts) = snapshot["nativeMessageContext"].as_object() {
            for (id, context) in contexts {
                if let Some(at) = context["latestUserAt"].as_i64() {
                    user_at.insert(id, at);
                }
            }
        } else {
            for message in rows(snapshot, "messages")
                .iter()
                .filter(|m| text(m, "role") == "user")
            {
                let at = message["createdAt"].as_i64().unwrap_or(0);
                user_at
                    .entry(text(message, "taskId"))
                    .and_modify(|old| *old = (*old).max(at))
                    .or_insert(at);
            }
        }
        let mut latest_by_task = HashMap::<&str, &Value>::new();
        for event in rows(snapshot, "events") {
            let id = text(event, "taskId");
            let at = event["createdAt"].as_i64().unwrap_or(0);
            if text(event, "title").trim().is_empty()
                || user_at.get(id).is_some_and(|start| at < *start)
            {
                continue;
            }
            latest_by_task
                .entry(id)
                .and_modify(|old| {
                    if at > old["createdAt"].as_i64().unwrap_or(0) {
                        *old = event;
                    }
                })
                .or_insert(event);
        }
        let mut plan_by_task = HashMap::<&str, &Value>::new();
        let plan_order = |plan: &Value| {
            (
                text(plan, "status") == "active",
                plan["updatedAt"].as_i64().unwrap_or(0),
                plan["createdAt"].as_i64().unwrap_or(0),
            )
        };
        for plan in rows(snapshot, "workPlans") {
            plan_by_task
                .entry(text(plan, "taskId"))
                .and_modify(|old| {
                    if plan_order(plan) > plan_order(old) {
                        *old = plan;
                    }
                })
                .or_insert(plan);
        }
        let mut seen = HashSet::new();
        let mut cards = Vec::new();
        for task in rows(snapshot, "tasks") {
            let id = text(task, "id");
            if id.is_empty() || task["archived"].as_bool().unwrap_or(false) || !seen.insert(id) {
                continue;
            }
            let agent = agents.get(text(task, "agentId")).copied();
            if agent.is_some_and(|agent| agent["internal"] == true) {
                continue;
            }
            let project_id = text(task, "projectId");
            let project = projects.get(project_id).copied();
            let project_name = project
                .map(|p| text(p, "name"))
                .filter(|s| !s.is_empty())
                .unwrap_or(if project_id.is_empty() {
                    "No project"
                } else {
                    "Unavailable project"
                });
            let pending = pending_by_task.get(id).map(Vec::as_slice).unwrap_or(&[]);
            let status = text(task, "status");
            let state = if !pending.is_empty() {
                State::NeedsYou
            } else if matches!(status, "error" | "failed") {
                State::Failed
            } else if active(status) {
                State::Working
            } else if matches!(status, "completed" | "succeeded") {
                State::Done
            } else {
                State::Idle
            };
            let latest = latest_by_task.get(id).copied();
            let activity = pending
                .first()
                .and_then(|request| {
                    rows(&request["input"], "questions")
                        .iter()
                        .map(|q| text(q, "question"))
                        .find(|value| !value.trim().is_empty())
                        .or_else(|| {
                            let summary = text(request, "summary");
                            (!summary.trim().is_empty()).then_some(summary)
                        })
                })
                .or_else(|| latest.map(|event| text(event, "title")))
                .unwrap_or(match state {
                    State::NeedsYou => "A response is needed",
                    State::Working => "Agent is working",
                    State::Failed => "Open Watch to inspect the failure",
                    State::Done => "Work completed",
                    State::Idle => "Waiting for the next message",
                })
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            let plan = plan_by_task.get(id).copied();
            let steps = plan
                .map(|plan| {
                    rows(plan, "items")
                        .iter()
                        .map(|item| match text(item, "status") {
                            "completed" | "done" => State::Done,
                            "in_progress" | "running" => State::Working,
                            "failed" | "error" => State::Failed,
                            "blocked" => State::NeedsYou,
                            _ => State::Idle,
                        })
                        .collect()
                })
                .unwrap_or_default();
            let updated_at = [
                task["updatedAt"].as_i64().unwrap_or(0),
                task["createdAt"].as_i64().unwrap_or(0),
                user_at.get(id).copied().unwrap_or(0),
                latest_by_task
                    .get(id)
                    .and_then(|event| event["createdAt"].as_i64())
                    .unwrap_or(0),
                plan.and_then(|plan| plan["updatedAt"].as_i64())
                    .unwrap_or(0),
            ]
            .into_iter()
            .max()
            .unwrap_or(0);
            if updated_at < now.saturating_sub(12 * 60 * 60 * 1000) {
                continue;
            }
            let children = child_counts.get(id).copied().unwrap_or(0);
            let agent_name = agent
                .map(|agent| text(agent, "name"))
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| {
                    let provider = text(task, "provider");
                    if provider.is_empty() {
                        "Agent"
                    } else {
                        provider
                    }
                });
            let title = text(task, "title");
            let (workspace_id, workspace_name, workspace_path) = workspace_identity(task);
            cards.push(Card {
                task_id: id.into(),
                title: if title.is_empty() {
                    "Untitled chat".into()
                } else {
                    title.into()
                },
                agent: agent_name.into(),
                model: text(task, "model").into(),
                provider: text(task, "provider").into(),
                project_id: project_id.into(),
                project_name: project_name.into(),
                workspace_id,
                workspace_name,
                workspace_path,
                state,
                activity,
                steps,
                children,
                attention: pending.len(),
                updated_at,
            });
        }
        cards.sort_by(|a, b| {
            a.project_name
                .cmp(&b.project_name)
                .then_with(|| state_rank(a.state).cmp(&state_rank(b.state)))
                .then_with(|| b.updated_at.cmp(&a.updated_at))
                .then_with(|| a.task_id.cmp(&b.task_id))
        });
        let search_text = cards.iter().map(|card| format!("{} {} {} {} {} {}",
            card.title, card.agent, card.model, card.project_name, card.workspace_name, card.workspace_path,
        ).to_lowercase()).collect();
        let filter_counts = std::array::from_fn(|index| cards.iter()
            .filter(|card| Filter::ALL[index].matches(card.state)).count());
        let group_counts = [NativeActivityGrouping::Project, NativeActivityGrouping::Workspace]
            .map(|grouping| cards.iter().map(|card| card.group_id(grouping))
                .filter(|id| !id.is_empty()).collect::<HashSet<_>>().len());
        Self { cards, search_text, filter_counts, group_counts }
    }
    fn visible(
        &self,
        filter: Filter,
        query: &str,
        grouping: NativeActivityGrouping,
    ) -> Vec<Card> {
        let query = query.trim().to_lowercase();
        // Establish group membership before search so a matching quiet card
        // remains visible even when its working sibling does not match the query.
        let working_groups: HashSet<_> = if filter == Filter::Working {
            self.cards.iter().filter(|card| card.state == State::Working)
                .map(|card| card.group_id(grouping)).collect()
        } else {
            HashSet::new()
        };
        self.cards
            .iter()
            .enumerate()
            .filter(|(index, card)| {
                (if filter == Filter::Working {
                    working_groups.contains(card.group_id(grouping))
                } else {
                    filter.matches(card.state)
                })
                    && (query.is_empty() || self.search_text[*index].contains(&query))
            })
            .map(|(_, card)| card.clone())
            .collect()
    }
    fn count(&self, filter: Filter) -> usize {
        self.filter_counts[Filter::ALL.iter().position(|candidate| *candidate == filter).unwrap()]
    }
    fn group_count(&self, grouping: NativeActivityGrouping) -> usize {
        self.group_counts[match grouping {
            NativeActivityGrouping::Project => 0,
            NativeActivityGrouping::Workspace => 1,
        }]
    }
}
fn child_counts(snapshot: &Value) -> HashMap<String, usize> {
    // Reuse Watch's exact native/routed deduplication only for delegation parents.
    let parents: HashSet<_> = rows(snapshot, "subagentSessions")
        .iter()
        .map(|s| text(s, "parentTaskId"))
        .chain(
            rows(snapshot, "collaborations")
                .iter()
                .filter(|c| text(c, "kind") == "delegation")
                .map(|c| text(c, "fromTaskId")),
        )
        .filter(|id| !id.is_empty())
        .collect();
    parents
        .into_iter()
        .map(|id| {
            (
                id.to_owned(),
                crate::collaboration::records(snapshot, id).len(),
            )
        })
        .collect()
}
fn relevant_changed(previous: &Value, next: &Value) -> bool {
    [
        "tasks",
        "agents",
        "projects",
        "approvalRequests",
        "workPlans",
        "nativeMessageContext",
        "events",
        "subagentSessions",
        "collaborations",
    ]
    .iter()
    .any(|key| previous[*key] != next[*key])
        || !rows(previous, "messages")
            .iter()
            .filter(|m| text(m, "role") == "user")
            .map(|m| (text(m, "taskId"), m["createdAt"].as_i64()))
            .eq(rows(next, "messages")
                .iter()
                .filter(|m| text(m, "role") == "user")
                .map(|m| (text(m, "taskId"), m["createdAt"].as_i64())))
}
fn child_inputs_changed(previous: &Value, next: &Value) -> bool {
    previous["subagentSessions"] != next["subagentSessions"]
        || previous["collaborations"] != next["collaborations"]
        || !rows(previous, "tasks")
            .iter()
            .map(|t| (text(t, "id"), text(t, "nativeSessionId")))
            .eq(rows(next, "tasks")
                .iter()
                .map(|t| (text(t, "id"), text(t, "nativeSessionId"))))
}
fn state_rank(state: State) -> u8 {
    match state {
        State::NeedsYou => 0,
        State::Working => 1,
        State::Failed => 2,
        State::Done => 3,
        State::Idle => 4,
    }
}
fn clamped_zoom(zoom: f32) -> f32 {
    if zoom.is_finite() {
        zoom.clamp(0.5, 1.5)
    } else {
        1.
    }
}

fn working_indicator(size: Pixels, color: u32, motion: bool) -> AnyElement {
    let icon = gpui_kit::component::Icon::new(gpui_kit::assets::IconName::LoaderCircle).size(size);
    if motion {
        gpui_kit::component::spinner::Spinner::new().with_size(size)
            .color(rgb(color).into()).into_any_element()
    } else { icon.text_color(rgb(color)).into_any_element() }
}

const CARD_W: f32 = 199.;
const CARD_H: f32 = 104.;
const HUB_W: f32 = 170.;
const HUB_H: f32 = 46.;
fn interface_font_ratio(font: Pixels) -> f32 {
    (f32::from(font) / f32::from(px(design::INTERFACE_FONT_SIZE))).max(0.5)
}
fn updated_label(updated_at: i64, now: i64) -> String {
    if updated_at <= 0 {
        return String::new();
    }
    let minutes = now.saturating_sub(updated_at).max(0) / 60_000;
    if minutes < 1 {
        "updated now".into()
    } else if minutes < 60 {
        format!("updated {minutes}m")
    } else if minutes < 1440 {
        format!("updated {}h", minutes / 60)
    } else {
        format!("updated {}d", minutes / 1440)
    }
}
#[derive(Clone, Copy, Debug)]
struct Placement {
    x: f32,
    y: f32,
}
fn panned_placement(current: Placement, delta: Placement, limit_x: f32, limit_y: f32) -> Placement {
    Placement {
        x: (current.x + delta.x).clamp(-limit_x, limit_x),
        y: (current.y + delta.y).clamp(-limit_y, limit_y),
    }
}
#[derive(Clone, Copy, Debug)]
struct MapRect {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}
impl MapRect {
    fn overlaps(self, other: Self, gap: f32) -> bool {
        self.x < other.x + other.w + gap
            && self.x + self.w + gap > other.x
            && self.y < other.y + other.h + gap
            && self.y + self.h + gap > other.y
    }
}
// A deterministic golden-angle search places every new rectangle at the first
// free point near the cluster centre. This keeps groups close without snapping
// their edges into rows and columns.
fn organic_positions(sizes: &[(f32, f32)], reserved: &[MapRect], gap: f32) -> Vec<MapRect> {
    organic_positions_for_aspect(sizes, reserved, gap, 1.)
}
fn organic_positions_for_aspect(
    sizes: &[(f32, f32)],
    reserved: &[MapRect],
    gap: f32,
    aspect: f32,
) -> Vec<MapRect> {
    let mut occupied = reserved.to_vec();
    let mut result = Vec::new();
    const GOLDEN_ANGLE: f32 = 2.399_963_1;
    let x_bias = aspect.sqrt().clamp(0.65, 1.55);
    let y_bias = 1. / x_bias;
    for (index, &(w, h)) in sizes.iter().enumerate() {
        let best = if occupied.is_empty() {
            MapRect { x: -w / 2., y: -h / 2., w, h }
        } else {
            let step = ((w.min(h) + gap) * 0.22).max(12.);
            (0..4096)
                .map(|sample| {
                    let radius = step * (sample as f32).sqrt();
                    let angle = sample as f32 * GOLDEN_ANGLE + index as f32 * 0.73;
                    MapRect {
                        x: angle.cos() * radius * x_bias - w / 2.,
                        y: angle.sin() * radius * y_bias - h / 2.,
                        w,
                        h,
                    }
                })
                .find(|rect| occupied.iter().all(|other| !rect.overlaps(*other, gap - 0.01)))
                .unwrap_or(MapRect {
                    x: 0.,
                    y: occupied.iter().map(|rect| rect.y + rect.h + gap).fold(0., f32::max),
                    w,
                    h,
                })
        };
        occupied.push(best);
        result.push(best);
    }
    result
}

/// Pack project clusters into a compact organic island, then centre that island
/// in the available map surface. The packing is recomputed for the live viewport
/// so a narrow or tall Activity pane forms a different, natural silhouette.
fn distributed_positions(
    sizes: &[(f32, f32)],
    gap: f32,
    available_width: f32,
    available_height: f32,
) -> (Vec<MapRect>, f32, f32) {
    if sizes.is_empty() {
        return (Vec::new(), available_width.max(0.), available_height.max(0.));
    }

    let available_width = available_width.max(sizes.iter().map(|(w, _)| *w).fold(0., f32::max));
    let available_height = available_height.max(sizes.iter().map(|(_, h)| *h).fold(0., f32::max));
    let mut positions = organic_positions_for_aspect(
        sizes,
        &[],
        gap,
        (available_width / available_height.max(1.)).max(0.01),
    );
    let min_x = positions.iter().map(|rect| rect.x).fold(f32::INFINITY, f32::min);
    let min_y = positions.iter().map(|rect| rect.y).fold(f32::INFINITY, f32::min);
    let max_x = positions.iter().map(|rect| rect.x + rect.w).fold(f32::NEG_INFINITY, f32::max);
    let max_y = positions.iter().map(|rect| rect.y + rect.h).fold(f32::NEG_INFINITY, f32::max);
    let packed_width = max_x - min_x;
    let packed_height = max_y - min_y;
    let canvas_width = available_width.max(packed_width + gap * 2.);
    let canvas_height = available_height.max(packed_height + gap * 2.);
    let origin_x = (canvas_width - packed_width) / 2. - min_x;
    let origin_y = (canvas_height - packed_height) / 2. - min_y;
    for rect in &mut positions {
        rect.x += origin_x;
        rect.y += origin_y;
    }
    (positions, canvas_width, canvas_height)
}
// Cast dense angular rays from the hub. Each expanded rectangle contributes a
// forbidden radial interval; the first free interval packs satellites inward.
fn satellite_position(
    hub: MapRect,
    occupied: &[MapRect],
    ordinal: usize,
    nonnegative: bool,
) -> MapRect {
    let center = (hub.x + hub.w / 2., hub.y + hub.h / 2.);
    let phase = ordinal as f32 * 2.3999632;
    (0..72)
        .filter_map(|i| {
            let angle = phase + i as f32 * std::f32::consts::TAU / 72.;
            let (dx, dy) = (angle.cos() * 1.55, angle.sin());
            let mut intervals = Vec::new();
            for r in occupied {
                let gap = if r.x == hub.x && r.y == hub.y && r.w == hub.w && r.h == hub.h { 24. } else { 10. };
                let axis = |low: f32, high: f32, origin: f32, d: f32| {
                    if d.abs() < 0.00001 {
                        if origin > low && origin < high {
                            Some((f32::NEG_INFINITY, f32::INFINITY))
                        } else {
                            None
                        }
                    } else {
                        let a = (low - origin) / d;
                        let b = (high - origin) / d;
                        Some((a.min(b), a.max(b)))
                    }
                };
                let Some((xl, xh)) = axis(
                    r.x - gap - CARD_W / 2.,
                    r.x + r.w + gap + CARD_W / 2.,
                    center.0,
                    dx,
                ) else {
                    continue;
                };
                let Some((yl, yh)) = axis(
                    r.y - gap - CARD_H / 2.,
                    r.y + r.h + gap + CARD_H / 2.,
                    center.1,
                    dy,
                ) else {
                    continue;
                };
                let low = xl.max(yl);
                let high = xh.min(yh);
                if high >= low && high >= 0. {
                    intervals.push((low, high));
                }
            }
            intervals.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut radius = 0.;
            for (low, high) in intervals {
                if low > radius + 0.001 {
                    break;
                }
                if high >= radius {
                    radius = high + 0.02;
                }
            }
            let rect = MapRect {
                x: center.0 + dx * radius - CARD_W / 2.,
                y: center.1 + dy * radius - CARD_H / 2.,
                w: CARD_W,
                h: CARD_H,
            };
            if nonnegative && (rect.x < 12. || rect.y < 12.) {
                return None;
            }
            Some((radius + i as f32 * 0.0001, rect))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, r)| r)
        .unwrap_or_else(|| MapRect {
            x: hub.x,
            y: occupied.iter().map(|r| r.y + r.h + 10.).fold(12., f32::max),
            w: CARD_W,
            h: CARD_H,
        })
}
fn normalized_layout(mut rects: Vec<MapRect>) -> (Vec<MapRect>, f32, f32) {
    let left = rects.iter().map(|r| r.x).fold(0., f32::min) - 12.;
    let top = rects.iter().map(|r| r.y).fold(0., f32::min) - 12.;
    for rect in &mut rects {
        rect.x -= left;
        rect.y -= top;
    }
    let width = rects.iter().map(|r| r.x + r.w).fold(0., f32::max) + 12.;
    let height = rects.iter().map(|r| r.y + r.h).fold(0., f32::max) + 12.;
    (rects, width, height)
}
fn cluster_layout(count: usize) -> (Vec<MapRect>, f32, f32) {
    let hub = MapRect {
        x: 0.,
        y: 0.,
        w: HUB_W,
        h: HUB_H,
    };
    let mut rects = vec![hub];
    for ordinal in 0..count {
        let rect = satellite_position(hub, &rects, ordinal, false);
        rects.push(rect);
    }
    normalized_layout(rects)
}
fn grouped(
    cards: Vec<Card>,
    grouping: NativeActivityGrouping,
) -> Vec<(String, Vec<Card>)> {
    let mut groups = BTreeMap::<(String, String), Vec<Card>>::new();
    for card in cards {
        groups
            .entry((
                card.group_name(grouping).to_owned(),
                card.group_id(grouping).to_owned(),
            ))
            .or_default()
            .push(card);
    }
    let mut groups: Vec<_> = groups
        .into_iter()
        .map(|((name, _), cards)| (name, cards))
        .collect();
    groups.sort_by_key(|(_, cards): &(String, Vec<Card>)| {
        std::cmp::Reverse((
            cards.iter().filter(|c| c.state == State::Working).count(),
            cards.len(),
            cards.iter().map(|c| c.updated_at).max().unwrap_or(0),
        ))
    });
    groups
}

#[derive(Clone, Copy, Debug, Default)]
struct Motion {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
}
impl Motion {
    // Velocity is in map pixels per 16ms tick. Dragging caps throw speed, and
    // exponential drag gives a short, predictable glide rather than endless drift.
    fn coast(&mut self, min_x: f32, min_y: f32) -> bool {
        self.x += self.vx;
        self.y += self.vy;
        if self.x < min_x { self.x = min_x; self.vx = 0.; }
        if self.y < min_y { self.y = min_y; self.vy = 0.; }
        self.vx *= 0.86;
        self.vy *= 0.86;
        if self.vx.hypot(self.vy) < 0.12 { self.vx = 0.; self.vy = 0.; }
        self.vx != 0. || self.vy != 0.
    }
    fn stop(&mut self) { self.vx = 0.; self.vy = 0.; }
    fn settle(&mut self, target: Placement) -> bool {
        self.spring(target, 0.16)
    }
    fn spring(&mut self, target: Placement, stiffness: f32) -> bool {
        self.vx = (self.vx + (target.x - self.x) * stiffness) * 0.64;
        self.vy = (self.vy + (target.y - self.y) * stiffness) * 0.64;
        self.x += self.vx;
        self.y += self.vy;
        let moving =
            (self.x - target.x).abs() + (self.y - target.y).abs() + self.vx.abs() + self.vy.abs()
                > 0.1;
        if !moving {
            self.x = target.x;
            self.y = target.y;
            self.vx = 0.;
            self.vy = 0.;
        }
        moving
    }
}
#[derive(Default)]
struct RetainedProject {
    cards: HashMap<String, MapRect>,
    card_motion: HashMap<String, Motion>,
    hub: Option<MapRect>,
    bounds: Option<MapRect>,
    target: Motion,
    followers: HashMap<String, Motion>,
    view_cards: Vec<String>,
}
impl RetainedProject {
    fn follower(&self, id: &str) -> Motion {
        self.followers.get(id).copied().unwrap_or(self.target)
    }
    fn spring_strength(&self, id: &str) -> f32 {
        let rect = self.card_rect(id);
        let hub = self.hub.unwrap();
        let distance = (rect.x + rect.w / 2. - hub.x - hub.w / 2.)
            .hypot(rect.y + rect.h / 2. - hub.y - hub.h / 2.);
        // Distance adds a little weight; stable variation keeps equidistant
        // siblings from moving as one rigid group. No randomness or slot changes.
        let variation = id.bytes().fold(0u32, |hash, byte| hash.wrapping_mul(31).wrapping_add(byte as u32)) % 17;
        (0.20 / (1. + distance / 450.)) * (0.94 + variation as f32 * 0.0075)
    }
    fn card_rect(&self, id: &str) -> MapRect {
        let mut rect = self.cards[id];
        if let Some(offset) = self.card_motion.get(id) { rect.x += offset.x; rect.y += offset.y; }
        rect
    }
}
#[derive(Clone)]
enum DragTarget { Project(String), Card(String, String) }
struct DragState {
    target: DragTarget,
    start: Point<Pixels>,
    last: Point<Pixels>,
    sampled_at: std::time::Instant,
    velocity: Placement,
    moved: bool,
}
impl DragState {
    fn new(target: DragTarget, position: Point<Pixels>) -> Self {
        Self { target, start: position, last: position, sampled_at: std::time::Instant::now(),
            velocity: Placement { x: 0., y: 0. }, moved: false }
    }
}
struct CanvasPanState {
    last: Point<Pixels>,
}
struct RetainedLayout {
    projects: HashMap<String, RetainedProject>,
    canvas_width: f32,
    canvas_height: f32,
    available_width: f32,
    available_height: f32,
}
impl Default for RetainedLayout {
    fn default() -> Self {
        Self {
            projects: HashMap::new(),
            canvas_width: 0.,
            canvas_height: 0.,
            available_width: -1.,
            available_height: -1.,
        }
    }
}
impl RetainedLayout {
    #[cfg(test)]
    fn sync(&mut self, cards: Vec<Card>, available_width: f32, available_height: f32,
        grouping: NativeActivityGrouping) {
        self.sync_groups(&grouped(cards, grouping), available_width, available_height, grouping);
    }
    fn sync_groups(
        &mut self,
        groups: &[(String, Vec<Card>)],
        available_width: f32,
        available_height: f32,
        grouping: NativeActivityGrouping,
    ) {
        let mut view_changed = false;
        let mut group_sizes = Vec::with_capacity(groups.len());
        for (_, cards) in groups {
            let key = cards[0].group_id(grouping).to_owned();
            let project = self.projects.entry(key.clone()).or_default();
            if !project.view_cards.iter().map(String::as_str)
                .eq(cards.iter().map(|card| card.task_id.as_str())) {
                let view_cards: Vec<_> = cards.iter().map(|card| card.task_id.clone()).collect();
                if project.hub.is_none() {
                    let (placements, _, _) = cluster_layout(cards.len());
                    project.hub = Some(placements[0]);
                    for (card, rect) in cards.iter().zip(placements.iter().skip(1)) {
                        project.cards.insert(card.task_id.clone(), *rect);
                    }
                }
                let hub = project.hub.unwrap();
                for card in cards.iter() {
                    if !project.cards.contains_key(&card.task_id) {
                        let mut occupied: Vec<_> = project.cards.values().copied().collect();
                        occupied.push(hub);
                        let rect = satellite_position(hub, &occupied, project.cards.len(), true);
                        project.cards.insert(card.task_id.clone(), rect);
                    }
                }
                project.view_cards = view_cards;
                view_changed = true;
            }
            for id in &project.view_cards {
                if !project.followers.contains_key(id) {
                    project.followers.insert(id.clone(), project.target);
                }
            }
            let w = project
                .view_cards
                .iter()
                .map(|id| { let r = project.card_rect(id); r.x + r.w + 12. })
                .fold(HUB_W + 24., f32::max);
            let h = project
                .view_cards
                .iter()
                .map(|id| { let r = project.card_rect(id); r.y + r.h + 12. })
                .fold(HUB_H + 24., f32::max);
            group_sizes.push((key, w, h));
        }

        let viewport_changed = (self.available_width - available_width).abs() > 0.5
            || (self.available_height - available_height).abs() > 0.5;
        if view_changed || viewport_changed {
            let sizes: Vec<_> = group_sizes.iter().map(|(_, width, height)| (*width, *height)).collect();
            let (positions, canvas_width, canvas_height) =
                distributed_positions(&sizes, 8., available_width, available_height);
            for ((key, _, _), position) in group_sizes.iter().zip(positions) {
                self.projects.get_mut(key).unwrap().bounds = Some(position);
            }
            self.canvas_width = canvas_width;
            self.canvas_height = canvas_height;
            self.available_width = available_width;
            self.available_height = available_height;
        } else {
            for (key, width, height) in &group_sizes {
                if let Some(bounds) = self.projects.get_mut(key).and_then(|project| project.bounds.as_mut()) {
                    bounds.w = *width;
                    bounds.h = *height;
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
pub enum ActivityOverviewEvent {
    OpenTask { task_id: String },
    GroupingChanged { grouping: NativeActivityGrouping },
    Close,
}
impl EventEmitter<ActivityOverviewEvent> for ActivityOverview {}
// Filtering, normalization and grouping depend on publications and controls, not animation ticks.
struct PreparedActivityView {
    revision: u64,
    filter: Filter,
    query: String,
    grouping: NativeActivityGrouping,
    cards: Rc<Vec<Card>>,
    groups: Rc<Vec<(String, Vec<Card>)>>,
    list_rows: Rc<Vec<(usize, Option<usize>)>>,
    row_keys: Vec<String>,
}
#[derive(Clone, Copy)]
struct PaintedMapViewport {
    origin: Point<Pixels>,
    clip: Bounds<Pixels>,
    pan: Placement,
    scale: f32,
}

pub struct ActivityOverview {
    focus: FocusHandle,
    search: Entity<InputState>,
    snapshot: Rc<Value>,
    palette: Palette,
    model: OverviewModel,
    model_revision: u64,
    prepared_view: Option<PreparedActivityView>,
    list_state: ListState,
    list_metrics: Option<(Pixels, u16, SharedString)>,
    layout_input: Option<(u64, f32, f32)>,
    view_revision: u64,
    painted_map: Rc<Cell<Option<PaintedMapViewport>>>,
    child_counts: HashMap<String, usize>,
    grouping: NativeActivityGrouping,
    filter: Filter,
    list: bool,
    zoom: f32,
    layout: RetainedLayout,
    dragging: Option<DragState>,
    canvas_pan: Placement,
    canvas_pan_drag: Option<CanvasPanState>,
    suppress_card_click: Option<String>,
    motion_task: Option<Task<()>>,
    communications: Rc<RefCell<communications::Communications>>,
    _communication_clock: Task<()>,
    _subscription: Subscription,
    _refresh: Task<()>,
}
impl ActivityOverview {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::new_with_grouping(window, cx, NativeActivityGrouping::default())
    }
    pub fn new_with_grouping(
        window: &mut Window,
        cx: &mut Context<Self>,
        grouping: NativeActivityGrouping,
    ) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Find an agent or chat…"));
        let subscription = cx.subscribe(&search, |_, _, _: &InputEvent, cx| cx.notify());
        let refresh = cx.spawn(async move |overview, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(60))
                    .await;
                if overview
                    .update(cx, |overview, cx| {
                        overview.model_revision = overview.model_revision.wrapping_add(1);
                        overview.model = OverviewModel::from_indexed_snapshot(
                            &overview.snapshot,
                            &overview.child_counts,
                        );
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        let communications = Rc::new(RefCell::new(communications::Communications::default()));
        let clock_state = communications.clone();
        let communication_clock = cx.spawn_in(window, async move |overview, cx| {
            let mut previous = std::time::Instant::now();
            loop {
                cx.background_executor().timer(Duration::from_millis(100)).await;
                let now = std::time::Instant::now();
                let elapsed = now.duration_since(previous);
                previous = now;
                if overview.update_in(cx, |_, window, cx| {
                    if clock_state.borrow_mut().tick(elapsed, window.is_window_active()) { cx.notify(); }
                }).is_err() { break; }
            }
        });
        Self {
            focus: cx.focus_handle(),
            search,
            snapshot: Rc::new(Value::Null),
            palette: design::palette(&Value::Null, window.appearance()),
            model: OverviewModel::default(),
            model_revision: 0,
            prepared_view: None,
            list_state: ListState::new(0, ListAlignment::Top, px(300.)),
            list_metrics: None,
            layout_input: None,
            view_revision: 0,
            painted_map: Rc::new(Cell::new(None)),
            child_counts: HashMap::new(),
            grouping,
            filter: Filter::All,
            list: false,
            zoom: 1.,
            layout: RetainedLayout::default(),
            dragging: None,
            canvas_pan: Placement { x: 0., y: 0. },
            canvas_pan_drag: None,
            suppress_card_click: None,
            motion_task: None,
            communications,
            _communication_clock: communication_clock,
            _subscription: subscription,
            _refresh: refresh,
        }
    }
    fn set_grouping(
        &mut self,
        grouping: NativeActivityGrouping,
        cx: &mut Context<Self>,
    ) {
        if self.grouping == grouping {
            return;
        }
        self.grouping = grouping;
        self.layout = RetainedLayout::default();
        self.layout_input = None;
        self.painted_map.set(None);
        self.dragging = None;
        self.canvas_pan = Placement { x: 0., y: 0. };
        self.canvas_pan_drag = None;
        self.motion_task = None;
        cx.emit(ActivityOverviewEvent::GroupingChanged { grouping });
        cx.notify();
    }
    pub fn set_snapshot(&mut self, snapshot: Rc<Value>, palette: Palette, cx: &mut Context<Self>) {
        let mut changed = self.palette != palette;
        if !Rc::ptr_eq(&self.snapshot, &snapshot) {
            if self.snapshot.is_null() || self.snapshot["collaborations"] != snapshot["collaborations"] {
                self.communications.borrow_mut().ingest(&snapshot, chrono::Utc::now().timestamp_millis());
                changed = true;
            }
            if relevant_changed(&self.snapshot, &snapshot) {
                if child_inputs_changed(&self.snapshot, &snapshot) {
                    self.child_counts = child_counts(&snapshot);
                }
                self.model_revision = self.model_revision.wrapping_add(1);
                self.model = OverviewModel::from_indexed_snapshot(&snapshot, &self.child_counts);
                changed = true;
            }
            self.snapshot = snapshot;
        }
        self.palette = palette;
        if changed {
            cx.notify();
        }
    }
    #[cfg(test)]
    pub(super) fn communication_viewed_time(&self, a: &str, b: &str) -> Option<Duration> {
        let pair = if a < b { (a.to_owned(), b.to_owned()) } else { (b.to_owned(), a.to_owned()) };
        self.communications.borrow().links.get(&pair).map(|link| link.viewed)
    }
    fn motion_enabled(cx: &App) -> bool {
        !cx.reduce_motion() && crate::native_motion::motion_allowed()
    }
    fn begin_drag(&mut self, target: DragTarget, position: Point<Pixels>, cx: &mut Context<Self>) {
        self.canvas_pan_drag = None;
        self.suppress_card_click = None;
        match &target {
            DragTarget::Project(key) => self.layout.projects.get_mut(key).unwrap().target.stop(),
            DragTarget::Card(key, id) => self.layout.projects.get_mut(key).unwrap().card_motion.entry(id.clone()).or_default().stop(),
        }
        self.dragging = Some(DragState::new(target, position));
        cx.notify();
    }
    fn begin_canvas_pan(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        self.dragging = None;
        self.canvas_pan_drag = Some(CanvasPanState { last: position });
        cx.notify();
    }
    fn end_pointer_drag(&mut self, cx: &mut Context<Self>) {
        if self.canvas_pan_drag.take().is_some() {
            cx.notify();
        }
        self.end_drag(cx);
    }
    fn end_drag(&mut self, cx: &mut Context<Self>) {
        let Some(drag) = self.dragging.take() else { return; };
        let velocity = if drag.moved && drag.sampled_at.elapsed() < Duration::from_millis(100) && Self::motion_enabled(cx) {
            drag.velocity
        } else { Placement { x: 0., y: 0. } };
        let motion = match drag.target {
            DragTarget::Project(key) => &mut self.layout.projects.get_mut(&key).unwrap().target,
            DragTarget::Card(key, id) => {
                if drag.moved { self.suppress_card_click = Some(id.clone()); }
                self.layout.projects.get_mut(&key).unwrap().card_motion.entry(id).or_default()
            }
        };
        motion.vx = velocity.x;
        motion.vy = velocity.y;
        self.start_motion(cx);
    }
    fn drag_mouse_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        if self.canvas_pan_drag.is_some() && event.pressed_button != Some(MouseButton::Left) {
            self.canvas_pan_drag = None;
            cx.notify();
            return;
        }
        if let Some(pan) = self.canvas_pan_drag.as_mut() {
            let delta = event.position - pan.last;
            pan.last = event.position;
            let limit_x = (self.layout.available_width * self.zoom * 0.6).max(180.);
            let limit_y = (self.layout.available_height * self.zoom * 0.6).max(180.);
            self.canvas_pan = panned_placement(
                self.canvas_pan,
                Placement {
                    x: f32::from(delta.x),
                    y: f32::from(delta.y),
                },
                limit_x,
                limit_y,
            );
            cx.notify();
            return;
        }
        if self.dragging.is_none() { return; }
        if event.pressed_button != Some(MouseButton::Left) {
            self.end_drag(cx);
            return;
        }
        let scale = f32::from(px(1.)) * self.zoom * interface_font_ratio(cx.theme().font_size);
        let animate = Self::motion_enabled(cx);
        let drag = self.dragging.as_mut().unwrap();
        let distance = event.position - drag.start;
        if !drag.moved && f32::from(distance.x).hypot(f32::from(distance.y)) < 4. { return; }
        drag.moved = true;
        let delta = event.position - drag.last;
        let dx = f32::from(delta.x) / scale;
        let dy = f32::from(delta.y) / scale;
        let ticks = (drag.sampled_at.elapsed().as_secs_f32() / 0.016).max(1.);
        let speed = (dx / ticks).hypot(dy / ticks);
        let cap = if speed > 18. { 18. / speed } else { 1. };
        drag.velocity = Placement { x: dx / ticks * cap, y: dy / ticks * cap };
        drag.last = event.position;
        drag.sampled_at = std::time::Instant::now();
        match &drag.target {
            DragTarget::Project(key) => {
                let project = self.layout.projects.get_mut(key).unwrap();
                let bounds = project.bounds.unwrap();
                project.target.x = (project.target.x + dx).max(-bounds.x);
                project.target.y = (project.target.y + dy).max(-bounds.y);
                if !animate { for follower in project.followers.values_mut() { *follower = project.target; } }
            }
            DragTarget::Card(key, id) => {
                let project = self.layout.projects.get_mut(key).unwrap();
                let rect = project.cards[id];
                let bounds = project.bounds.unwrap();
                let follower = project.follower(id);
                let offset = project.card_motion.entry(id.clone()).or_default();
                offset.x = (offset.x + dx).max(-bounds.x - follower.x - rect.x);
                offset.y = (offset.y + dy).max(-bounds.y - follower.y - rect.y);
            }
        }
        self.start_motion(cx);
        cx.notify();
    }
    fn step_motion(&mut self, animate: bool) -> bool {
        let mut moving = false;
        for p in self.layout.projects.values_mut() {
            let bounds = p.bounds.unwrap();
            if animate {
                moving |= p.target.coast(-bounds.x, -bounds.y);

            } else {
                p.target.stop();
            }
            let springs: Vec<_> = p.cards.keys().map(|id| (id.clone(), p.spring_strength(id))).collect();
            for (id, stiffness) in springs {
                let rect = p.card_rect(&id);
                let follower = p.followers.entry(id).or_insert(p.target);
                if animate {
                    moving |= follower.spring(Placement { x: p.target.x, y: p.target.y }, stiffness);
                    if follower.x < -bounds.x - rect.x { follower.x = -bounds.x - rect.x; follower.vx = 0.; }
                    if follower.y < -bounds.y - rect.y { follower.y = -bounds.y - rect.y; follower.vy = 0.; }
                } else { *follower = p.target; }
            }
            for (id, offset) in &mut p.card_motion {
                if animate {
                    let rect = p.cards[id];
                    let follower = p.followers.get(id).copied().unwrap_or(p.target);
                    moving |= offset.coast(-bounds.x - follower.x - rect.x, -bounds.y - follower.y - rect.y);
                } else { offset.stop(); }
            }
        }
        moving
    }
    fn start_motion(&mut self, cx: &mut Context<Self>) {
        if self.motion_task.is_some() { return; }
        self.motion_task = Some(cx.spawn(async move |entity, cx| {
            for _ in 0..240 {
                cx.background_executor().timer(Duration::from_millis(16)).await;
                let moving = entity.update(cx, |overview, cx| {
                    let moving = overview.step_motion(Self::motion_enabled(cx));
                    cx.notify();
                    moving
                }).unwrap_or(false);
                if !moving { break; }
            }
            let _ = entity.update(cx, |overview, _| overview.motion_task = None);
        }));
    }
    fn card(&self, card: &Card, list: bool, cx: &mut Context<Self>) -> AnyElement {
        let quiet = card.state == State::Idle;
        let tone = |color: u32| {
            if !quiet { return color; }
            let color = rgb(color);
            let gray = ((0.2126 * color.r + 0.7152 * color.g + 0.0722 * color.b) * 255.).round() as u32;
            (gray << 16) | (gray << 8) | gray
        };
        let mut p = self.palette;
        p.ink = tone(p.ink); p.muted = tone(p.muted); p.accent = tone(p.accent);
        p.card = tone(p.card); p.soft = tone(p.soft);
        let zoom = if list { 1. } else { self.zoom };
        let ratio = interface_font_ratio(cx.theme().font_size);
        let scaled = |n: f32| px(n * zoom * ratio);
        let small = scaled(10.);
        let color = tone(card.state.color(p));
        let mut background = design::surface_color(p.card);
        // Tint the selected theme surface while retaining configured translucency.
        background.r *= 0.96;
        background.g *= 0.96;
        background.b *= 0.96;
        let task_id = card.task_id.clone();
        let done = card
            .steps
            .iter()
            .filter(|state| **state == State::Done)
            .count();
        let outline = match card.state {
            State::Idle => rgb(p.hairline(0)),
            State::Done => rgb(p.hairline(1)),
            State::Working => rgb(p.glow()),
            State::NeedsYou | State::Failed => rgb(color).opacity(0.42),
        };
        let mut progress = div()
            .flex()
            .gap(scaled(3.))
            .w_full()
            .h(scaled(4.))
            .flex_shrink_0();
        // Coalesce very large plans while preserving the true count in the label.
        if !card.steps.is_empty() {
            for states in card.steps.chunks((card.steps.len() + 15) / 16) {
                let state = if states.iter().all(|s| *s == State::Done) {
                    State::Done
                } else if states.contains(&State::Working) {
                    State::Working
                } else if states.contains(&State::Failed) {
                    State::Failed
                } else if states.contains(&State::NeedsYou) {
                    State::NeedsYou
                } else {
                    State::Idle
                };
                let fill = if state == State::Idle {
                    rgb(p.tint()).opacity(0.22)
                } else {
                    rgb(tone(state.color(p)))
                };
                progress = progress.child(div().flex_1().h_full().rounded_full().bg(fill));
            }
        }
        let mut metadata = if card.steps.is_empty() {
            "No work plan".into()
        } else {
            format!("{done} of {} steps", card.steps.len())
        };
        if card.children > 0 {
            metadata.push_str(&format!(
                " · {} subagent{}",
                card.children,
                if card.children == 1 { "" } else { "s" }
            ));
        }
        let identity = crate::branding::provider_for_ink(&card.provider, &card.model, p.ink)
            .map(|image| img(image).grayscale(quiet).size(scaled(14.)).into_any_element())
            .unwrap_or_else(|| {
                gpui_kit::component::Icon::new(gpui_kit::assets::IconName::MessageSquare)
                    .size(scaled(14.))
                    .into_any_element()
            });
        let title = div()
            .h(scaled(14.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(scaled(6.))
            .min_w_0()
            .w_full()
            .child(
                div()
                    .size(scaled(14.))
                    .flex_shrink_0()
                    .rounded(scaled(5.))
                    .bg(rgb(p.accent).opacity(0.18))
                    .text_size(small)
                    .text_color(rgb(p.accent))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(identity),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_weight(FontWeight::MEDIUM)
                    .child(card.agent.clone()),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap(scaled(4.))
                    .text_size(small)
                    .text_color(rgb(color))
                    .child(if card.state == State::Working {
                        div().debug_selector({let id = card.task_id.clone(); move || format!("activity-card-spinner-{id}").into()})
                            .child(working_indicator(scaled(11.), color, !cx.reduce_motion() && crate::native_motion::motion_allowed())).into_any_element()
                    } else { div().size(scaled(4.)).rounded_full().bg(rgb(color)).into_any_element() })
                    .child(card.state.label()),
            );
        let card_title = card.title.clone();
        let title_selector = format!("activity-card-title-{}", card.task_id);
        let footer_selector = format!("activity-card-footer-{}", card.task_id);
        let mut body = div()
            .id(SharedString::from(format!(
                "activity-card-{}",
                card.task_id
            )))
            .debug_selector(move || format!("activity-card-{task_id}").into())
            .flex()
            .flex_col()
            .gap(scaled(5.))
            .min_w_0()
            .relative()
            .overflow_hidden()
            .px(scaled(12.))
            .py(scaled(10.))
            .rounded(scaled(12.))
            .border_1()
            .border_color(outline)
            .bg(background)
            .when(quiet, |body| body.opacity(0.72))
            .when(self.filter == Filter::Working && matches!(card.state, State::Idle | State::Done),
                |body| body.opacity(if quiet { 0.72 * 0.6 } else { 0.6 }))
            .shadow({
                let mut shadows = vec![BoxShadow::new(px(0.), scaled(6.), rgb(0x000000).opacity(if p.dark { 0.28 } else { 0.12 }).into())
                    .blur_radius(scaled(18.))];
                if card.state == State::Working {
                    shadows.push(BoxShadow::new(px(0.), px(0.), rgb(color).opacity(0.08).into())
                        .blur_radius(scaled(24.)));
                }
                shadows
            })
            .text_color(rgb(p.ink))
            .text_size(scaled(11.))
            .line_height(scaled(12.))
            .cursor_pointer()
            .hover(move |style| {
                style
                    .border_color(rgb(p.accent))
                    .bg(design::surface_color(p.soft))
            })
            .when(card.state == State::Working, |body| body.children(
                crate::activity_effects::for_task(&card.task_id, "activity-overview-card",
                    crate::activity_sparkles::SparkleLayout::Compact, false, cx)))
            .child(title)
            .child(progress)
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .truncate()
                    .text_size(small)
                    .h(scaled(12.))
                    .flex_shrink_0()
                    .debug_selector(move || title_selector.clone().into())
                    .child(card_title),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(scaled(4.))
                    .text_size(small)
                    .text_color(rgb(p.muted))
                    .child(div().flex_1().min_w_0().truncate().child(metadata))
                    .when(card.attention > 0, |el| {
                        el.child(div().text_color(rgb(color)).child("Reply →"))
                    }),
            )
            .child(
                div()
                    .text_size(small)
                    .text_color(rgb(p.muted))
                    .h(scaled(12.))
                    .flex_shrink_0()
                    .debug_selector(move || footer_selector.clone().into())
                    .child(updated_label(
                        card.updated_at,
                        chrono::Utc::now().timestamp_millis(),
                    )),
            );
        let task_id = card.task_id.clone();
        if !list {
            let group = card.group_id(self.grouping).to_owned();
            let id = card.task_id.clone();
            body = body.on_mouse_down(MouseButton::Left, cx.listener(move |overview, event: &MouseDownEvent, _, cx| {
                overview.begin_drag(DragTarget::Card(group.clone(), id.clone()), event.position, cx);
                cx.stop_propagation();
            }));
        }
        body = body.on_click(cx.listener(move |overview, _, _, cx| {
            if overview.suppress_card_click.take().as_deref() == Some(task_id.as_str()) { return; }
            cx.emit(ActivityOverviewEvent::OpenTask {
                task_id: task_id.clone(),
            });
        }));
        if list {
            body.w_full().into_any_element()
        } else {
            body.w(scaled(CARD_W)).h(scaled(CARD_H)).into_any_element()
        }
    }
    fn communication_layer(&self, endpoints: HashMap<String, MapRect>, scale: f32, viewport: MapRect, cx: &mut Context<Self>) -> AnyElement {
        let hubs: Vec<_> = self.layout.projects.values().filter_map(|project| {
            let mut hub = project.hub?; let bounds = project.bounds?;
            hub.x += bounds.x + project.target.x; hub.y += bounds.y + project.target.y;
            Some(hub)
        }).collect();
        let links: Vec<_> = self.communications.borrow().links.values().filter_map(|link| {
            let a=*endpoints.get(&link.pair.0)?; let b=*endpoints.get(&link.pair.1)?;
            if !a.overlaps(viewport, 0.) || !b.overlaps(viewport, 0.) { return None; }
            let mut obstacles=hubs.clone();
            obstacles.extend(endpoints.iter().filter(|(id,_)| **id!=link.pair.0 && **id!=link.pair.1).map(|(_,r)| *r));
            let curves=communications::Curve::directed(a,b,&obstacles,link.forward,link.reverse);
            Some((link.clone(),a,b,curves))
        }).collect();
        let state = self.communications.clone();
        // Match recorded agent exchanges in the chat stream (agent_exchange.rs).
        let color = self.palette.agent();
        let animate = !links.is_empty() && Self::motion_enabled(cx);
        let phase = Rc::new(Cell::new(0.));
        let paint_phase = phase.clone();
        let layer = div().id("activity-communications")
            .debug_selector(|| "activity-communications".into()).absolute().size_full()
            .child(canvas(|_, _, _| (), move |bounds, _, window, _| {
                let clip = window.content_mask().bounds;
                let scaled = |p: (f32, f32)| point(bounds.origin.x + px(p.0 * scale), bounds.origin.y + px(p.1 * scale));
                let visible = |r: MapRect| {
                    let origin = scaled((r.x, r.y));
                    origin.x < clip.right() && origin.y < clip.bottom()
                        && origin.x + px(r.w * scale) > clip.left() && origin.y + px(r.h * scale) > clip.top()
                };
                for (link, a, b, curves) in &links {
                    if !visible(*a) || !visible(*b) { continue; }
                    if window.is_window_active() { state.borrow_mut().painted.insert(link.pair.clone()); }
                    let alpha = link.opacity();
                    for curve in curves {
                        for dot in curve.dots(paint_phase.get()) {
                            let bounds = Bounds::new(scaled((dot.0 - 1., dot.1 - 1.)), size(px(2. * scale), px(2. * scale)));
                            window.paint_quad(fill(bounds, rgb(color).opacity(alpha)).corner_radii(px(scale)));
                        }
                    }
                }
            }).absolute().size_full());
        if animate {
            layer.with_animation("activity-communication-flow", Animation::new(Duration::from_millis(180))
                .repeat_synced().with_max_fps(30.), move |layer, progress| { phase.set(progress * 6.); layer }).into_any_element()
        } else { layer.into_any_element() }
    }
    fn cluster(
        &self,
        index: usize,
        name: String,
        cards: &[Card],
        viewport: MapRect,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = self.palette;
        let zoom = self.zoom;
        let ratio = interface_font_ratio(cx.theme().font_size);
        let scaled = move |value: f32| px(value * zoom * ratio);
        let key = cards[0].group_id(self.grouping).to_owned();
        let project = (self.grouping == NativeActivityGrouping::Project)
            .then(|| {
                rows(&self.snapshot, "projects")
                    .iter()
                    .find(|project| text(project, "id") == key)
            })
            .flatten();
        let project_color = design::parse_color(
            project.map_or("", |project| text(project, "color")),
            p.accent,
        );
        let project_icon = if self.grouping == NativeActivityGrouping::Workspace {
            gpui_kit::assets::IconName::Folder
        } else {
            crate::shell::project_icon(project.map_or("", |project| text(project, "icon")))
        };
        let retained = &self.layout.projects[&key];
        let bounds = retained.bounds.unwrap();
        let (width, height) = (bounds.w, bounds.h);
        let mut placements = vec![retained.hub.unwrap()];
        placements.extend(cards.iter().map(|c| {
            let mut r = retained.card_rect(&c.task_id);
            let follower = retained.follower(&c.task_id);
            r.x += follower.x - retained.target.x;
            r.y += follower.y - retained.target.y;
            r
        }));
        let hub = Placement {
            x: placements[0].x + HUB_W / 2.,
            y: placements[0].y + HUB_H / 2.,
        };
        let edges: Vec<_> = cards
            .iter()
            .enumerate()
            .filter_map(|(index, card)| {
                let placement = placements[index + 1];
                let end_x = placement.x + CARD_W / 2.;
                let end_y = placement.y + CARD_H / 2.;
                let line = MapRect { x: bounds.x + retained.target.x + hub.x.min(end_x),
                    y: bounds.y + retained.target.y + hub.y.min(end_y),
                    w: (hub.x - end_x).abs().max(1.), h: (hub.y - end_y).abs().max(1.) };
                line.overlaps(viewport, 0.).then_some((placement, card.state))
            }).collect();
        let animate_links = edges.iter().any(|(_, state)| *state == State::Working);
        let has_working = cards.iter().any(|card| card.state == State::Working);
        let motion = !cx.reduce_motion() && crate::native_motion::motion_allowed();
        let phase = Rc::new(Cell::new(0.));
        let paint_phase = phase.clone();
        let links = div().id(SharedString::from(format!("activity-links-{key}")))
            .debug_selector(|| "activity-project-links".into()).absolute().size_full()
            .child(canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| {
                        for (placement, state) in &edges {
                            let mut path = PathBuilder::stroke(scaled(0.8));
                            let start = point(
                                bounds.origin.x + scaled(hub.x),
                                bounds.origin.y + scaled(hub.y),
                            );
                            let end = point(
                                bounds.origin.x + scaled(placement.x + CARD_W / 2.),
                                bounds.origin.y + scaled(placement.y + CARD_H / 2.),
                            );
                            if *state == State::Working {
                                // Moving dash phase travels from the project to its working agent.
                                for segment in -1..20 {
                                    let a = ((segment as f32 + paint_phase.get()) / 20.).max(0.);
                                    let b = ((segment as f32 + paint_phase.get() + 0.55) / 20.).clamp(0., 1.);
                                    if b <= a { continue; }
                                    path.move_to(point(
                                        start.x + (end.x - start.x) * a,
                                        start.y + (end.y - start.y) * a,
                                    ));
                                    path.line_to(point(
                                        start.x + (end.x - start.x) * b,
                                        start.y + (end.y - start.y) * b,
                                    ));
                                }
                            } else {
                                path.move_to(start);
                                path.line_to(end);
                            }
                            if let Ok(path) = path.build() {
                                window.paint_path(
                                    path,
                                    rgb(state.color(p)).opacity(if *state == State::Idle {
                                        0.18
                                    } else {
                                        0.34
                                    }),
                                );
                            }
                        }
                    },
                )
                .absolute()
                .size_full());
        let links = if animate_links && motion {
            links.with_animation("activity-working-links", Animation::new(Duration::from_millis(900))
                .repeat_synced().with_max_fps(30.), move |links, progress| { phase.set(progress); links }).into_any_element()
        } else { links.into_any_element() };
        let mut cluster = div()
            .id(SharedString::from(format!("activity-project-{index}")))
            .relative().w(scaled(width)).h(scaled(height)).flex_shrink_0().child(links);
        for (index, card) in cards.iter().enumerate() {
            let placement = placements[index + 1];
            let rect = MapRect { x: bounds.x + retained.target.x + placement.x,
                y: bounds.y + retained.target.y + placement.y, w: CARD_W, h: CARD_H };
            let dragged = self.dragging.as_ref().is_some_and(|drag| matches!(&drag.target,
                DragTarget::Card(_, id) if id == &card.task_id));
            if !dragged && !rect.overlaps(viewport, 0.) { continue; }
            cluster = cluster.child(
                div()
                    .absolute()
                    .left(scaled(placement.x))
                    .top(scaled(placement.y))
                    .child(self.card(card, false, cx)),
            );
        }
        let hub_rect = MapRect { x: bounds.x + retained.target.x + hub.x - HUB_W / 2.,
            y: bounds.y + retained.target.y + hub.y - HUB_H / 2., w: HUB_W, h: HUB_H };
        if !hub_rect.overlaps(viewport, 0.) { return cluster.into_any_element(); }
        let working = cards.iter().filter(|c| c.state == State::Working).count();
        let needs = cards.iter().filter(|c| c.state == State::NeedsYou).count();
        let failed = cards.iter().filter(|c| c.state == State::Failed).count();
        let summary = [
            (working, "working"),
            (needs, "needs you"),
            (failed, "failed"),
            (
                cards
                    .iter()
                    .filter(|c| matches!(c.state, State::Idle | State::Done))
                    .count(),
                "quiet",
            ),
        ]
        .into_iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, label)| format!("{n} {label}"))
        .collect::<Vec<_>>()
        .join(" · ");
        cluster
            .child(
                div()
                    .id(SharedString::from(format!("activity-hub-{key}")))
                    .debug_selector({
                        let key = key.clone();
                        move || format!("activity-hub-{key}")
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |overview, event: &MouseDownEvent, _, cx| {
                            overview.begin_drag(DragTarget::Project(key.clone()), event.position, cx);
                            cx.stop_propagation();
                        }),
                    )
                    .absolute()
                    .left(scaled(hub.x - HUB_W / 2.))
                    .top(scaled(hub.y - HUB_H / 2.))
                    .w(scaled(HUB_W))
                    .min_h(scaled(HUB_H))
                    .rounded(scaled(12.))
                    .p(scaled(8.))
                    .border_1()
                    .border_color(rgb(project_color).opacity(0.40))
                    .bg(rgb(design::mix_rgb(p.paper, project_color, 0.10)))
                    .shadow(vec![BoxShadow::new(px(0.), scaled(3.), rgb(0x000000).opacity(if p.dark { 0.24 } else { 0.10 }).into())
                        .blur_radius(scaled(10.))])
                    .flex().flex_col().items_start().justify_center()
                    .child(
                        div()
                            .flex().items_center().gap(scaled(5.)).min_w_0().w_full()
                            .font_weight(FontWeight::MEDIUM)
                            .text_size(cx.theme().font_size * zoom)
                            .child(div().flex_shrink_0()
                                .debug_selector(|| "activity-project-icon".into())
                                .child(gpui_kit::component::Icon::new(project_icon).with_size(scaled(16.))))
                            .child(div().min_w_0().truncate().child(name))
                            .when(has_working, |title| title.child(div()
                                .debug_selector(|| "activity-project-spinner".into())
                                .child(working_indicator(scaled(12.), project_color, motion)))),
                    )
                    .child(
                        div()
                            .text_size((cx.theme().font_size - px(2.)) * zoom)
                            .text_color(rgb(p.muted))
                            .child(summary),
                    ),
            )
            .into_any_element()
    }
}
impl Focusable for ActivityOverview {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for ActivityOverview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette;
        let font = cx.theme().font_size;
        let list_metrics = (font, design::interface_scale(), cx.theme().font_family.clone());
        if self.list_metrics.as_ref() != Some(&list_metrics) {
            self.list_state.remeasure();
            self.list_metrics = Some(list_metrics);
        }
        let query = self.search.read(cx).value().trim().to_lowercase();
        let membership_changed = self.prepared_view.as_ref().is_none_or(|view| {
            view.revision != self.model_revision || view.filter != self.filter
                || view.query != query || view.grouping != self.grouping
        });
        if membership_changed {
            let cards = self.model.visible(self.filter, &query, self.grouping);
            let groups = grouped(cards.clone(), self.grouping);
            let mut list_rows = Vec::new();
            let mut row_keys = Vec::new();
            for (group, (_, cards)) in groups.iter().enumerate() {
                list_rows.push((group, None));
                row_keys.push(format!("group:{}", cards[0].group_id(self.grouping)));
                list_rows.extend((0..cards.len()).map(|card| (group, Some(card))));
                row_keys.extend(cards.iter().map(|card| format!("card:{}", card.task_id)));
            }
            let old_offset = self.list_state.logical_scroll_top();
            let old_view = self.prepared_view.as_ref().filter(|view| {
                view.filter == self.filter && view.query == query && view.grouping == self.grouping
            });
            if old_view.is_some_and(|view| view.row_keys == row_keys) {
                self.list_state.remeasure();
            } else {
                let anchor = old_view.and_then(|view| view.row_keys.get(old_offset.item_ix))
                    .and_then(|key| row_keys.iter().position(|candidate| candidate == key));
                self.list_state.reset(list_rows.len());
                if let Some(item_ix) = anchor {
                    self.list_state.scroll_to(ListOffset { item_ix, offset_in_item: old_offset.offset_in_item });
                }
            }
            self.view_revision = self.view_revision.wrapping_add(1);
            self.prepared_view = Some(PreparedActivityView {
                revision: self.model_revision, filter: self.filter, query, grouping: self.grouping,
                cards: Rc::new(cards), groups: Rc::new(groups), list_rows: Rc::new(list_rows), row_keys,
            });
        }
        let view = self.prepared_view.as_ref().unwrap();
        let visible = view.cards.clone();
        let groups = view.groups.clone();
        let list_rows = view.list_rows.clone();
        let map_scale = self.zoom * interface_font_ratio(font);
        let available_width = ((f32::from(window.viewport_size().width) - 64.) / map_scale).max(CARD_W + 24.);
        let available_height = ((f32::from(window.viewport_size().height) - 190.) / map_scale).max(CARD_H + 24.);
        let layout_input = (self.view_revision, available_width, available_height);
        if self.layout_input != Some(layout_input) || self.dragging.is_some() || self.motion_task.is_some() {
            self.layout.sync_groups(&groups, available_width, available_height, self.grouping);
            self.layout_input = Some(layout_input);
        }
        let group_count = self.model.group_count(self.grouping);
        let group_label = match self.grouping {
            NativeActivityGrouping::Project => "projects",
            NativeActivityGrouping::Workspace => "workspaces",
        };
        let mut filters = div().flex().items_center().gap(px(5.)).flex_wrap();
        for filter in Filter::ALL {
            let selected = self.filter == filter;
            filters = filters.child(
                Button::new(SharedString::from(format!(
                    "activity-filter-{}",
                    filter.label()
                )))
                .debug_selector(move || format!("activity-filter-{}", filter.label()).into())
                .compact()
                .ghost()
                .label(format!("{} {}", filter.label(), self.model.count(filter)))
                .text_size(font - px(1.))
                .when(selected, |el| el.bg(design::surface_color(p.soft)))
                .on_click(cx.listener(move |overview, _, _, cx| {
                    overview.filter = filter;
                    cx.notify();
                })),
            );
        }
        let controls = div()
            .id("activity-controls")
            .debug_selector(|| "activity-controls".into())
            .flex()
            .items_center()
            .gap(px(5.))
            .flex_wrap()
            .child(div().w(px(205.)).child(Input::new(&self.search)))
            .child(
                Button::new("activity-group-projects")
                    .debug_selector(|| "activity-group-projects".into())
                    .compact()
                    .ghost()
                    .label("Projects")
                    .text_size(font - px(1.))
                    .when(self.grouping == NativeActivityGrouping::Project, |el| {
                        el.bg(design::surface_color(p.soft))
                    })
                    .on_click(cx.listener(|overview, _, _, cx| {
                        overview.set_grouping(NativeActivityGrouping::Project, cx);
                    })),
            )
            .child(
                Button::new("activity-group-workspaces")
                    .debug_selector(|| "activity-group-workspaces".into())
                    .compact()
                    .ghost()
                    .label("Workspaces")
                    .text_size(font - px(1.))
                    .when(self.grouping == NativeActivityGrouping::Workspace, |el| {
                        el.bg(design::surface_color(p.soft))
                    })
                    .on_click(cx.listener(|overview, _, _, cx| {
                        overview.set_grouping(NativeActivityGrouping::Workspace, cx);
                    })),
            )
            .child(
                Button::new("activity-clusters")
                    .debug_selector(|| "activity-clusters".into())
                    .compact()
                    .ghost()
                    .label("Clusters")
                    .text_size(font - px(1.))
                    .when(!self.list, |el| el.bg(design::surface_color(p.soft)))
                    .on_click(cx.listener(|overview, _, _, cx| {
                        overview.list = false;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("activity-list")
                    .debug_selector(|| "activity-list".into())
                    .compact()
                    .ghost()
                    .label("List")
                    .text_size(font - px(1.))
                    .when(self.list, |el| el.bg(design::surface_color(p.soft)))
                    .on_click(cx.listener(|overview, _, _, cx| {
                        overview.list = true;
                        cx.notify();
                    })),
            )
            .when(!self.list, |el| {
                el.child(
                    Button::new("activity-zoom-out")
                        .compact()
                        .ghost()
                        .label("−")
                        .on_click(cx.listener(|overview, _, _, cx| {
                            overview.zoom = clamped_zoom(overview.zoom - 0.1);
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("activity-zoom-reset")
                        .compact()
                        .ghost()
                        .label(format!("{:.0}%", self.zoom * 100.))
                        .text_size(font - px(2.))
                        .on_click(cx.listener(|overview, _, _, cx| {
                            overview.zoom = 1.;
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("activity-zoom-in")
                        .compact()
                        .ghost()
                        .label("+")
                        .on_click(cx.listener(|overview, _, _, cx| {
                            overview.zoom = clamped_zoom(overview.zoom + 0.1);
                            cx.notify();
                        })),
                )
            });
        let header = div()
            .id("activity-header")
            .debug_selector(|| "activity-header".into())
            .flex()
            .items_center()
            .flex_wrap()
            .gap(px(10.))
            .min_h(px(56.))
            .px(px(20.))
            .py(px(10.))
            .border_b_1()
            .border_color(rgb(p.hairline(0)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(div().font_weight(FontWeight::SEMIBOLD).text_size(px(15.)).child("Activity"))
                    .child(
                        div()
                            .font_family(design::FONT_MONO)
                            .text_size(px(11.))
                            .text_color(rgb(p.text(2)))
                            .child(format!(
                                "{} chats · {group_count} {group_label} · last 12h",
                                self.model.cards.len(),
                            )),
                    ),
            )
            .child(filters)
            .child(div().flex_1())
            .child(controls);
        let mut content = div()
            .id("activity-overview-scroll")
            .debug_selector(|| "activity-overview-scroll".into())
            .flex_1()
            .min_h_0()
            .overflow_scroll()
            .p(px(12.));
        if visible.is_empty() {
            content = content.child(div().p(px(30.)).text_color(rgb(p.muted)).child(
                if self.model.cards.is_empty() {
                    "No agent activity in the last 12 hours."
                } else {
                    "No chats match this search and filter."
                },
            ));
        } else if self.list {
            let owner = cx.weak_entity();
            let rows = gpui_kit::list(self.list_state.clone(), move |index, _, cx| {
                let (group, card) = list_rows[index];
                let (name, cards) = &groups[group];
                match card {
                    None => div().font_weight(FontWeight::MEDIUM).px(px(4.))
                        .pt(px(if group == 0 { 0. } else { 15. })).pb(px(6.))
                        .child(name.clone()).into_any_element(),
                    Some(index) => owner.update(cx, |overview, cx| {
                        div().w_full().pb(px(6.)).child(overview.card(&cards[index], true, cx)).into_any_element()
                    }).unwrap_or_else(|_| div().into_any_element()),
                }
            }).size_full();
            content = content.overflow_hidden().child(rows);
        } else {
            let mut endpoints = HashMap::new();
            for card in visible.iter() {
                let project = &self.layout.projects[card.group_id(self.grouping)];
                let bounds = project.bounds.unwrap();
                let mut rect = project.card_rect(&card.task_id);
                let follower = project.follower(&card.task_id);
                rect.x += bounds.x + follower.x;
                rect.y += bounds.y + follower.y;
                endpoints.insert(card.task_id.clone(), rect);
            }
            let scale = map_scale;
            let width = self.layout.canvas_width.max(self
                .layout
                .projects
                .values()
                .filter_map(|p| p.bounds.map(|r| r.x + r.w + p.target.x.max(0.)))
                .fold(0., f32::max)
                + 24.);
            let height = self.layout.canvas_height.max(self
                .layout
                .projects
                .values()
                .filter_map(|p| p.bounds.map(|r| r.y + r.h + p.target.y.max(0.)))
                .fold(0., f32::max)
                + 24.);
            // Use the previous frame's measured graph origin and clip; account for the
            // current pan immediately. Overscan covers shadows and geometry changes.
            let graph_width = px(width * scale);
            let graph_height = px(height * scale);
            let viewport = self.painted_map.get().filter(|painted| (painted.scale - scale).abs() < 0.001);
            let (origin, clip) = viewport.map(|painted| (
                painted.origin + point(gpui_kit::px(self.canvas_pan.x - painted.pan.x), gpui_kit::px(self.canvas_pan.y - painted.pan.y)),
                painted.clip,
            )).unwrap_or_else(|| (
                point((window.viewport_size().width - graph_width) / 2. + gpui_kit::px(self.canvas_pan.x),
                    (window.viewport_size().height - graph_height) / 2. + gpui_kit::px(self.canvas_pan.y)),
                Bounds::new(point(gpui_kit::px(0.), gpui_kit::px(0.)), window.viewport_size()),
            ));
            let unit = px(scale).as_f32();
            let overscan = 256. / unit;
            let map_viewport = MapRect {
                x: (clip.origin.x - origin.x).as_f32() / unit - overscan,
                y: (clip.origin.y - origin.y).as_f32() / unit - overscan,
                w: clip.size.width.as_f32() / unit + overscan * 2.,
                h: clip.size.height.as_f32() / unit + overscan * 2.,
            };
            let geometry = self.painted_map.clone();
            let pan = self.canvas_pan;
            let owner = cx.weak_entity();
            let mut graph = div()
                .id("activity-responsive-map")
                .debug_selector(|| "activity-responsive-map".into())
                .relative()
                .left(gpui_kit::px(self.canvas_pan.x))
                .top(gpui_kit::px(self.canvas_pan.y))
                .w(px(width * scale))
                .h(px(height * scale))
                .flex_shrink_0()
                .child(canvas(move |bounds, window, cx| {
                    let clip = window.content_mask().bounds;
                    let old = geometry.get();
                    let changed = old.is_none_or(|old| old.origin != bounds.origin || old.clip != clip || old.scale != scale);
                    geometry.set(Some(PaintedMapViewport { origin: bounds.origin, clip, pan, scale }));
                    if changed {
                        let owner = owner.clone();
                        cx.defer(move |cx| { let _ = owner.update(cx, |_, cx| cx.notify()); });
                    }
                }, |_, _, _, _| ()).absolute().inset_0())
                .child(self.communication_layer(endpoints, scale, map_viewport, cx));
            for (index, (name, cards)) in groups.iter().enumerate() {
                let project = &self.layout.projects[cards[0].group_id(self.grouping)];
                let position = project.bounds.unwrap();
                graph = graph.child(
                    div()
                        .absolute()
                        .left(px((position.x + project.target.x) * scale))
                        .top(px((position.y + project.target.y) * scale))
                        .child(self.cluster(index, name.clone(), cards, map_viewport, cx)),
                );
            }
            let cursor = if self.canvas_pan_drag.is_some() {
                CursorStyle::ClosedHand
            } else {
                CursorStyle::OpenHand
            };
            content = content
                .overflow_hidden()
                .cursor(cursor)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|overview, event: &MouseDownEvent, _, cx| {
                        overview.begin_canvas_pan(event.position, cx);
                        cx.stop_propagation();
                    }),
                )
                .child(
                div()
                    .id("activity-pan-sheet")
                    .debug_selector(|| "activity-pan-sheet".into())
                    .min_w_full()
                    .min_h_full()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor(cursor)
                    .child(graph),
            );
        }
        let mut legend = div()
            .id("activity-footer")
            .debug_selector(|| "activity-footer".into())
            .flex()
            .items_center()
            .flex_wrap()
            .gap(px(12.))
            .px(px(15.))
            .py(px(8.))
            .border_t_1()
            .border_color(design::surface_color(p.line))
            .text_size(font - px(2.))
            .text_color(rgb(p.muted));
        for state in [
            State::Working,
            State::NeedsYou,
            State::Failed,
            State::Done,
            State::Idle,
        ] {
            legend = legend.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .child(div().size(px(5.)).rounded_full().bg(rgb(state.color(p))))
                    .child(state.label()),
            );
        }
        legend = legend
            .child(div().child("Dashed line = working · Dotted blue line = communication"))
            .child(div().flex_1())
            .child(div().child("Click a card to open Watch"));
        div()
            .id("activity-overview")
            .child(
                canvas(|_, _, _| (), {
                    let entity = cx.entity().downgrade();
                    move |_, _, window, _| {
                        let moving = entity.clone();
                        window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                            if phase.capture() {
                                let _ = moving
                                    .update(cx, |overview, cx| overview.drag_mouse_move(event, cx));
                            }
                        });
                        let released = entity.clone();
                        window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                            if phase.capture() && event.button == MouseButton::Left {
                                let _ = released.update(cx, |overview, cx| overview.end_pointer_drag(cx));
                            }
                        });
                    }
                })
                .absolute()
                .size_full(),
            )
            .debug_selector(|| "activity-overview".into())
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .size_full()
            .min_w_0()
            .min_h_0()
            .text_color(rgb(p.ink))
            .font_family(cx.theme().font_family.clone())
            .text_size(font)
            .on_key_down(cx.listener(|overview, event: &KeyDownEvent, window, cx| {
                let search_focused = overview.search.read(cx).focus_handle(cx).is_focused(window);
                if event.keystroke.key == "/"
                    && !search_focused
                    && !event.keystroke.modifiers.platform
                {
                    window.focus(&overview.search.read(cx).focus_handle(cx), cx);
                    cx.stop_propagation();
                } else if event.keystroke.key == "escape" && !search_focused {
                    cx.emit(ActivityOverviewEvent::Close);
                    cx.stop_propagation();
                }
            }))
            .child(header)
            .child(div().relative().flex_1().min_h_0().overflow_hidden()
                .child(div().debug_selector(|| "activity-dot-grid".into()).absolute().size_full()
                    .child(canvas(|_, _, _| (), move |bounds, _, window, _| {
                        let spacing = px(28.);
                        let diameter = px(1.);
                        let color = rgb(p.tint()).opacity(0.10);
                        let columns = (f32::from(bounds.size.width) / f32::from(spacing)).ceil() as usize;
                        let rows = (f32::from(bounds.size.height) / f32::from(spacing)).ceil() as usize;
                        for row in 0..rows {
                            for column in 0..columns {
                                window.paint_quad(fill(Bounds::new(
                                    bounds.origin + point(spacing * (column as f32 + 0.5), spacing * (row as f32 + 0.5)),
                                    size(diameter, diameter)), color).corner_radii(diameter / 2.));
                            }
                        }
                    }).absolute().size_full()))
                .child(content.size_full()))
            .child(legend)
    }
}

/// Explicitly opt-in native renderer fixture. No service, provider or saved profile is opened.
#[cfg(all(feature = "visual-export", feature = "performance-profiling"))]
pub(super) fn open_preview_if_requested(cx: &mut App) -> bool {
    use serde_json::json;
    use gpui_kit::component::{Theme, ThemeMode};
    let Ok(mode) = std::env::var("MONITTER_GPUI_ACTIVITY_COMMUNICATION_PREVIEW") else { return false; };
    let light = mode == "light";
    let _ = monitter_gpui_desktop::fonts::register(cx);
    crate::native_motion::initialize_for_ui_fixture(cx);
    Theme::change(if light { ThemeMode::Light } else { ThemeMode::Dark }, None, cx);
    Theme::update(cx, |theme| { theme.font_family = "IBM Plex Sans".into(); theme.font_size = px(14.); });
    let (handle, _) = gpui_kit::open_window(WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None,size(px(1160.),px(780.)),cx))),
        titlebar: Some(TitlebarOptions {title: Some("Monitter · Synthetic communication preview".into()),..Default::default()}),
        ..Default::default()
    },cx,|window,cx| {
        window.on_window_should_close(cx,|_,cx| {cx.quit();true});
        cx.new(|cx| {
            let mut view = ActivityOverview::new(window,cx);
            let now = chrono::Utc::now().timestamp_millis();
            let snapshot = Rc::new(json!({
                "settings":{"theme":if light {"light"} else {"dark"}},
                "projects":[{"id":"p","name":"Synthetic team","icon":"rocket","color":"#7c5cbf"}],
                "agents":[{"id":"one","name":"Sender","provider":"codex"},{"id":"two","name":"Recipient","provider":"codex"}],
                "tasks":[{"id":"a","title":"Share implementation details","agentId":"one","projectId":"p","status":"running","updatedAt":now},
                    {"id":"b","title":"Review and reply","agentId":"two","projectId":"p","status":"running","updatedAt":now},
                    {"id":"c","title":"Independent task","agentId":"one","projectId":"p","status":"completed","updatedAt":now}],
                "collaborations":[{"id":"synthetic-send","kind":"message","status":"completed","fromTaskId":"a","toTaskId":"b","createdAt":now},
                    {"id":"synthetic-reply","kind":"message","status":"completed","fromTaskId":"b","toTaskId":"a","createdAt":now}]
            }));
            view.palette = design::palette(&snapshot["settings"],if light {WindowAppearance::Light} else {WindowAppearance::Dark});
            view.set_snapshot(snapshot,view.palette,cx);
            view
        })
    }).expect("isolated communication preview");
    crate::visual_export::install(cx);
    crate::visual_export::schedule(handle,cx);
    cx.activate(true);
    true
}

#[cfg(test)]
mod tests {
    use super::{
        CARD_H, CARD_W, HUB_H, HUB_W, MapRect, RetainedProject, Filter, Motion,
        NativeActivityGrouping, OverviewModel, Placement, RetainedLayout, State,
        child_inputs_changed, clamped_zoom, cluster_layout, grouped, interface_font_ratio,
        distributed_positions, organic_positions, panned_placement, px, relevant_changed,
        updated_label,
    };
    use serde_json::json;
    use std::collections::HashSet;
    #[gpui_kit::test]
    fn activity_overview_native_communication_paint_and_card_interaction(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::{AppContext as _, IntoElement, Entity, Render, Window, Context, MouseButton, point};
        use std::time::Duration;
        struct Harness(Entity<super::ActivityOverview>);
        impl Render for Harness {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement { self.0.clone() }
        }
        cx.update(|cx| { gpui_kit::init(cx); crate::native_motion::initialize_for_ui_fixture(cx); });
        let mut retained = None;
        let (_, visual) = cx.add_window_view(|window, cx| {
            let overview = cx.new(|cx| {
                let mut view = super::ActivityOverview::new(window,cx);
                view.set_snapshot(std::rc::Rc::new(json!({
                    "settings":{"theme":"dark"},
                    "projects":[{"id":"p","name":"Monitter"}],
                    "agents":[{"id":"shared","name":"Codex"}],
                    "tasks":[{"id":"a","title":"Sender","agentId":"shared","projectId":"p","status":"running","updatedAt":chrono::Utc::now().timestamp_millis()},
                        {"id":"b","title":"Recipient","agentId":"shared","projectId":"p","status":"running","updatedAt":chrono::Utc::now().timestamp_millis()}],
                    "collaborations":[{"id":"real","kind":"message","status":"queued","fromTaskId":"a","toTaskId":"b"},
                        {"id":"reply","kind":"message","status":"queued","fromTaskId":"b","toTaskId":"a"}]
                })),view.palette,cx);
                assert_eq!(view.model.cards.len(),2,"fixture chats are current and visible");
                view
            });
            retained = Some(overview.clone());
            gpui_kit::component::Root::new(cx.new(|_| Harness(overview)),window,cx)
        });
        let overview = retained.unwrap();
        visual.update(|window,_| window.activate_window());
        visual.simulate_resize(gpui_kit::size(px(1200.),px(850.)));
        visual.run_until_parked();
        visual.update(|window,cx| window.draw(cx).clear(cx));
        assert!(visual.debug_bounds("activity-communications").is_some());
        let dot_size = overview.read_with(visual, |view, cx| {
            use gpui_kit::component::ActiveTheme as _;
            f32::from(px(2. * view.zoom * super::interface_font_ratio(cx.theme().font_size)))
        });
        let agent_color = overview.read_with(visual, |view, _| view.palette.agent());
        let channels = [
            ((agent_color >> 16) & 0xff) as f32 / 255.,
            ((agent_color >> 8) & 0xff) as f32 / 255.,
            (agent_color & 0xff) as f32 / 255.,
        ];
        visual.update(|window, _| {
            let raster_tolerance = 1. / window.scale_factor() + 0.01;
            let dots: Vec<_> = window.painted_quads().into_iter().filter(|quad| {
                let width = quad.bounds.size.width.as_f32() / window.scale_factor();
                let height = quad.bounds.size.height.as_f32() / window.scale_factor();
                if (width - dot_size).abs() >= raster_tolerance || (height - dot_size).abs() >= raster_tolerance {
                    return false;
                }
                quad.background.as_solid().is_some_and(|color| {
                    let ink = gpui_kit::Rgba::from(color);
                    (ink.r - channels[0]).abs() < 0.005 && (ink.g - channels[1]).abs() < 0.005
                        && (ink.b - channels[2]).abs() < 0.005 && ink.a > 0.99
                })
            }).collect();
            assert!(dots.len() > 5, "real communication curve paints theme-derived agent dots");
            for dot in dots {
                let width = dot.bounds.size.width.as_f32() / window.scale_factor();
                let height = dot.bounds.size.height.as_f32() / window.scale_factor();
                assert!((width - dot_size).abs() < raster_tolerance && (height - dot_size).abs() < raster_tolerance,
                    "dots are 2px circles at configured scale without a broad glow stroke: {width}x{height}, expected {dot_size}");
            }
        });
        let agent_positions = |visual: &mut gpui_kit::VisualTestContext| visual.update(|window, _| {
            let raster_tolerance = 1. / window.scale_factor() + 0.01;
            window.painted_quads().into_iter().filter(|quad| {
                let width = quad.bounds.size.width.as_f32() / window.scale_factor();
                let height = quad.bounds.size.height.as_f32() / window.scale_factor();
                (width - dot_size).abs() < raster_tolerance && (height - dot_size).abs() < raster_tolerance
                    && quad.background.as_solid().is_some_and(|color| {
                        let ink = gpui_kit::Rgba::from(color);
                        (ink.r - channels[0]).abs() < 0.005 && (ink.g - channels[1]).abs() < 0.005
                            && (ink.b - channels[2]).abs() < 0.005 && ink.a > 0.99
                    })
            }).map(|quad| (quad.bounds.origin.x.as_f32(), quad.bounds.origin.y.as_f32())).collect::<Vec<_>>()
        });
        let before_flow = agent_positions(visual);
        visual.executor().advance_clock(Duration::from_millis(70));
        visual.run_until_parked();
        visual.update(|window,cx| window.draw(cx).clear(cx));
        assert_ne!(before_flow, agent_positions(visual), "actual agent dots flow between native frames");
        overview.read_with(visual, |view,_| {
            let state = view.communications.borrow();
            assert_eq!(state.links.len(),1);
            assert_eq!(state.painted.len(),1,"both real endpoints painted in foreground");
            let link = state.links.values().next().unwrap();
            assert!(link.forward && link.reverse);
        });
        let sheet = visual.debug_bounds("activity-pan-sheet").unwrap();
        let map_before_pan = visual.debug_bounds("activity-responsive-map").unwrap();
        let pan_start = point(sheet.left() + px(18.), sheet.top() + px(18.));
        let pan_end = pan_start + point(px(60.), px(35.));
        visual.simulate_mouse_down(pan_start, MouseButton::Left, Default::default());
        visual.simulate_mouse_move(pan_end, Some(MouseButton::Left), Default::default());
        visual.simulate_mouse_up(pan_end, MouseButton::Left, Default::default());
        visual.run_until_parked();
        visual.update(|window,cx| window.draw(cx).clear(cx));
        let map_after_pan = visual.debug_bounds("activity-responsive-map").unwrap();
        let rendered_pan = pan_end - pan_start;
        overview.read_with(visual, |view, _| {
            assert_eq!(
                (view.canvas_pan.x, view.canvas_pan.y),
                (f32::from(rendered_pan.x), f32::from(rendered_pan.y)),
            );
            assert!(view.canvas_pan_drag.is_none(), "releasing the hand ends sheet panning");
        });
        assert!((f32::from(map_after_pan.left() - map_before_pan.left()) - f32::from(rendered_pan.x)).abs() < 1.,
            "sheet horizontal displacement follows the hand: before={map_before_pan:?}, after={map_after_pan:?}");
        assert!((f32::from(map_after_pan.top() - map_before_pan.top()) - f32::from(rendered_pan.y)).abs() < 1.,
            "sheet vertical displacement follows the hand: before={map_before_pan:?}, after={map_after_pan:?}");
        // The overlay must leave the actual native card click target usable.
        let opened = std::rc::Rc::new(std::cell::Cell::new(false));
        let received = opened.clone();
        visual.update(|_,cx| { cx.subscribe(&overview,move |_,event: &super::ActivityOverviewEvent,_| {
            if matches!(event, super::ActivityOverviewEvent::OpenTask{task_id} if task_id=="b") { received.set(true); }
        }).detach(); });
        let card_bounds = visual.debug_bounds("activity-card-b").unwrap();
        visual.simulate_click(card_bounds.center(),Default::default());
        visual.run_until_parked();
        assert!(opened.get(),"communication canvas must not intercept chat clicks");
        opened.set(false);
        let hub_before = visual.debug_bounds("activity-hub-p").unwrap();
        let start = visual.debug_bounds("activity-card-b").unwrap().center();
        let end = point(start.x + px(65.), start.y + px(30.));
        visual.simulate_mouse_down(start, MouseButton::Left, Default::default());
        visual.simulate_mouse_move(end, Some(MouseButton::Left), Default::default());
        visual.simulate_mouse_up(end, MouseButton::Left, Default::default());
        visual.run_until_parked();
        visual.update(|window,cx| window.draw(cx).clear(cx));
        let released = visual.debug_bounds("activity-card-b").unwrap();
        assert!(!opened.get(), "dragging a card cannot open Watch");
        for _ in 0..120 {
            visual.executor().advance_clock(Duration::from_millis(16));
            visual.run_until_parked();
            visual.update(|window,cx| window.draw(cx).clear(cx));
        }
        let settled = visual.debug_bounds("activity-card-b").unwrap();
        assert!(settled.left() > released.left() + px(10.), "individual card coasts after release");
        assert!(settled.left() < released.left() + px(150.), "throw distance is bounded");
        assert_eq!(visual.debug_bounds("activity-hub-p").unwrap(), hub_before, "card drag leaves its project fixed");
        overview.read_with(visual, |view,_| assert!(view.motion_task.is_none(), "motion loop stops at rest"));
        visual.simulate_click(settled.center(), Default::default());
        visual.run_until_parked();
        assert!(opened.get(), "normal click after a drag still opens Watch");
        // Motion preference changes stop an in-flight glide on its next frame.
        visual.update(|_,cx| {
            cx.set_reduce_motion(true);
            overview.update(cx, |view,cx| {
                let offset = view.layout.projects.get_mut("p").unwrap().card_motion.get_mut("b").unwrap();
                offset.vx = 18.; offset.vy = 8.;
                view.start_motion(cx);
            });
        });
        visual.executor().advance_clock(Duration::from_millis(16));
        visual.run_until_parked();
        overview.read_with(visual, |view,_| {
            let offset = view.layout.projects["p"].card_motion["b"];
            assert_eq!((offset.vx, offset.vy), (0., 0.));
        });
        visual.update(|_,cx| cx.set_reduce_motion(false));

        let children_before = [visual.debug_bounds("activity-card-a").unwrap(), visual.debug_bounds("activity-card-b").unwrap()];
        let hub_start = visual.debug_bounds("activity-hub-p").unwrap().center();
        let hub_end = hub_start + point(px(95.), px(45.));
        visual.simulate_mouse_down(hub_start, MouseButton::Left, Default::default());
        visual.simulate_mouse_move(hub_end, Some(MouseButton::Left), Default::default());
        visual.executor().advance_clock(Duration::from_millis(16));
        visual.run_until_parked();
        visual.update(|window,cx| window.draw(cx).clear(cx));
        let displacement_a = visual.debug_bounds("activity-card-a").unwrap().left() - children_before[0].left();
        let displacement_b = visual.debug_bounds("activity-card-b").unwrap().left() - children_before[1].left();
        assert!((f32::from(displacement_a - displacement_b)).abs() > 0.1,
            "native siblings move independently during the same project drag");
        visual.simulate_mouse_up(hub_end, MouseButton::Left, Default::default());
        for _ in 0..240 {
            visual.executor().advance_clock(Duration::from_millis(16));
            visual.run_until_parked();
        }
        overview.read_with(visual, |view,_| {
            let project = &view.layout.projects["p"];
            assert!(view.motion_task.is_none(), "project followers stop scheduling at rest");
            for id in ["a", "b"] {
                let follower = project.follower(id);
                assert_eq!((follower.x, follower.y), (project.target.x, project.target.y));
            }
        });

        // A communication publication must repaint even when task/model data is identical.
        overview.update(visual,|view,cx| {
            view.communications.borrow_mut().links.clear();
            view.communications.borrow_mut().painted.clear();
            cx.notify();
        });
        visual.update(|window,cx| window.draw(cx).clear(cx));
        overview.update(visual,|view,cx| {
            let mut snapshot = (*view.snapshot).clone();
            snapshot["collaborations"].as_array_mut().unwrap().push(json!({
                "id":"later-message","kind":"message","status":"queued","fromTaskId":"a","toTaskId":"b"
            }));
            view.set_snapshot(std::rc::Rc::new(snapshot),view.palette,cx);
        });
        visual.run_until_parked();
        visual.update(|window,cx| window.draw(cx).clear(cx));
        overview.read_with(visual,|view,_| assert_eq!(view.communications.borrow().painted.len(),1,
            "new communication paints without an unrelated task update"));
        visual.deactivate_window();
        overview.update(visual,|view,_| view.communications.borrow_mut().painted.clear());
        visual.update(|window,cx| window.draw(cx).clear(cx));
        overview.read_with(visual,|view,_| assert!(view.communications.borrow().painted.is_empty()));
    }
    #[test]
    fn real_tasks_attention_and_status_counts_exclude_archives_and_internal_agents() {
        let model = OverviewModel::from_snapshot(&json!({
            "agents":[{"id":"hidden","internal":true}],
            "projects":[{"id":"p","name":"Monitter"}],
            "tasks":[
                {"id":"a","projectId":"p","status":"running"},
                {"id":"b","status":"queued"}, {"id":"c","status":"failed"},
                {"id":"d","status":"completed"}, {"id":"e","status":"interrupted"},
                {"id":"f","status":"running","archived":true},
                {"id":"g","agentId":"hidden","status":"running"},
                {"id":"b","status":"queued"}
            ],
            "approvalRequests":[{"taskId":"a","status":"pending","summary":"Approve?"}]
        }));
        assert_eq!(model.cards.len(), 5);
        assert_eq!(model.count(Filter::NeedsYou), 1);
        assert_eq!(model.count(Filter::Working), 1);
        assert_eq!(model.count(Filter::Failed), 1);
        assert_eq!(model.count(Filter::Idle), 1);
        assert_eq!(
            model
                .cards
                .iter()
                .find(|c| c.task_id == "a")
                .unwrap()
                .activity,
            "Approve?"
        );
        assert!(
            model
                .cards
                .iter()
                .find(|c| c.task_id == "d")
                .unwrap()
                .steps
                .is_empty()
        );
    }
    #[test]
    fn search_combines_with_filter_and_preserves_orphaned_tasks() {
        let model = OverviewModel::from_snapshot(&json!({
            "agents":[{"id":"agent","name":"Terra"}],
            "projects":[{"id":"p","name":"Seafood"}],
            "tasks":[{"id":"one","agentId":"agent","projectId":"p","title":"Fix packaging","status":"running"},
                {"id":"two","agentId":"removed","projectId":"gone","title":"Orphan","status":"idle"}]
        }));
        assert_eq!(model.visible(Filter::Working, "SEAFOOD", NativeActivityGrouping::Project)[0].task_id, "one");
        assert_eq!(model.visible(Filter::All, "terra", NativeActivityGrouping::Project).len(), 1);
        assert!(model.visible(Filter::Failed, "terra", NativeActivityGrouping::Project).is_empty());
        assert_eq!(
            model.visible(Filter::Idle, "orphan", NativeActivityGrouping::Project)[0].project_name,
            "Unavailable project"
        );
    }
    #[test]
    fn working_filter_includes_whole_projects_before_applying_search() {
        let mut snapshot = json!({
            "projects":[{"id":"p","name":"Active"},{"id":"q","name":"Quiet"},{"id":"r","name":"Queued"}],
            "tasks":[
                {"id":"run","projectId":"p","status":"running"},
                {"id":"idle","projectId":"p","status":"idle","title":"Find this"},
                {"id":"done","projectId":"p","status":"completed"},
                {"id":"failed","projectId":"p","status":"failed"},
                {"id":"quiet","projectId":"q","status":"idle"},
                {"id":"archived","projectId":"q","status":"running","archived":true},
                {"id":"queued","projectId":"r","status":"queued"},
                {"id":"queued-sibling","projectId":"r","status":"idle"}]
        });
        let model = OverviewModel::from_snapshot(&snapshot);
        let ids: std::collections::HashSet<_> = model.visible(Filter::Working, "", NativeActivityGrouping::Project)
            .into_iter().map(|card| card.task_id).collect();
        assert_eq!(ids, ["run", "idle", "done", "failed", "queued", "queued-sibling"].map(String::from).into());
        assert_eq!(model.count(Filter::Working), 2, "badge still counts working agents");
        assert_eq!(model.visible(Filter::Working, "find this", NativeActivityGrouping::Project)[0].task_id, "idle");
        assert!(model.visible(Filter::Working, "Quiet", NativeActivityGrouping::Project).is_empty());
        snapshot["tasks"][0]["status"] = json!("idle");
        assert!(OverviewModel::from_snapshot(&snapshot).visible(Filter::Working, "Active", NativeActivityGrouping::Project).is_empty(),
            "a project disappears when its last working agent stops");
        assert_eq!(model.visible(Filter::All, "", NativeActivityGrouping::Project).len(), 7);
        assert_eq!(model.visible(Filter::Idle, "", NativeActivityGrouping::Project).len(), 3);
    }
    #[test]
    fn workspace_grouping_uses_authoritative_checkout_identity_and_unknown_fallback() {
        let snapshot = json!({
            "projects":[{"id":"p","name":"One"},{"id":"q","name":"Two"}],
            "tasks":[
                {"id":"run","projectId":"p","cwd":"/code/shared","hostId":"local-host","status":"running"},
                {"id":"quiet-sibling","projectId":"q","cwd":"/code/shared","hostId":"local-host","status":"idle"},
                {"id":"remote","projectId":"q","cwd":"/code/shared","hostId":"remote-host","status":"idle"},
                {"id":"unknown","projectId":"q","hostId":"local-host","status":"idle"}
            ]
        });
        let model = OverviewModel::from_snapshot(&snapshot);
        assert_eq!(
            grouped(model.cards.clone(), NativeActivityGrouping::Project).len(),
            2
        );
        let workspace_groups = grouped(model.cards.clone(), NativeActivityGrouping::Workspace);
        assert_eq!(workspace_groups.len(), 3, "host plus cwd forms workspace identity");
        assert!(workspace_groups.iter().any(|(name, cards)| {
            name == "shared" && cards.len() == 2
        }));
        assert!(workspace_groups.iter().any(|(name, cards)| {
            name == "Unknown workspace" && cards[0].task_id == "unknown"
        }));
        let working = model.visible(Filter::Working, "", NativeActivityGrouping::Workspace);
        assert_eq!(
            working.iter().map(|card| card.task_id.as_str()).collect::<HashSet<_>>(),
            HashSet::from(["run", "quiet-sibling"]),
            "Working includes quiet siblings from the selected workspace group"
        );
        assert_eq!(
            model.visible(Filter::All, "/code/shared", NativeActivityGrouping::Workspace).len(),
            3,
            "workspace paths participate in Activity search"
        );
    }
    #[gpui_kit::test]
    fn activity_grouping_toggle_rebuilds_clusters_and_emits_persistable_choice(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        use gpui_kit::{AppContext as _, Context, Entity, IntoElement, Render, Window};
        use std::{cell::Cell, rc::Rc};
        struct Harness(Entity<super::ActivityOverview>);
        impl Render for Harness {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                self.0.clone()
            }
        }
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::native_motion::initialize_for_ui_fixture(cx);
        });
        let selected = Rc::new(Cell::new(None));
        let selected_event = selected.clone();
        let mut retained = None;
        let (_, visual) = cx.add_window_view(|window, cx| {
            let overview = cx.new(|cx| {
                let mut view = super::ActivityOverview::new_with_grouping(
                    window,
                    cx,
                    NativeActivityGrouping::Project,
                );
                let now = chrono::Utc::now().timestamp_millis();
                view.set_snapshot(std::rc::Rc::new(json!({
                    "projects":[{"id":"p","name":"Monitter"}],
                    "tasks":[
                        {"id":"a","projectId":"p","cwd":"/code/main","status":"running","updatedAt":now},
                        {"id":"b","projectId":"p","cwd":"/code/feature","status":"idle","updatedAt":now}
                    ]
                })), view.palette, cx);
                view
            });
            retained = Some(overview.clone());
            gpui_kit::component::Root::new(cx.new(|_| Harness(overview)), window, cx)
        });
        let overview = retained.unwrap();
        visual.update(|_, cx| {
            cx.subscribe(
                &overview,
                move |_, event: &super::ActivityOverviewEvent, _| {
                    if let super::ActivityOverviewEvent::GroupingChanged { grouping } = event {
                        selected_event.set(Some(*grouping));
                    }
                },
            )
            .detach();
        });
        visual.simulate_resize(gpui_kit::size(px(1400.), px(900.)));
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        overview.read_with(visual, |view, _| {
            assert_eq!(view.grouping, NativeActivityGrouping::Project);
            assert_eq!(view.layout.projects.len(), 1);
        });
        let toggle = visual.debug_bounds("activity-group-workspaces").unwrap();
        visual.simulate_click(toggle.center(), Default::default());
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        overview.read_with(visual, |view, _| {
            assert_eq!(view.grouping, NativeActivityGrouping::Workspace);
            assert_eq!(view.layout.projects.len(), 2);
            assert_eq!(view.model.cards.len(), 2, "presentation toggle preserves chat population");
        });
        assert_eq!(selected.get(), Some(NativeActivityGrouping::Workspace));
    }
    #[gpui_kit::test]
    fn activity_overview_working_filter_click_keeps_and_dims_quiet_cards(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::{AppContext as _, Context, Entity, IntoElement, Render, Window};
        struct Harness(Entity<super::ActivityOverview>);
        impl Render for Harness {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement { self.0.clone() }
        }
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::surface_blend::initialize_for_ui_fixture();
            crate::native_motion::initialize_for_ui_fixture(cx);
        });
        let (_, visual) = cx.add_window_view(|window, cx| {
            let overview = cx.new(|cx| {
                let mut view = super::ActivityOverview::new(window, cx);
                view.set_snapshot(std::rc::Rc::new(json!({
                    "projects":[{"id":"p","name":"Active"},{"id":"q","name":"Quiet"}],
                    "tasks":[
                        {"id":"run","projectId":"p","status":"running","updatedAt":chrono::Utc::now().timestamp_millis()},
                        {"id":"idle","projectId":"p","status":"idle","updatedAt":chrono::Utc::now().timestamp_millis()},
                        {"id":"done","projectId":"p","status":"completed","updatedAt":chrono::Utc::now().timestamp_millis()},
                        {"id":"quiet","projectId":"q","status":"idle","updatedAt":chrono::Utc::now().timestamp_millis()}]
                })), view.palette, cx);
                view
            });
            gpui_kit::component::Root::new(cx.new(|_| Harness(overview)), window, cx)
        });
        visual.simulate_resize(gpui_kit::size(px(1800.), px(1200.)));
        let draw = |visual: &mut gpui_kit::VisualTestContext| {
            visual.run_until_parked();
            visual.update(|window, cx| window.draw(cx).clear(cx));
        };
        let alpha = |visual: &mut gpui_kit::VisualTestContext, id: &str| {
            let bounds = visual.debug_bounds(match id { "run" => "activity-card-run", "idle" => "activity-card-idle", "done" => "activity-card-done", _ => unreachable!() }).expect("card mounted");
            visual.update(|window, _| {
                let bounds = bounds.scale(window.scale_factor());
                window.painted_quads().iter().find_map(|quad| {
                    let near = |a: gpui_kit::ScaledPixels, b: gpui_kit::ScaledPixels| (a.as_f32() - b.as_f32()).abs() < 0.5;
                    (near(quad.bounds.origin.x, bounds.origin.x) && near(quad.bounds.origin.y, bounds.origin.y)
                        && near(quad.bounds.size.width, bounds.size.width) && near(quad.bounds.size.height, bounds.size.height))
                        .then(|| quad.background.as_solid().map(|color| gpui_kit::Rgba::from(color).a)).flatten()
                }).expect("actual card background painted")
            })
        };
        draw(visual);
        assert!(visual.debug_bounds("activity-dot-grid").is_some());
        let idle_bounds = visual.debug_bounds("activity-card-done").unwrap();
        visual.update(|window, _| {
            let card = idle_bounds.scale(window.scale_factor());
            let quads = window.painted_quads();
            let paints: Vec<_> = quads.iter().filter(|quad| card.contains(&quad.bounds.center()))
                .filter_map(|quad| quad.background.as_solid().map(gpui_kit::Rgba::from))
                .filter(|ink| ink.a > 0.30).collect();
            assert!(!paints.is_empty(), "completed card paints its surface: bounds={idle_bounds:?}");
            let grid_dots = quads.iter().filter(|quad| {
                let width = quad.bounds.size.width.as_f32() / window.scale_factor();
                let height = quad.bounds.size.height.as_f32() / window.scale_factor();
                width > 0.5 && width < 3. && height > 0.5 && height < 3.
                    && quad.background.as_solid().is_some_and(|color| {
                        let ink = gpui_kit::Rgba::from(color); ink.a > 0.08 && ink.a < 0.12
                    })
            }).count();
            assert!(grid_dots > 100, "native background paints the reference dot grid");
        });
        for list in [false, true] {
            if list {
                let button = visual.debug_bounds("activity-list").unwrap();
                visual.simulate_click(button.center(), Default::default());
                draw(visual);
            }
            let original = [alpha(visual, "run"), alpha(visual, "idle"), alpha(visual, "done")];
            let working = visual.debug_bounds("activity-filter-Working").unwrap();
            visual.simulate_click(working.center(), Default::default());
            draw(visual);
            assert!(visual.debug_bounds("activity-card-quiet").is_none(), "inactive projects are hidden");
            for (index, id) in ["run", "idle", "done"].into_iter().enumerate() {
                let expected = if id == "run" { 1. } else { 0.6 };
                assert!((alpha(visual, id) / original[index] - expected).abs() < 0.01,
                    "{id} has the expected painted opacity in list={list}");
            }
            let all = visual.debug_bounds("activity-filter-All").unwrap();
            visual.simulate_click(all.center(), Default::default());
            draw(visual);
            assert!(visual.debug_bounds("activity-card-quiet").is_some());
            assert!((alpha(visual, "idle") - original[1]).abs() < 0.01, "All restores full opacity");
        }
    }
    #[test]
    fn current_turn_events_and_latest_active_plan_are_truthful() {
        let model = OverviewModel::from_snapshot(&json!({
            "tasks":[{"id":"t","status":"running","createdAt":1}],
            "nativeMessageContext":{"t":{"latestUserAt":100}},
            "events":[{"taskId":"t","createdAt":90,"title":"Stale run"}, {"taskId":"t","createdAt":110,"title":"cargo test"}],
            "workPlans":[{"taskId":"t","status":"completed","updatedAt":500,"items":[{"status":"completed"}]},
                {"taskId":"t","status":"active","updatedAt":200,"items":[{"status":"completed"},{"status":"in_progress"},{"status":"pending"}]}]
        }));
        assert_eq!(model.cards[0].activity, "cargo test");
        assert_eq!(
            model.cards[0].steps,
            [State::Done, State::Working, State::Idle]
        );
    }
    #[test]
    fn organic_geometry_is_deterministic_and_collision_free_at_both_levels() {
        let mut dimensions = Vec::new();
        for count in [1, 2, 3, 8, 71, 500] {
            let (rects, width, height) = cluster_layout(count);
            assert_eq!(
                format!("{:?}", rects),
                format!("{:?}", cluster_layout(count).0)
            );
            for (i, rect) in rects.iter().enumerate() {
                assert!(
                    rect.x >= 0.
                        && rect.y >= 0.
                        && rect.x + rect.w <= width
                        && rect.y + rect.h <= height
                );
                for other in &rects[..i] {
                    assert!(!rect.overlaps(*other, 9.));
                }
            }
            dimensions.push((width, height));
        }
        let clusters = organic_positions(&dimensions, &[], 32.);
        for (i, rect) in clusters.iter().enumerate() {
            for other in &clusters[..i] {
                assert!(!rect.overlaps(*other, 31.));
            }
        }
        assert_eq!(clamped_zoom(f32::NAN), 1.);
        assert_eq!(clamped_zoom(-1.), 0.5);
        assert_eq!(clamped_zoom(3.), 1.5);
    }
    #[test]
    fn retained_slots_survive_status_changes_filters_and_new_cards() {
        let now = 100_000_000;
        let mut snapshot = json!({"projects":[{"id":"p","name":"Z strongest"},{"id":"q","name":"A quiet"}],"tasks":[
            {"id":"a","projectId":"p","status":"running","updatedAt":now},
            {"id":"b","projectId":"p","status":"idle","updatedAt":now},
            {"id":"c","projectId":"q","status":"idle","updatedAt":now}]});
        let mut layout = RetainedLayout::default();
        layout.sync(OverviewModel::from_snapshot_at(&snapshot, now).cards, 1200., 700., NativeActivityGrouping::Project);
        let coordinates = |layout: &RetainedLayout, project: &str, task: &str| {
            let p = &layout.projects[project];
            let r = p.cards[task];
            (r.x, r.y)
        };
        let a = coordinates(&layout, "p", "a");
        let b = coordinates(&layout, "p", "b");
        let c = coordinates(&layout, "q", "c");
        let p_bounds = layout.projects["p"].bounds.unwrap();
        let q_bounds = layout.projects["q"].bounds.unwrap();
        assert!(!p_bounds.overlaps(q_bounds, 7.9), "organic project islands do not overlap");
        snapshot["tasks"][0]["status"] = json!("idle");
        snapshot["tasks"][1]["status"] = json!("running");
        for filter in [Filter::Working, Filter::Idle, Filter::All] {
            let model = OverviewModel::from_snapshot_at(&snapshot, now);
            let visible = model.visible(filter, "", NativeActivityGrouping::Project);
            if filter == Filter::Working {
                assert_eq!(visible.iter().map(|card| card.task_id.as_str()).collect::<Vec<_>>(), ["b", "a"]);
            } else {
                assert!(visible.iter().all(|card| filter.matches(card.state)));
            }
            // Repeated full-population syncs retain positions while filters change.
            layout.sync(model.cards, 1200., 700., NativeActivityGrouping::Project);
            assert_eq!(a, coordinates(&layout, "p", "a"));
            assert_eq!(b, coordinates(&layout, "p", "b"));
            assert_eq!(c, coordinates(&layout, "q", "c"));
        }
        snapshot["tasks"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"d","projectId":"p","status":"running","updatedAt":now}));
        layout.sync(OverviewModel::from_snapshot_at(&snapshot, now).cards, 1200., 700., NativeActivityGrouping::Project);
        let d = coordinates(&layout, "p", "d");
        assert_eq!(a, coordinates(&layout, "p", "a"));
        assert_eq!(b, coordinates(&layout, "p", "b"));
        assert_eq!(c, coordinates(&layout, "q", "c"));
        snapshot["tasks"]
            .as_array_mut()
            .unwrap()
            .retain(|task| task["id"] != "b");
        snapshot["tasks"][0]["updatedAt"] = json!(now - 12 * 60 * 60 * 1000 - 1);
        let model = OverviewModel::from_snapshot_at(&snapshot, now);
        assert_eq!(
            model.cards.len(),
            2,
            "removed and expired cards leave the live population"
        );
        layout.sync(model.cards, 1200., 700., NativeActivityGrouping::Project);
        assert_eq!(c, coordinates(&layout, "q", "c"));
        assert_eq!(d, coordinates(&layout, "p", "d"));
        assert_eq!(
            a,
            coordinates(&layout, "p", "a"),
            "returning chats retain their original slots"
        );
    }
    #[test]
    fn responsive_map_centers_sparse_content_in_both_axes() {
        let (positions, width, height) =
            distributed_positions(&[(320., 180.)], 14., 1200., 700.);
        assert_eq!((width, height), (1200., 700.));
        let cluster = positions[0];
        assert!((cluster.x - 440.).abs() < 0.01);
        assert!((cluster.y - 260.).abs() < 0.01);
        assert!((cluster.x - (width - cluster.x - cluster.w)).abs() < 0.01);
        assert!((cluster.y - (height - cluster.y - cluster.h)).abs() < 0.01);
    }
    #[test]
    fn responsive_map_reflows_as_a_compact_organic_island() {
        let sizes = [
            (220., 160.),
            (205., 145.),
            (240., 150.),
            (210., 175.),
            (225., 155.),
        ];
        let (wide, wide_width, wide_height) =
            distributed_positions(&sizes, 8., 1200., 520.);
        for (index, rect) in wide.iter().enumerate() {
            for other in &wide[..index] {
                assert!(!rect.overlaps(*other, 7.9), "compact clusters retain an 8px breathing gap");
            }
        }
        assert!(
            wide.iter().map(|rect| rect.x.round() as i32).collect::<HashSet<_>>().len() > 2
                && wide.iter().map(|rect| rect.y.round() as i32).collect::<HashSet<_>>().len() > 2,
            "cluster origins should not collapse onto grid rows or columns"
        );
        let bounds = |rects: &[MapRect]| {
            let left = rects.iter().map(|rect| rect.x).fold(f32::INFINITY, f32::min);
            let top = rects.iter().map(|rect| rect.y).fold(f32::INFINITY, f32::min);
            let right = rects.iter().map(|rect| rect.x + rect.w).fold(f32::NEG_INFINITY, f32::max);
            let bottom = rects.iter().map(|rect| rect.y + rect.h).fold(f32::NEG_INFINITY, f32::max);
            (left, top, right, bottom)
        };
        let (left, top, right, bottom) = bounds(&wide);
        assert!((left - (wide_width - right)).abs() < 0.01);
        assert!((top - (wide_height - bottom)).abs() < 0.01);

        let (tall, _, _) = distributed_positions(&sizes, 8., 520., 1200.);
        assert!(wide.iter().zip(&tall).any(|(a, b)| (a.x - b.x).abs() > 20. || (a.y - b.y).abs() > 20.),
            "the island should remap when the viewport aspect changes");
        let wide_bounds = bounds(&wide);
        let tall_bounds = bounds(&tall);
        assert!(wide_bounds.2 - wide_bounds.0 > wide_bounds.3 - wide_bounds.1);
        assert!(tall_bounds.3 - tall_bounds.1 > tall_bounds.2 - tall_bounds.0);
    }
    #[test]
    fn activity_canvas_pan_tracks_the_hand_and_stays_recoverable() {
        let moved = panned_placement(
            Placement { x: 10., y: -20. },
            Placement { x: 85., y: 45. },
            180.,
            160.,
        );
        assert_eq!((moved.x, moved.y), (95., 25.));
        let clamped = panned_placement(moved, Placement { x: 500., y: -500. }, 180., 160.);
        assert_eq!((clamped.x, clamped.y), (180., -160.));
    }
    #[test]
    fn radial_satellites_surround_the_hub_with_high_density() {
        let (rects, width, height) = cluster_layout(8);
        let hub = rects[0];
        assert!(rects[1..].iter().all(|r| !r.overlaps(hub, 23.9)), "orbiting cards keep 24px of clearance from the project");
        let center = (hub.x + hub.w / 2., hub.y + hub.h / 2.);
        assert!(rects[1..].iter().any(|r| r.x + r.w < center.0));
        assert!(rects[1..].iter().any(|r| r.x > center.0));
        assert!(rects[1..].iter().any(|r| r.y + r.h < center.1));
        assert!(rects[1..].iter().any(|r| r.y > center.1));
        assert!(
            (center.0 / width - 0.5).abs() < 0.2 && (center.1 / height - 0.5).abs() < 0.2,
            "project hub stays near the cluster center"
        );
        let occupied: f32 = rects.iter().map(|r| r.w * r.h).sum();
        assert!(
            occupied / (width * height) > 0.55,
            "eight-card cluster should fill most of its bounds: {width} x {height}"
        );
    }
    #[test]
    fn activity_overview_inertia_decays_and_stops_at_map_edges() {
        let mut motion = Motion { x: 20., y: 20., vx: 18., vy: 8. };
        let ticks = (0..120).find(|_| !motion.coast(0., 0.)).expect("finite glide");
        assert!(ticks < 60);
        assert!(motion.x > 50. && motion.x < 150.);
        let stopped = (motion.x, motion.y);
        assert!(!motion.coast(0., 0.));
        assert_eq!((motion.x, motion.y), stopped);
        motion = Motion { x: 1., y: 1., vx: -18., vy: -18. };
        assert!(!motion.coast(0., 0.));
        assert_eq!((motion.x, motion.y, motion.vx, motion.vy), (0., 0., 0., 0.));
    }
    #[test]
    fn follower_spring_converges_without_unbounded_animation() {
        let mut motion = Motion::default();
        let target = Placement { x: 310., y: 120. };
        let settled = (0..240).any(|_| !motion.settle(target));
        assert!(settled);
        assert_eq!(motion.x, target.x);
        assert_eq!(motion.y, target.y);
    }
    #[test]
    fn activity_overview_children_have_distance_sensitive_springs() {
        let mut project = RetainedProject::default();
        project.hub = Some(MapRect { x: 0., y: 0., w: HUB_W, h: HUB_H });
        project.cards.insert("near".into(), MapRect { x: 180., y: 0., w: CARD_W, h: CARD_H });
        project.cards.insert("far".into(), MapRect { x: 650., y: 0., w: CARD_W, h: CARD_H });
        let near_strength = project.spring_strength("near");
        let far_strength = project.spring_strength("far");
        assert!(near_strength > far_strength, "farther cards have more lag");
        let target = Placement { x: 120., y: 45. };
        let mut near = Motion::default();
        let mut far = Motion::default();
        near.spring(target, near_strength); far.spring(target, far_strength);
        assert!(near.x > far.x && near.x < target.x);
        for _ in 0..240 { near.spring(target, near_strength); far.spring(target, far_strength); }
        assert_eq!((near.x, near.y), (target.x, target.y));
        assert_eq!((far.x, far.y), (target.x, target.y));
    }
    #[test]
    fn twelve_hour_population_expires_without_trusting_stale_running_status() {
        let now = 100_000_000;
        let cutoff = now - 12 * 60 * 60 * 1000;
        let snapshot = json!({"tasks":[
            {"id":"boundary","updatedAt":cutoff},
            {"id":"stale","projectId":"old-project","status":"running","updatedAt":cutoff-1},
            {"id":"event","status":"running","updatedAt":1},
            {"id":"unknown","status":"running"}],
            "events":[{"taskId":"event","title":"Working","createdAt":now-1}]});
        let model = OverviewModel::from_snapshot_at(&snapshot, now);
        assert_eq!(model.cards.len(), 2);
        assert!(
            grouped(model.visible(Filter::All, "", NativeActivityGrouping::Project), NativeActivityGrouping::Project)
                .iter()
                .all(|(_, cards)| cards.iter().all(|card| card.project_id != "old-project")),
            "stale-only projects must not produce empty hubs"
        );
        assert_eq!(model.visible(Filter::All, "stale", NativeActivityGrouping::Project).len(), 0);
        assert_eq!(model.count(Filter::Working), 1);
        assert_eq!(
            OverviewModel::from_snapshot_at(&snapshot, now + 1)
                .cards
                .len(),
            1
        );
    }
    #[test]
    fn compact_card_geometry_scales_with_supported_fonts_and_zoom() {
        for font in [14., 18., 24., 32.] {
            for zoom in [0.5, 1., 1.5] {
                let scale = interface_font_ratio(px(font)) * zoom;
                assert!(CARD_H * scale >= (14. + 4. + 3. * 12. + 4. * 5. + 20.) * scale);
            }
        }
    }
    #[test]
    fn pending_user_questions_take_priority_and_resolved_requests_do_not() {
        let mut snapshot = json!({"tasks":[{"id":"t","status":"starting"}],
            "approvalRequests":[{"taskId":"t","status":"pending","summary":"Choose an option", "input":{"kind":"questions","questions":[{"id":"choice","header":"Choice","question":"Which option?"}]}}]});
        let model = OverviewModel::from_snapshot(&snapshot);
        assert_eq!(model.cards[0].state, State::NeedsYou);
        assert_eq!(model.cards[0].activity, "Which option?");
        snapshot["approvalRequests"][0]["status"] = json!("answered");
        assert_eq!(
            OverviewModel::from_snapshot(&snapshot).cards[0].state,
            State::Working
        );
    }
    #[test]
    fn body_only_publications_do_not_invalidate_model_or_delegation_cache() {
        let old = json!({"tasks":[{"id":"t","status":"running","nativeSessionId":"thread"}],"messages":[{"text":"old"}]});
        let mut next = old.clone();
        next["messages"][0]["text"] = json!("streaming body");
        assert!(!relevant_changed(&old, &next));
        assert!(!child_inputs_changed(&old, &next));
        next["tasks"][0]["status"] = json!("completed");
        assert!(relevant_changed(&old, &next));
        assert!(!child_inputs_changed(&old, &next));
    }
    #[test]
    fn updated_time_is_explicit_and_never_inferred_from_task_creation() {
        assert_eq!(updated_label(0, 100_000), "");
        assert_eq!(updated_label(60_000, 240_000), "updated 3m");
    }
}
