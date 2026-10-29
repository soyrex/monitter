//! Content-free, one-window chat/logo render counters for the finite profiler.
//! This module is compiled only with `performance-profiling`.
use gpui_kit::{EntityId, WindowId};
use serde_json::{Value, json};
use std::{cell::RefCell, sync::OnceLock};

const MAX_ENTRIES: usize = 64;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind { Pane, Logo }

struct Entry {
    kind: Kind,
    entity: EntityId,
    owner: Option<EntityId>,
    ordinal: usize,
    renders: u64,
    image_present_renders: u64,
    animation_requested_renders: u64,
    animation_enabled_renders: u64,
    animation_evaluations: u64,
    working_logo_renders: u64,
}

struct Observation {
    window: WindowId,
    entries: Vec<Entry>,
    next_ordinal: usize,
    dropped_chat_render_events: u64,
    dropped_logo_render_events: u64,
    dropped_logo_owner_events: u64,
    dropped_animation_evaluation_events: u64,
}

thread_local! {
    static ACTIVE: RefCell<Option<Observation>> = const { RefCell::new(None) };
}

/// The override is read at most once and affects only WorkingLogo::render.
pub(crate) fn static_working_logo_override() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("MONITTER_GPUI_PERF_STATIC_WORKING_LOGO")
        .is_some_and(|value| value == "1"))
}

/// The pane-splicing comparison is available only in profiling builds and is
/// opt-in for the lifetime of the process.
pub(crate) fn retained_chat_splice_override() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("MONITTER_GPUI_PERF_SPLICE_CHAT_PANES")
        .is_some_and(|value| value == "1"))
}

pub(crate) fn begin(window: WindowId) {
    ACTIVE.with(|active| *active.borrow_mut() = Some(Observation {
        window, entries: Vec::new(), next_ordinal: 1,
        dropped_chat_render_events: 0, dropped_logo_render_events: 0,
        dropped_logo_owner_events: 0,
        dropped_animation_evaluation_events: 0,
    }));
}

fn with_observation(window: WindowId, f: impl FnOnce(&mut Observation)) {
    ACTIVE.with(|active| {
        if let Some(observation) = active.borrow_mut().as_mut()
            .filter(|observation| observation.window == window) { f(observation); }
    });
}

fn ensure_entry(observation: &mut Observation, kind: Kind, entity: EntityId,
                owner: Option<EntityId>) -> Option<usize> {
    if let Some(index) = observation.entries.iter().position(|entry|
        entry.kind == kind && entry.entity == entity && entry.owner == owner) {
        return Some(index);
    }
    if observation.entries.len() >= MAX_ENTRIES { return None; }
    let ordinal = observation.next_ordinal;
    observation.next_ordinal += 1;
    observation.entries.push(Entry { kind, entity, owner, ordinal, renders: 0,
        image_present_renders: 0, animation_requested_renders: 0, animation_enabled_renders: 0,
        animation_evaluations: 0, working_logo_renders: 0 });
    Some(observation.entries.len() - 1)
}

pub(crate) fn chat_render(window: WindowId, pane: EntityId) {
    with_observation(window, |observation| {
        if let Some(index) = ensure_entry(observation, Kind::Pane, pane, None) {
            observation.entries[index].renders += 1;
        } else { observation.dropped_chat_render_events += 1; }
    });
}

pub(crate) fn working_logo_render(
    window: WindowId,
    pane: EntityId,
    logo: EntityId,
    animation_requested: bool,
    animation_enabled: bool,
    image_present: bool,
) {
    with_observation(window, |observation| {
        let pane_index = ensure_entry(observation, Kind::Pane, pane, None);
        let logo_index = ensure_entry(observation, Kind::Logo, logo, Some(pane));
        if let Some(index) = pane_index { observation.entries[index].working_logo_renders += 1; }
        else { observation.dropped_logo_owner_events += 1; }
        if let Some(index) = logo_index {
            let entry = &mut observation.entries[index];
            entry.renders += 1;
            if image_present { entry.image_present_renders += 1; }
            if animation_requested { entry.animation_requested_renders += 1; }
            if animation_enabled { entry.animation_enabled_renders += 1; }
        }
        else { observation.dropped_logo_render_events += 1; }
    });
}

pub(crate) fn animation_evaluation(window: WindowId, pane: EntityId, logo: EntityId) {
    with_observation(window, |observation| {
        if let Some(index) = ensure_entry(observation, Kind::Logo, logo, Some(pane)) {
            observation.entries[index].animation_evaluations += 1;
        } else { observation.dropped_animation_evaluation_events += 1; }
    });
}

/// Ends and returns this window's observation. Entity/window IDs never leave
/// the process; output uses only observation-local anonymous ordinals.
pub(crate) fn finish(window: WindowId) -> Value {
    ACTIVE.with(|active| {
        let mut active = active.borrow_mut();
        if active.as_ref().is_some_and(|observation| observation.window != window) {
            return json!({"status":"window-mismatch"});
        }
        let Some(observation) = active.take() else { return json!({"status":"inactive"}); };
        let chat_panes: Vec<Value> = observation.entries.iter().filter(|entry| entry.kind == Kind::Pane)
            .map(|entry| json!({"ordinal":entry.ordinal,"renderCount":entry.renders,
                "workingLogoRenderCount":entry.working_logo_renders})).collect();
        let logos: Vec<Value> = observation.entries.iter().filter(|entry| entry.kind == Kind::Logo)
            .map(|entry| json!({"ordinal":entry.ordinal,"totalRenderCount":entry.renders,
                "imagePresentRenderCount":entry.image_present_renders,
                "animationRequestedRenderCount":entry.animation_requested_renders,
                "animationEnabledRenderCount":entry.animation_enabled_renders,
                "animationEvaluationCount":entry.animation_evaluations,
                "ownerPaneOrdinal":observation.entries.iter().find(|pane|
                    pane.kind == Kind::Pane && Some(pane.entity) == entry.owner).map(|pane| pane.ordinal)})).collect();
        json!({"scope":"thread-local GPUI callback observer; selected process window only; anonymous entity ordinals; bounded to 64 combined entries",
            "countSemantics":"ChatPane and logo render callbacks plus animation transformation evaluations; animationRequestedRenderCount already includes motion policy gates. Notifications and invalidation causality are not counted. The static-logo override affects all windows in this process, while these counters observe only the selected window.",
            "chatPanes":chat_panes,"workingLogos":logos,
            "droppedChatRenderEvents":observation.dropped_chat_render_events,
            "droppedLogoRenderEvents":observation.dropped_logo_render_events,
            "droppedLogoOwnerEvents":observation.dropped_logo_owner_events,
            "droppedAnimationEvaluationEvents":observation.dropped_animation_evaluation_events})
    })
}
