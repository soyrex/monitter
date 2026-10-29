//! Retained decorative children. Animation invalidation stays in each child
//! entity, rather than repainting the desktop or polling agent state.
use crate::{
    activity_sparkles::{SparkleField, SparkleLayout, TabAnchor},
    design::Palette,
    native_motion,
};
use gpui_kit::{App, AppContext as _, Entity, Global, SharedString, Window};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::time::Duration;

const EXIT_CLEANUP_GRACE: Duration = Duration::from_millis(50);

#[derive(Clone, PartialEq, Eq)]
struct Presentation {
    running: HashSet<String>,
    titles: HashMap<String, String>,
    palette: Palette,
    animation: native_motion::ActivityAnimation,
    motion: bool,
}
impl Presentation {
    fn from_snapshot(snapshot: &Value, palette: Palette) -> Self {
        Self {
            running: snapshot["tasks"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|task| task["status"] == "running")
                .filter_map(|task| task["id"].as_str().map(str::to_owned))
                .collect(),
            titles: snapshot["tasks"].as_array().into_iter().flatten()
                .filter_map(|task| Some((task["id"].as_str()?.to_owned(), task["title"].as_str()?.to_owned())))
                .collect(),
            palette,
            animation: native_motion::ActivityAnimation::from_snapshot(
                snapshot["settings"]["activityAnimation"].as_str(),
            ),
            motion: native_motion::motion_allowed(),
        }
    }
}
struct Entry {
    task: String,
    layout: SparkleLayout,
    mirror: bool,
    field: Entity<SparkleField>,
    generation: u64,
    surface_visible: bool,
    anchor_key: Option<String>,
}
#[derive(Default)]
struct Effects {
    // Pointer bookkeeping is not visual state: updating it must not notify
    // every retained pane that reads the activity presentation.
    source: std::cell::RefCell<std::rc::Weak<Value>>,
    obscured: HashSet<String>,
    presentation: Option<Presentation>,
    naming: HashSet<String>,
    pending_titles: HashMap<String, String>,
    fields: HashMap<String, Entry>,
    anchors: HashMap<String, std::rc::Rc<TabAnchor>>,
}
impl Global for Effects {}

pub fn sync(snapshot: &Value, palette: Palette, cx: &mut App) -> bool {
    if cx.try_global::<Effects>().is_none() {
        cx.set_global(Effects::default());
    }
    cx.global::<Effects>().source.replace(std::rc::Weak::new());
    let next = Presentation::from_snapshot(snapshot, palette);
    let completed: Vec<_> = cx.global::<Effects>().pending_titles.iter()
        .filter(|(task, title)| next.titles.get(*task) == Some(*title))
        .map(|(task, _)| task.clone()).collect();
    for task in &completed {
        let effects = cx.global_mut::<Effects>();
        effects.pending_titles.remove(task);
        effects.naming.remove(task);
    }
    if completed.is_empty() && cx.global::<Effects>().presentation.as_ref() == Some(&next) {
        return false;
    }
    cx.global_mut::<Effects>().presentation = Some(next.clone());
    update_existing_fields(&next, cx);
    if !completed.is_empty() { cx.refresh_windows(); }
    true
}

/// Immutable snapshots let ordinary redraws skip task-map construction. Local
/// motion and palette changes remain independent invalidation inputs.
pub fn sync_cached(snapshot: &std::rc::Rc<Value>, palette: Palette, cx: &mut App) -> bool {
    let unchanged = cx.try_global::<Effects>().is_some_and(|effects| {
        effects.presentation.as_ref().is_some_and(|presentation| {
            if presentation.palette != palette || presentation.motion != native_motion::motion_allowed() {
                return false;
            }
            if effects.source.borrow().ptr_eq(&std::rc::Rc::downgrade(snapshot)) { return true; }
            if presentation.animation != native_motion::ActivityAnimation::from_snapshot(
                snapshot["settings"]["activityAnimation"].as_str()) {
                return false;
            }
            let mut titles = 0;
            let mut running = 0;
            for task in snapshot["tasks"].as_array().into_iter().flatten() {
                let Some(id) = task["id"].as_str() else { continue; };
                if let Some(title) = task["title"].as_str() {
                    titles += 1;
                    if presentation.titles.get(id).map(String::as_str) != Some(title) { return false; }
                }
                let active = task["status"] == "running";
                running += usize::from(active);
                if presentation.running.contains(id) != active { return false; }
            }
            titles == presentation.titles.len() && running == presentation.running.len()
        })
    });
    if unchanged {
        cx.global::<Effects>().source.replace(std::rc::Rc::downgrade(snapshot));
        return false;
    }
    let changed = sync(snapshot, palette, cx);
    cx.global::<Effects>().source.replace(std::rc::Rc::downgrade(snapshot));
    changed
}

