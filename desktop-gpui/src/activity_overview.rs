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
use std::{
    collections::{BTreeMap, HashMap, HashSet},
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
    state: State,
    activity: String,
    /// Individual plan item states, never an inferred completion percentage.
    steps: Vec<State>,
    children: usize,
    attention: usize,
    updated_at: i64,
}
#[derive(Clone, Default)]
struct OverviewModel {
    cards: Vec<Card>,
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
        Self { cards }
    }
    fn visible(&self, filter: Filter, query: &str) -> Vec<Card> {
        let query = query.trim().to_lowercase();
        self.cards
            .iter()
            .filter(|card| {
                filter.matches(card.state)
                    && (query.is_empty()
                        || format!(
                            "{} {} {} {}",
                            card.title, card.agent, card.model, card.project_name
                        )
                        .to_lowercase()
                        .contains(&query))
            })
            .cloned()
            .collect()
    }
    fn count(&self, filter: Filter) -> usize {
        self.cards
            .iter()
            .filter(|card| filter.matches(card.state))
            .count()
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
const HUB_W: f32 = 144.;
const HUB_H: f32 = 60.;
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
// Dense frontier packing keeps every new rectangle touching the existing cluster.
// A bounded target width produces compact, irregular stair-step silhouettes.
fn organic_positions(sizes: &[(f32, f32)], reserved: &[MapRect], gap: f32) -> Vec<MapRect> {
    let area: f32 = sizes.iter().map(|(w, h)| (w + gap) * (h + gap)).sum();
    packed_positions(sizes, reserved, gap, area.sqrt().max(640.))
}
fn packed_positions(
    sizes: &[(f32, f32)],
    reserved: &[MapRect],
    gap: f32,
    target_width: f32,
) -> Vec<MapRect> {
    let mut occupied = reserved.to_vec();
    let mut result = Vec::new();
    let target = target_width.max(sizes.iter().map(|(w, _)| *w).fold(0., f32::max));
    for &(w, h) in sizes {
        let mut candidates = vec![(0., 0.)];
        for r in &occupied {
            candidates.push((r.x + r.w + gap, r.y));
            candidates.push((r.x, r.y + r.h + gap));
            candidates.push((0., r.y + r.h + gap));
        }
        let best = candidates
            .into_iter()
            .map(|(x, y)| MapRect { x, y, w, h })
            .filter(|r| occupied.iter().all(|o| !r.overlaps(*o, gap - 0.01)))
            .min_by(|a, b| {
                let score = |r: &MapRect| r.y + (r.x + w - target).max(0.) * 3. + r.x * 0.16;
                score(a)
                    .total_cmp(&score(b))
                    .then(a.y.total_cmp(&b.y))
                    .then(a.x.total_cmp(&b.x))
            })
            .unwrap_or(MapRect {
                x: 0.,
                y: occupied.iter().map(|r| r.y + r.h + gap).fold(0., f32::max),
                w,
                h,
            });
        occupied.push(best);
        result.push(best);
    }
    result
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
fn grouped(cards: Vec<Card>) -> Vec<(String, Vec<Card>)> {
    let mut groups = BTreeMap::<(String, String), Vec<Card>>::new();
    for card in cards {
        groups
            .entry((card.project_name.clone(), card.project_id.clone()))
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
    fn settle(&mut self, target: Placement) -> bool {
        self.vx = (self.vx + (target.x - self.x) * 0.16) * 0.64;
        self.vy = (self.vy + (target.y - self.y) * 0.16) * 0.64;
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
    hub: Option<MapRect>,
    bounds: Option<MapRect>,
    target: Motion,
    followers: Motion,
}
#[derive(Default)]
struct RetainedLayout {
    projects: HashMap<String, RetainedProject>,
}
impl RetainedLayout {
    fn sync(&mut self, cards: Vec<Card>, available_width: f32) {
        for (_, cards) in grouped(cards) {
            let key = cards[0].project_id.clone();
            let reserved: Vec<_> = self.projects.values().filter_map(|p| p.bounds).collect();
            let project = self.projects.entry(key).or_default();
            if project.hub.is_none() {
                let (placements, _, _) = cluster_layout(cards.len());
                project.hub = Some(placements[0]);
                for (card, rect) in cards.iter().zip(placements.iter().skip(1)) {
                    project.cards.insert(card.task_id.clone(), *rect);
                }
            }
            let hub = project.hub.unwrap();
            for card in cards {
                if !project.cards.contains_key(&card.task_id) {
                    let mut occupied: Vec<_> = project.cards.values().copied().collect();
                    occupied.push(hub);
                    if let Some(origin) = project.bounds {
                        occupied.extend(
                            reserved
                                .iter()
                                .filter(|r| r.x != origin.x || r.y != origin.y)
                                .map(|r| MapRect {
                                    x: r.x - origin.x,
                                    y: r.y - origin.y,
                                    w: r.w,
                                    h: r.h,
                                }),
                        );
                    }
                    let rect = satellite_position(hub, &occupied, project.cards.len(), true);
                    project.cards.insert(card.task_id, rect);
                }
            }
            {
                let w = project
                    .cards
                    .values()
                    .map(|r| r.x + r.w + 12.)
                    .fold(HUB_W + 24., f32::max);
                let h = project
                    .cards
                    .values()
                    .map(|r| r.y + r.h + 12.)
                    .fold(HUB_H + 24., f32::max);
                if let Some(bounds) = project.bounds.as_mut() {
                    bounds.w = w;
                    bounds.h = h;
                } else {
                    project.bounds =
                        Some(packed_positions(&[(w, h)], &reserved, 14., available_width)[0]);
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
pub enum ActivityOverviewEvent {
    OpenTask { task_id: String },
    Close,
}
impl EventEmitter<ActivityOverviewEvent> for ActivityOverview {}
pub struct ActivityOverview {
    focus: FocusHandle,
    search: Entity<InputState>,
    snapshot: Rc<Value>,
    palette: Palette,
    model: OverviewModel,
    child_counts: HashMap<String, usize>,
    filter: Filter,
    list: bool,
    zoom: f32,
    layout: RetainedLayout,
    dragging: Option<(String, Point<Pixels>)>,
    motion_task: Option<Task<()>>,
    communications: Rc<RefCell<communications::Communications>>,
    _communication_clock: Task<()>,
    _subscription: Subscription,
    _refresh: Task<()>,
}
impl ActivityOverview {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Find an agent or chat…"));
        let subscription = cx.subscribe(&search, |_, _, _: &InputEvent, cx| cx.notify());
        let refresh = cx.spawn(async move |overview, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(60))
                    .await;
                if overview
                    .update(cx, |overview, cx| {
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
            child_counts: HashMap::new(),
            filter: Filter::All,
            list: false,
            zoom: 1.,
            layout: RetainedLayout::default(),
            dragging: None,
            motion_task: None,
            communications,
            _communication_clock: communication_clock,
            _subscription: subscription,
            _refresh: refresh,
        }
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
    fn drag_mouse_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some((key, last)) = self.dragging.as_mut() else {
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            self.dragging = None;
            return;
        }
        let scale = f32::from(px(1.)) * self.zoom * interface_font_ratio(cx.theme().font_size);
        let project = self.layout.projects.get_mut(key).unwrap();
        let bounds = project.bounds.unwrap();
        project.target.x =
            (project.target.x + f32::from(event.position.x - last.x) / scale).max(-bounds.x);
        project.target.y =
            (project.target.y + f32::from(event.position.y - last.y) / scale).max(-bounds.y);
        *last = event.position;
        if self.motion_task.is_none() {
            self.motion_task = Some(cx.spawn(async move |entity, cx| {
                for _ in 0..240 {
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(16))
                        .await;
                    let moving = entity
                        .update(cx, |overview, cx| {
                            let mut moving = false;
                            for p in overview.layout.projects.values_mut() {
                                moving |= p.followers.settle(Placement {
                                    x: p.target.x,
                                    y: p.target.y,
                                });
                            }
                            cx.notify();
                            if !moving {
                                overview.motion_task = None;
                            }
                            moving
                        })
                        .unwrap_or(false);
                    if !moving {
                        break;
                    }
                }
                let _ = entity.update(cx, |overview, _| overview.motion_task = None);
            }));
        }
        cx.notify();
    }
    fn card(&self, card: &Card, list: bool, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette;
        let zoom = if list { 1. } else { self.zoom };
        let ratio = interface_font_ratio(cx.theme().font_size);
        let scaled = |n: f32| px(n * zoom * ratio);
        let small = scaled(10.);
        let color = card.state.color(p);
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
        let mut outline = rgb(color);
        outline.a = if card.state == State::Idle {
            0.20
        } else {
            0.52
        };
        let mut progress = div()
            .flex()
            .gap(scaled(3.))
            .w_full()
            .h(scaled(3.))
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
                let mut fill = rgb(state.color(p));
                if state == State::Idle {
                    fill.a = 0.28;
                }
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
            .map(|image| img(image).size(scaled(14.)).into_any_element())
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
            .p(scaled(9.))
            .rounded(scaled(10.))
            .border_1()
            .border_color(outline)
            .bg(background)
            .shadow_sm()
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
        body = body.on_click(cx.listener(move |_, _, _, cx| {
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
    fn communication_layer(&self, endpoints: HashMap<String, MapRect>, scale: f32, cx: &mut Context<Self>) -> AnyElement {
        let hubs: Vec<_> = self.layout.projects.values().filter_map(|project| {
            let mut hub = project.hub?; let bounds = project.bounds?;
            hub.x += bounds.x + project.target.x; hub.y += bounds.y + project.target.y;
            Some(hub)
        }).collect();
        let links: Vec<_> = self.communications.borrow().links.values().filter_map(|link| {
            let a=*endpoints.get(&link.pair.0)?; let b=*endpoints.get(&link.pair.1)?;
            let mut obstacles=hubs.clone();
            obstacles.extend(endpoints.iter().filter(|(id,_)| **id!=link.pair.0 && **id!=link.pair.1).map(|(_,r)| *r));
            let curve=communications::Curve::routed(a,b,&obstacles);
            Some((link.clone(),a,b,curve))
        }).collect();
        let has_links = !links.is_empty();
        let state = self.communications.clone();
        let color = self.palette.accent;
        let motion = !cx.reduce_motion() && crate::native_motion::motion_allowed();
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
                for (link, a, b, curve) in &links {
                    if !visible(*a) || !visible(*b) { continue; }
                    if window.is_window_active() { state.borrow_mut().painted.insert(link.pair.clone()); }
                    let alpha = link.opacity();
                    for (width, opacity) in [(8., 0.05), (4., 0.14), (1.4, 0.85)] {
                        let mut path = PathBuilder::stroke(px(width * scale));
                        path.move_to(scaled(curve.start));
                        for step in 1..=32 { path.line_to(scaled(curve.at(step as f32 / 32.))); }
                        if let Ok(path) = path.build() { window.paint_path(path, rgb(color).opacity(opacity * alpha)); }
                    }
                    for forward in [true, false] {
                        if (forward && !link.forward) || (!forward && !link.reverse) { continue; }
                        for index in 0..3 {
                            let travel = (index as f32 / 3. + paint_phase.get()) % 1.;
                            let t = 0.07 + 0.86 * if forward { travel } else { 1. - travel };
                            let (x,y) = curve.at(t);
                            let (mut dx,mut dy) = curve.tangent(t);
                            if !forward { dx = -dx; dy = -dy; }
                            let length = dx.hypot(dy).max(0.001); dx /= length; dy /= length;
                            // Opposing traffic shares the curve, with a small lane offset.
                            let lane = if link.forward && link.reverse { 2.5 } else { 0. };
                            let tip = (x - dy*lane, y + dx*lane);
                            let mut arrow = PathBuilder::stroke(px(1.7 * scale));
                            arrow.move_to(scaled((tip.0-dx*7.-dy*3.5,tip.1-dy*7.+dx*3.5)));
                            arrow.line_to(scaled(tip));
                            arrow.line_to(scaled((tip.0-dx*7.+dy*3.5,tip.1-dy*7.-dx*3.5)));
                            if let Ok(arrow) = arrow.build() { window.paint_path(arrow,rgb(color).opacity(alpha)); }
                        }
                    }
                }
            }).absolute().size_full());
        if has_links && motion {
            layer.with_animation("activity-communication-flow", Animation::new(Duration::from_millis(1800))
                .repeat_synced().with_max_fps(30.), move |layer, progress| { phase.set(progress); layer }).into_any_element()
        } else { layer.into_any_element() }
    }
    fn cluster(
        &self,
        index: usize,
        name: String,
        cards: Vec<Card>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = self.palette;
        let zoom = self.zoom;
        let ratio = interface_font_ratio(cx.theme().font_size);
        let scaled = move |value: f32| px(value * zoom * ratio);
        let key = cards[0].project_id.clone();
        let project = rows(&self.snapshot, "projects").iter()
            .find(|project| text(project, "id") == key);
        let project_color = design::parse_color(project.map_or("", |project| text(project, "color")), p.accent);
        let project_icon = crate::shell::project_icon(project.map_or("", |project| text(project, "icon")));
        let retained = &self.layout.projects[&key];
        let bounds = retained.bounds.unwrap();
        let (width, height) = (bounds.w, bounds.h);
        let lag = Placement {
            x: retained.followers.x - retained.target.x,
            y: retained.followers.y - retained.target.y,
        };
        let mut placements = vec![retained.hub.unwrap()];
        placements.extend(cards.iter().map(|c| {
            let mut r = retained.cards[&c.task_id];
            r.x += lag.x;
            r.y += lag.y;
            r
        }));
        let hub = Placement {
            x: placements[0].x + HUB_W / 2.,
            y: placements[0].y + HUB_H / 2.,
        };
        let edges: Vec<_> = cards
            .iter()
            .enumerate()
            .map(|(index, card)| (placements[index + 1], card.state))
            .collect();
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
        let links = if has_working && motion {
            links.with_animation("activity-working-links", Animation::new(Duration::from_millis(900))
                .repeat_synced().with_max_fps(30.), move |links, progress| { phase.set(progress); links }).into_any_element()
        } else { links.into_any_element() };
        let mut cluster = div()
            .id(SharedString::from(format!("activity-project-{index}")))
            .relative().w(scaled(width)).h(scaled(height)).flex_shrink_0().child(links);
        for (index, card) in cards.iter().enumerate() {
            let placement = placements[index + 1];
            cluster = cluster.child(
                div()
                    .absolute()
                    .left(scaled(placement.x))
                    .top(scaled(placement.y))
                    .child(self.card(card, false, cx)),
            );
        }
        let working = cards.iter().filter(|c| c.state == State::Working).count();
        let needs = cards.iter().filter(|c| c.state == State::NeedsYou).count();
        let failed = cards.iter().filter(|c| c.state == State::Failed).count();
        let summary = [
            (working, "working"),
            (needs, "need you"),
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
                            overview.dragging = Some((key.clone(), event.position));
                            cx.notify();
                            cx.stop_propagation();
                        }),
                    )
                    .absolute()
                    .left(scaled(hub.x - 72.))
                    .top(scaled(hub.y - 30.))
                    .w(scaled(144.))
                    .min_h(scaled(60.))
                    .rounded(scaled(27.))
                    .p(scaled(8.))
                    .border_1()
                    .border_color(rgb(project_color).opacity(0.7))
                    .bg(rgb(project_color).opacity(0.5))
                    .child(
                        div()
                            .flex().items_center().gap(scaled(5.)).min_w_0()
                            .font_weight(FontWeight::MEDIUM)
                            .text_size(cx.theme().font_size * zoom)
                            .child(div().flex_shrink_0()
                                .debug_selector(|| "activity-project-icon".into())
                                .child(gpui_kit::component::Icon::new(project_icon).with_size(scaled(16.))))
                            .child(div().flex_1().min_w_0().truncate().child(name))
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
        self.layout.sync(
            self.model.cards.clone(),
            (f32::from(window.viewport_size().width) - 64.)
                / (self.zoom * interface_font_ratio(cx.theme().font_size)),
        );
        let p = self.palette;
        let font = cx.theme().font_size;
        let query = self.search.read(cx).value().to_string();
        let visible = self.model.visible(self.filter, &query);
        let project_count = self
            .model
            .cards
            .iter()
            .map(|card| &card.project_id)
            .filter(|id| !id.is_empty())
            .collect::<HashSet<_>>()
            .len();
        let mut filters = div().flex().items_center().gap(px(5.)).flex_wrap();
        for filter in Filter::ALL {
            let selected = self.filter == filter;
            filters = filters.child(
                Button::new(SharedString::from(format!(
                    "activity-filter-{}",
                    filter.label()
                )))
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
            .px(px(15.))
            .py(px(10.))
            .border_b_1()
            .border_color(design::surface_color(p.line))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("Activity"))
                    .child(
                        div()
                            .text_size(font - px(2.))
                            .text_color(rgb(p.muted))
                            .child(format!(
                                "{} chats · {project_count} projects · last 12h",
                                self.model.cards.len()
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
            let mut list = div().flex().flex_col().gap(px(15.)).w_full();
            for (name, cards) in grouped(visible) {
                let mut group = div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .w_full()
                    .child(div().font_weight(FontWeight::MEDIUM).px(px(4.)).child(name));
                for card in cards {
                    group = group.child(self.card(&card, true, cx));
                }
                list = list.child(group);
            }
            content = content.child(list);
        } else {
            let mut endpoints = HashMap::new();
            for card in &visible {
                let project = &self.layout.projects[&card.project_id];
                let bounds = project.bounds.unwrap();
                let mut rect = project.cards[&card.task_id];
                rect.x += bounds.x + project.followers.x;
                rect.y += bounds.y + project.followers.y;
                endpoints.insert(card.task_id.clone(), rect);
            }
            let groups = grouped(visible);
            let scale = self.zoom * interface_font_ratio(font);
            let width = self
                .layout
                .projects
                .values()
                .filter_map(|p| p.bounds.map(|r| r.x + r.w + p.target.x.max(0.)))
                .fold(0., f32::max)
                + 24.;
            let height = self
                .layout
                .projects
                .values()
                .filter_map(|p| p.bounds.map(|r| r.y + r.h + p.target.y.max(0.)))
                .fold(0., f32::max)
                + 24.;
            let mut graph = div()
                .relative()
                .w(px(width * scale))
                .h(px(height * scale))
                .flex_shrink_0()
                .child(self.communication_layer(endpoints, scale, cx));
            for (index, (name, cards)) in groups.into_iter().enumerate() {
                let project = &self.layout.projects[&cards[0].project_id];
                let position = project.bounds.unwrap();
                graph = graph.child(
                    div()
                        .absolute()
                        .left(px((position.x + project.target.x) * scale))
                        .top(px((position.y + project.target.y) * scale))
                        .child(self.cluster(index, name, cards, cx)),
                );
            }
            content = content.child(graph);
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
            .child(div().child("Dashed line = working · Glowing arrows = communication"))
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
                                let _ = released.update(cx, |overview, _| overview.dragging = None);
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
            .child(content)
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
        CARD_H, Filter, Motion, OverviewModel, Placement, RetainedLayout, State,
        child_inputs_changed, clamped_zoom, cluster_layout, grouped, interface_font_ratio,
        organic_positions, px, relevant_changed, updated_label,
    };
    use serde_json::json;
    #[gpui_kit::test]
    fn activity_overview_native_communication_paint_and_card_interaction(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::{AppContext as _, IntoElement, Entity, Render, Window, Context};
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
        overview.read_with(visual, |view,_| {
            let state = view.communications.borrow();
            assert_eq!(state.links.len(),1);
            assert_eq!(state.painted.len(),1,"both real endpoints painted in foreground");
            let link = state.links.values().next().unwrap();
            assert!(link.forward && link.reverse);
        });
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
        assert_eq!(model.visible(Filter::Working, "SEAFOOD")[0].task_id, "one");
        assert_eq!(model.visible(Filter::All, "terra").len(), 1);
        assert!(model.visible(Filter::Failed, "terra").is_empty());
        assert_eq!(
            model.visible(Filter::Idle, "orphan")[0].project_name,
            "Unavailable project"
        );
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
        layout.sync(OverviewModel::from_snapshot_at(&snapshot, now).cards, 1200.);
        let coordinates = |layout: &RetainedLayout, project: &str, task: &str| {
            let p = &layout.projects[project];
            let r = p.cards[task];
            let b = p.bounds.unwrap();
            (b.x, b.y, r.x, r.y)
        };
        let a = coordinates(&layout, "p", "a");
        let b = coordinates(&layout, "p", "b");
        let c = coordinates(&layout, "q", "c");
        assert_eq!(
            (a.0, a.1),
            (0., 0.),
            "strongest project wins top left despite its alphabetically later name"
        );
        snapshot["tasks"][0]["status"] = json!("idle");
        snapshot["tasks"][1]["status"] = json!("running");
        for filter in [Filter::Working, Filter::Idle, Filter::All] {
            let model = OverviewModel::from_snapshot_at(&snapshot, now);
            let visible = model.visible(filter, "");
            assert!(visible.iter().all(|card| filter.matches(card.state)));
            // Render allocates from the complete population before applying visibility.
            layout.sync(model.cards, 1200.);
            assert_eq!(a, coordinates(&layout, "p", "a"));
            assert_eq!(b, coordinates(&layout, "p", "b"));
            assert_eq!(c, coordinates(&layout, "q", "c"));
        }
        snapshot["tasks"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"d","projectId":"p","status":"running","updatedAt":now}));
        layout.sync(OverviewModel::from_snapshot_at(&snapshot, now).cards, 1200.);
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
        layout.sync(model.cards, 1200.);
        assert_eq!(c, coordinates(&layout, "q", "c"));
        assert_eq!(d, coordinates(&layout, "p", "d"));
        assert_eq!(
            a,
            coordinates(&layout, "p", "a"),
            "returning chats retain their original slots"
        );
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
    fn follower_spring_converges_without_unbounded_animation() {
        let mut motion = Motion::default();
        let target = Placement { x: 310., y: 120. };
        let settled = (0..240).any(|_| !motion.settle(target));
        assert!(settled);
        assert_eq!(motion.x, target.x);
        assert_eq!(motion.y, target.y);
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
            grouped(model.visible(Filter::All, ""))
                .iter()
                .all(|(_, cards)| cards.iter().all(|card| card.project_id != "old-project")),
            "stale-only projects must not produce empty hubs"
        );
        assert_eq!(model.visible(Filter::All, "stale").len(), 0);
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
                assert!(CARD_H * scale >= (14. + 3. + 3. * 12. + 4. * 5. + 18.) * scale);
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