/// Local motion settings can change without a new backend snapshot.
pub fn sync_motion(cx: &mut App) {
    sync_motion_value(native_motion::motion_allowed(), cx);
}

fn sync_motion_value(motion: bool, cx: &mut App) {
    if cx.try_global::<Effects>().is_none() {
        return;
    }
    let next = {
        let effects = cx.global_mut::<Effects>();
        let Some(presentation) = effects.presentation.as_mut() else {
            return;
        };
        if presentation.motion == motion {
            return;
        }
        presentation.motion = motion;
        presentation.clone()
    };
    update_existing_fields(&next, cx);
}

/// Reserve a local naming operation without changing authoritative task status.
pub fn begin_naming(task: &str, cx: &mut App) -> bool {
    if cx.try_global::<Effects>().is_none() {
        cx.set_global(Effects::default());
    }
    if !cx.global_mut::<Effects>().naming.insert(task.to_owned()) {
        return false;
    }
    if let Some(presentation) = cx.global::<Effects>().presentation.clone() {
        update_existing_fields(&presentation, cx);
    }
    cx.refresh_windows();
    true
}

pub fn is_naming(task: &str, cx: &App) -> bool {
    cx.try_global::<Effects>().is_some_and(|effects| effects.naming.contains(task))
}

/// Keep feedback until the authoritative snapshot displays the generated title.
pub fn complete_naming(task: &str, title: &str, cx: &mut App) {
    let already_visible = cx.try_global::<Effects>().and_then(|effects| effects.presentation.as_ref())
        .and_then(|presentation| presentation.titles.get(task)).is_some_and(|current| current == title);
    if already_visible { end_naming(task, cx); }
    else if is_naming(task, cx) { cx.global_mut::<Effects>().pending_titles.insert(task.to_owned(), title.to_owned()); }
}

pub fn end_naming(task: &str, cx: &mut App) {
    let Some(effects) = cx.try_global::<Effects>() else { return; };
    if !effects.naming.contains(task) {
        return;
    }
    cx.global_mut::<Effects>().naming.remove(task);
    cx.global_mut::<Effects>().pending_titles.remove(task);
    if let Some(presentation) = cx.global::<Effects>().presentation.clone() {
        update_existing_fields(&presentation, cx);
    }
    cx.refresh_windows();
}

fn effect_is_desired(
    presentation: &Presentation,
    naming: &HashSet<String>,
    task: &str,
    layout: SparkleLayout,
) -> bool {
    (naming.contains(task) && matches!(layout, SparkleLayout::Tab | SparkleLayout::Header))
        || (presentation.running.contains(task)
            && (layout == SparkleLayout::Pane
                || presentation.animation == native_motion::ActivityAnimation::Sparkles))
}

fn update_existing_fields(presentation: &Presentation, cx: &mut App) {
    let fields = cx
        .global::<Effects>()
        .fields
        .iter()
        .map(|(key, entry)| {
            (
                key.clone(),
                entry.task.clone(),
                entry.layout,
                entry.mirror,
                entry.field.clone(),
                entry.generation,
                entry.surface_visible,
            )
        })
        .collect::<Vec<_>>();
    for (key, task, layout, mirror, field, prior_generation, surface_visible) in fields {
        let desired = effect_is_desired(presentation, &cx.global::<Effects>().naming, &task, layout);
        let (change, generation, target_visible) = field.update(cx, |field, cx| {
            let changed =
                field.set_config(layout, mirror, presentation.palette, presentation.motion && surface_visible);
            let animation_changed = field.set_animation(presentation.animation);
            let transition = field.set_visible(desired);
            if changed || animation_changed || transition.is_some() {
                cx.notify();
            }
            (
                transition,
                field.visibility_generation(),
                field.target_visible(),
            )
        });
        if let Some(entry) = cx.global_mut::<Effects>().fields.get_mut(&key) {
            entry.generation = generation;
        }
        if !target_visible && !presentation.motion {
            remove_entry_if_current(&key, generation, cx);
        } else if let Some(change) = change {
            if let Some(delay) = change.cleanup_after {
                schedule_cleanup(key, generation, delay, cx);
            }
        } else if generation != prior_generation && !target_visible {
            // Switching reduced motion on resolves an in-flight exit instantly.
            remove_entry_if_current(&key, generation, cx);
        }
    }
}

fn schedule_cleanup(key: String, generation: u64, delay: Duration, cx: &mut App) {
    let delay = if delay.is_zero() {
        Duration::ZERO
    } else {
        delay + EXIT_CLEANUP_GRACE
    };
    cx.spawn(async move |cx| {
        cx.background_executor().timer(delay).await;
        cx.update(move |cx| remove_entry_if_current(&key, generation, cx));
    })
    .detach();
}

fn can_remove_entry(entry_generation: u64, timer_generation: u64, target_visible: bool) -> bool {
    entry_generation == timer_generation && !target_visible
}

fn remove_entry_if_current(key: &str, generation: u64, cx: &mut App) {
    let current = cx.try_global::<Effects>().and_then(|effects| {
        effects
            .fields
            .get(key)
            .map(|entry| (entry.generation, entry.field.clone()))
    });
    let Some((entry_generation, field)) = current else {
        return;
    };
    if can_remove_entry(
        entry_generation,
        generation,
        field.read(cx).target_visible(),
    ) {
        let anchor_key = cx
            .global_mut::<Effects>()
            .fields
            .remove(key)
            .and_then(|entry| entry.anchor_key);
        if let Some(anchor_key) = anchor_key {
            let effects = cx.global_mut::<Effects>();
            if !effects
                .fields
                .values()
                .any(|entry| entry.anchor_key.as_deref() == Some(anchor_key.as_str()))
            {
                effects.anchors.remove(&anchor_key);
            }
        }
    }
}

/// Modal content must paint above retained pane decorations, including their deferred layer.
pub(crate) fn set_pane_obscured(task: &str, obscured: bool, cx: &mut App) {
    let Some(effects) = cx.try_global::<Effects>() else { return; };
    if effects.obscured.contains(task) == obscured { return; }
    if obscured { cx.global_mut::<Effects>().obscured.insert(task.to_owned()); }
    else { cx.global_mut::<Effects>().obscured.remove(task); }
    let fields: Vec<_> = cx.global::<Effects>().fields.values()
        .filter(|entry| entry.task == task && !matches!(entry.layout, SparkleLayout::Tab | SparkleLayout::Compact))
        .map(|entry| entry.field.clone()).collect();
    for field in fields { field.update(cx, |field, cx| { if field.set_obscured(obscured) { cx.notify(); } }); }
}

/// Park pane clocks while the dock omits the pane. Tab/sidebar markers keep
/// their own visibility; hiding a conversation must not stop a visible marker.
pub fn set_pane_visible(task: &str, visible: bool, cx: &mut App) {
    let Some(effects) = cx.try_global::<Effects>() else { return; };
    let Some(presentation) = effects.presentation.clone() else { return; };
    let fields: Vec<_> = effects.fields.iter().filter(|(_, entry)| {
        entry.task == task && !matches!(entry.layout, SparkleLayout::Tab | SparkleLayout::Compact)
    }).map(|(key, entry)| (key.clone(), entry.field.clone(), entry.layout, entry.mirror)).collect();
    for (key, field, layout, mirror) in fields {
        if let Some(entry) = cx.global_mut::<Effects>().fields.get_mut(&key) {
            entry.surface_visible = visible;
        }
        field.update(cx, |field, cx| {
            if field.set_config(layout, mirror, presentation.palette, presentation.motion && visible) {
                cx.notify();
            }
        });
    }
}

/// Running status creates the selected pane effect; local naming adds title stars.
/// Compact sidebar rows retain their small activity marker.
pub fn for_task(
    task: &str,
    surface: &str,
    layout: SparkleLayout,
    mirror: bool,
    cx: &mut App,
) -> Option<Entity<SparkleField>> {
    for_task_scoped(task, surface, layout, mirror, None, cx)
}

/// Retain a window/task-scoped field. Tab and Header calls for the same task
/// share one anchor clock and the selected tab's world-space bounds.
pub fn for_task_in_window(
    task: &str,
    surface: &str,
    layout: SparkleLayout,
    mirror: bool,
    window: &Window,
    cx: &mut App,
) -> Option<Entity<SparkleField>> {
    let window_id = window.window_handle().window_id().as_u64();
    for_task_scoped(task, surface, layout, mirror, Some(window_id), cx)
}

fn for_task_scoped(
    task: &str,
    surface: &str,
    layout: SparkleLayout,
    mirror: bool,
    window_id: Option<u64>,
    cx: &mut App,
) -> Option<Entity<SparkleField>> {
    let effects = cx.try_global::<Effects>()?;
    let presentation = effects.presentation.as_ref()?;
    let anchor_key = window_id.map(|id| format!("{id}:{task}"));
    let key = anchor_key.as_ref().map_or_else(
        || format!("{surface}:{task}"),
        |anchor_key| format!("{anchor_key}:{surface}"),
    );
    if let Some(entry) = effects.fields.get(&key) {
        return Some(entry.field.clone());
    }
    if !effect_is_desired(presentation, &effects.naming, task, layout) {
        return None;
    }
    let (palette, motion, animation) =
        (presentation.palette, presentation.motion, presentation.animation);
    let anchor = if matches!(layout, SparkleLayout::Tab | SparkleLayout::Header | SparkleLayout::HeaderWash | SparkleLayout::DockWash) {
        anchor_key.as_ref().map(|key| {
            cx.global_mut::<Effects>()
                .anchors
                .entry(key.clone())
                .or_insert_with(|| std::rc::Rc::new(TabAnchor::new()))
                .clone()
        })
    } else {
        None
    };
    let id = SharedString::from(format!("activity-{key}"));
    let obscured = cx.global::<Effects>().obscured.contains(task)
        && !matches!(layout, SparkleLayout::Tab | SparkleLayout::Compact);
    let field = cx.new(|_| {
        let mut field = SparkleField::new(
            id,
            layout,
            mirror,
            palette,
            motion,
        );
        field.set_anchor(anchor);
        field.set_obscured(obscured);
        field.set_animation(animation);
        field
    });
    let change = field.update(cx, |field, cx| {
        let change = field.set_visible(true);
        if change.is_some() {
            cx.notify();
        }
        change
    });
    let generation = field.read(cx).visibility_generation();
    cx.global_mut::<Effects>().fields.insert(
        key.clone(),
        Entry {
            task: task.to_owned(),
            layout,
            mirror,
            field: field.clone(),
            generation,
            surface_visible: true,
            anchor_key: anchor_key
                .filter(|_| matches!(layout, SparkleLayout::Tab | SparkleLayout::Header | SparkleLayout::HeaderWash | SparkleLayout::DockWash)),
        },
    );
    if let Some(change) = change {
        if let Some(delay) = change.cleanup_after {
            schedule_cleanup(key, generation, delay, cx);
        }
    }
    Some(field)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[gpui_kit::test]
    fn cached_activity_preserves_maps_until_title_status_palette_or_motion_changes(cx: &mut gpui_kit::TestAppContext) {
        let palette = crate::design::palette(&serde_json::json!({}), gpui_kit::WindowAppearance::Dark);
        cx.update(|cx| {
            let source = std::rc::Rc::new(serde_json::json!({"tasks":[{"id":"task","title":"Old","status":"running","updatedAt":1}],"settings":{"activityAnimation":"sparkles"}}));
            assert!(sync_cached(&source, palette, cx));
            let title = cx.global::<Effects>().presentation.as_ref().unwrap().titles["task"].as_ptr();
            assert!(!sync_cached(&source, palette, cx));
            let mut next = source.as_ref().clone();
            next["tasks"][0]["updatedAt"] = serde_json::json!(2);
            assert!(!sync_cached(&std::rc::Rc::new(next.clone()), palette, cx));
            assert_eq!(title, cx.global::<Effects>().presentation.as_ref().unwrap().titles["task"].as_ptr());
            begin_naming("task", cx);
            complete_naming("task", "New", cx);
            next["tasks"][0]["title"] = serde_json::json!("New");
            assert!(sync_cached(&std::rc::Rc::new(next.clone()), palette, cx));
            assert!(!is_naming("task", cx));
            next["tasks"][0]["status"] = serde_json::json!("completed");
            let next = std::rc::Rc::new(next);
            assert!(sync_cached(&next, palette, cx));
            let mut changed_palette = palette;
            changed_palette.accent ^= 1;
            assert!(sync_cached(&next, changed_palette, cx));
            sync_motion_value(!native_motion::motion_allowed(), cx);
            assert!(sync_cached(&next, changed_palette, cx));
        });
    }
    #[gpui_kit::test]
    fn retained_fields_fade_out_reverse_and_cleanup_only_after_current_exit(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        let palette =
            crate::design::palette(&serde_json::json!({}), gpui_kit::WindowAppearance::Dark);
        cx.update(|cx| {
            let mut snapshot = serde_json::json!({"tasks":[{"id":"actual","status":"running"}],"settings":{"activityAnimation":"sparkles"}});
            sync(&snapshot, palette, cx);
            let field = for_task("actual", "pane", SparkleLayout::Pane, false, cx).unwrap();
            let retained = for_task("actual", "pane", SparkleLayout::Pane, false, cx).unwrap();
            assert_eq!(field.entity_id(), retained.entity_id());
            snapshot["settings"]["activityAnimation"] = serde_json::json!("liquid");
            sync(&snapshot, palette, cx);
            assert_eq!(for_task("actual", "pane", SparkleLayout::Pane, false, cx).unwrap().entity_id(), field.entity_id());
            sync_motion_value(false, cx);
            assert!(!cx.global::<Effects>().presentation.as_ref().unwrap().motion);
            assert!(!field.update(cx, |field, _| field.set_config(SparkleLayout::Pane, false, palette, false)));
            sync_motion_value(true, cx);
            assert!(!field.update(cx, |field, _| field.set_config(SparkleLayout::Pane, false, palette, true)));
            set_pane_visible("actual", false, cx);
            assert!(field.read(cx).target_visible());
            assert!(!field.update(cx, |field, _| field.set_config(SparkleLayout::Pane, false, palette, false)));
            // A later global motion update cannot wake a hidden pane clock.
            sync_motion_value(false, cx);
            sync_motion_value(true, cx);
            assert!(!field.update(cx, |field, _| field.set_config(SparkleLayout::Pane, false, palette, false)));
            set_pane_visible("actual", true, cx);
            assert!(!field.update(cx, |field, _| field.set_config(SparkleLayout::Pane, false, palette, true)));
            snapshot["tasks"][0]["status"] = serde_json::json!("completed");
            sync(&snapshot, palette, cx);
            assert!(!field.read(cx).target_visible());
            let exiting_generation = field.read(cx).visibility_generation();
            assert_eq!(for_task("actual", "pane", SparkleLayout::Pane, false, cx).unwrap().entity_id(), field.entity_id());

            // A quick restart reverses the exit and fences its old cleanup.
            snapshot["tasks"][0]["status"] = serde_json::json!("running");
            sync(&snapshot, palette, cx);
            assert!(field.read(cx).target_visible());
            assert_ne!(field.read(cx).visibility_generation(), exiting_generation);
        });

        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(600));
        cx.run_until_parked();
        cx.update(|cx| assert_eq!(cx.global::<Effects>().fields.len(), 1));

        cx.update(|cx| {
            let snapshot = serde_json::json!({"tasks":[{"id":"actual","status":"completed"}],"settings":{"activityAnimation":"sparkles"}});
            sync(&snapshot, palette, cx);
            let field = for_task("actual", "pane", SparkleLayout::Pane, false, cx).unwrap();
            assert!(!field.read(cx).target_visible());
        });
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(600));
        cx.run_until_parked();
        cx.update(|cx| {
            assert!(cx.global::<Effects>().fields.is_empty());
            assert!(for_task("actual", "pane", SparkleLayout::Pane, false, cx).is_none());
        });
    }

    #[test]
    fn only_actual_running_tasks_get_effects_and_alternative_preference_is_preserved() {
        let palette =
            crate::design::palette(&serde_json::json!({}), gpui_kit::WindowAppearance::Dark);
        let state = Presentation::from_snapshot(
            &serde_json::json!({"tasks":[{"id":"running","status":"running"},{"id":"selected","status":"completed"},{"id":"queued","status":"queued"}],"settings":{"activityAnimation":"matrix"}}),
            palette,
        );
        assert_eq!(state.running, HashSet::from(["running".to_owned()]));
        assert_eq!(state.animation, native_motion::ActivityAnimation::Matrix);
        let naming = HashSet::new();
        assert!(effect_is_desired(&state, &naming, "running", SparkleLayout::Pane));
        assert!(!effect_is_desired(&state, &naming, "running", SparkleLayout::Tab));
        assert!(!effect_is_desired(&state, &naming, "running", SparkleLayout::Compact));
        assert!(!effect_is_desired(&state, &naming, "queued", SparkleLayout::Pane));
    }

    #[gpui_kit::test]
    fn naming_title_effects_survive_snapshot_sync_without_pane_activity(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        let palette = crate::design::palette(&serde_json::json!({}), gpui_kit::WindowAppearance::Dark);
        cx.update(|cx| {
            let mut snapshot = serde_json::json!({"tasks":[{"id":"named","status":"completed"}],"settings":{"activityAnimation":"matrix"}});
            sync(&snapshot, palette, cx);
            assert!(begin_naming("named", cx));
            assert!(!begin_naming("named", cx));
            let title = for_task("named", "header", SparkleLayout::Header, true, cx).unwrap();
            assert!(for_task("named", "tab", SparkleLayout::Tab, false, cx).is_some());
            for layout in [SparkleLayout::Pane, SparkleLayout::HeaderWash, SparkleLayout::DockWash, SparkleLayout::Compact] {
                assert!(for_task("named", "excluded", layout, false, cx).is_none());
            }
            snapshot["tasks"][0]["status"] = serde_json::json!("idle");
            sync(&snapshot, palette, cx);
            snapshot["settings"]["activityAnimation"] = serde_json::json!("sparkles");
            assert!(sync(&snapshot, palette, cx));
            assert!(cx.global::<Effects>().naming.contains("named"));
            assert!(title.read(cx).target_visible());
            assert!(cx.global::<Effects>().presentation.as_ref().unwrap().running.is_empty());
            sync_motion_value(false, cx);
            assert!(!title.update(cx, |field, _| field.set_config(SparkleLayout::Header, true, palette, false)));
            end_naming("named", cx);
            assert!(!title.read(cx).target_visible());
            assert!(!cx.global::<Effects>().naming.contains("named"));
            assert!(begin_naming("named", cx));
            let restarted = for_task("named", "header", SparkleLayout::Header, true, cx).unwrap();
            assert!(restarted.read(cx).target_visible());
            assert_ne!(restarted.entity_id(), title.entity_id());
            assert!(!title.read(cx).target_visible());
            end_naming("named", cx);
        });
    }

    #[gpui_kit::test]
    fn naming_waits_for_authoritative_title(cx: &mut gpui_kit::TestAppContext) {
        let palette = crate::design::palette(&serde_json::json!({}), gpui_kit::WindowAppearance::Dark);
        cx.update(|cx| {
            let mut snapshot = serde_json::json!({"tasks":[{"id":"named","title":"Old","status":"completed"}]});
            sync(&snapshot, palette, cx);
            begin_naming("named", cx);
            complete_naming("named", "New", cx);
            sync(&snapshot, palette, cx);
            assert!(is_naming("named", cx));
            snapshot["tasks"][0]["title"] = serde_json::json!("New");
            sync(&snapshot, palette, cx);
            assert!(!is_naming("named", cx));
            assert!(cx.global::<Effects>().pending_titles.is_empty());
            begin_naming("named", cx);
            complete_naming("named", "New", cx);
            assert!(!is_naming("named", cx));
            begin_naming("named", cx);
            complete_naming("named", "Other", cx);
            end_naming("named", cx);
            assert!(cx.global::<Effects>().pending_titles.is_empty());
        });
    }

    #[test]
    fn cleanup_is_fenced_by_current_generation_and_hidden_target() {
        assert!(can_remove_entry(9, 9, false));
        assert!(!can_remove_entry(10, 9, false));
        assert!(!can_remove_entry(9, 9, true));
    }
}
