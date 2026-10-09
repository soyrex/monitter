//! UI-only projection of resident internal tasks and saved native panes.
//!
//! The backend snapshot remains authoritative and untouched. These helpers
//! remove only tasks owned by agents explicitly marked `internal: true`.

use gpui_kit::base::dock::{DockAreaState, DockState, PanelInfo, PanelState};
use serde_json::Value;
use std::collections::HashSet;

const NATIVE_PANE: &str = "MonitterNativePane";

/// IDs of tasks owned by agents explicitly marked internal in the snapshot.
/// Tasks whose agent record is missing are deliberately retained.
pub fn internal_task_ids(snapshot: &Value) -> HashSet<String> {
    let internal_agents = internal_agent_ids(&snapshot);

    snapshot["tasks"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|task| {
            task["agentId"]
                .as_str()
                .is_some_and(|agent_id| internal_agents.contains(agent_id))
        })
        .filter_map(|task| task["id"].as_str().filter(|id| !id.is_empty()))
        .map(str::to_owned)
        .collect()
}

fn internal_agent_ids(snapshot: &Value) -> HashSet<String> {
    snapshot["agents"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|agent| agent["internal"].as_bool() == Some(true))
        .filter_map(|agent| agent["id"].as_str().filter(|id| !id.is_empty()))
        .map(str::to_owned)
        .collect()
}

/// Returns a renderer-only snapshot with internal task rows removed.
/// Agent records, settings, and every other snapshot field remain unchanged.
pub fn project_snapshot(mut snapshot: Value) -> Value {
    let internal_agents = internal_agent_ids(&snapshot);
    if internal_agents.is_empty() {
        return snapshot;
    }

    if let Some(tasks) = snapshot["tasks"].as_array_mut() {
        tasks.retain(|task| {
            !task["agentId"]
                .as_str()
                .is_some_and(|id| internal_agents.contains(id))
        });
    }
    snapshot
}

/// Removes saved `chat:<task-id>` pane leaves for internal tasks while
/// preserving the rest of a versioned native dock layout.
///
/// `saved_ids` is the same preflight used by DockWorkspace restore, so invalid
/// versions, malformed pane identities, excessive depth, and duplicate IDs
/// stay errors. If no internal chat pane is present, the input is returned
/// byte-for-byte as a JSON value (including unknown fields).
pub fn prune_layout(layout: &Value, internal_task_ids: &HashSet<String>) -> Result<Value, String> {
    if internal_task_ids.is_empty() {
        return Ok(layout.clone());
    }

    let ids = crate::docking::DockWorkspace::saved_ids(layout)?;
    let has_hidden_chat = ids.iter().any(|id| {
        id.strip_prefix("chat:")
            .is_some_and(|task_id| internal_task_ids.contains(task_id))
    });
    if !has_hidden_chat {
        return Ok(layout.clone());
    }

    let mut state: DockAreaState = serde_json::from_value(layout.clone())
        .map_err(|error| format!("Invalid native workspace layout: {error}"))?;

    if prune_panel(&mut state.center, internal_task_ids) == Prune::Removed {
        state.center = empty_tabs();
    }
    state.left_dock = prune_dock(state.left_dock.take(), internal_task_ids);
    state.right_dock = prune_dock(state.right_dock.take(), internal_task_ids);
    state.bottom_dock = prune_dock(state.bottom_dock.take(), internal_task_ids);

    serde_json::to_value(state).map_err(|error| error.to_string())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Prune {
    Kept,
    Removed,
}

fn pane_task_id(panel: &PanelState) -> Option<&str> {
    if panel.panel_name != NATIVE_PANE {
        return None;
    }
    match &panel.info {
        PanelInfo::Panel(payload) => payload["id"].as_str(),
        _ => None,
    }
}

fn empty_tabs() -> PanelState {
    PanelState {
        panel_name: "TabPanel".into(),
        children: Vec::new(),
        info: PanelInfo::tabs(0),
    }
}

fn prune_dock(dock: Option<DockState>, task_ids: &HashSet<String>) -> Option<DockState> {
    let dock = dock?;
    let mut panel = dock.panel().clone();
    if prune_panel(&mut panel, task_ids) == Prune::Removed {
        None
    } else {
        Some(DockState::new(
            panel,
            dock.placement(),
            dock.size(),
            dock.open(),
        ))
    }
}

fn prune_panel(panel: &mut PanelState, task_ids: &HashSet<String>) -> Prune {
    if let Some(id) = pane_task_id(panel)
        && let Some(task_id) = id.strip_prefix("chat:")
        && task_ids.contains(task_id)
    {
        return Prune::Removed;
    }

    let old_children = std::mem::take(&mut panel.children);
    if old_children.is_empty() {
        return Prune::Kept;
    }

    let old_active = match &panel.info {
        PanelInfo::Tabs { active_index } => Some(*active_index),
        _ => None,
    };
    let old_stack = match &panel.info {
        PanelInfo::Stack { sizes, axis } => Some((*axis, sizes.clone())),
        _ => None,
    };

    let mut kept = Vec::with_capacity(old_children.len());
    let mut kept_indices = Vec::with_capacity(old_children.len());
    for (index, mut child) in old_children.into_iter().enumerate() {
        if prune_panel(&mut child, task_ids) == Prune::Kept {
            kept.push(child);
            kept_indices.push(index);
        }
    }

    if kept.is_empty() {
        return Prune::Removed;
    }

    if let Some(active) = old_active {
        let selected_old_index = kept_indices
            .iter()
            .enumerate()
            .min_by_key(|(_, old_index)| (old_index.abs_diff(active), **old_index))
            .map(|(new_index, _)| new_index)
            .unwrap_or(0);
        panel.info = PanelInfo::tabs(selected_old_index);
    } else if let Some((axis, sizes)) = old_stack {
        let retained_sizes = kept_indices
            .iter()
            .filter_map(|index| sizes.get(*index).cloned())
            .collect();
        panel.info = PanelInfo::Stack {
            sizes: retained_sizes,
            axis,
        };
    }

    panel.children = kept;
    Prune::Kept
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Axis, px};
    use gpui_kit::base::dock::{DockPlacement, DockState};
    use serde_json::json;

    fn pane(id: &str) -> PanelState {
        PanelState {
            panel_name: NATIVE_PANE.into(),
            children: vec![],
            info: PanelInfo::panel(json!({"id": id})),
        }
    }

    fn tabs(children: Vec<PanelState>, active_index: usize) -> PanelState {
        PanelState {
            panel_name: "TabPanel".into(),
            children,
            info: PanelInfo::tabs(active_index),
        }
    }

    #[test]
    fn projection_hides_only_tasks_with_explicit_internal_owner() {
        let snapshot = json!({
            "agents": [
                {"id":"admin", "internal":true},
                {"id":"user", "internal":false}
            ],
            "tasks": [
                {"id":"admin-0", "agentId":"admin", "title":"Monitter Admin resident task"},
                {"id":"admin-1", "agentId":"admin", "title":"Monitter Admin resident task 1"},
                {"id":"user-title-match", "agentId":"user", "title":"Monitter Admin resident task"},
                {"id":"orphan-title-match", "agentId":"removed", "title":"Monitter Admin resident task"}
            ],
            "settings": {"keep": true}
        });

        assert_eq!(
            internal_task_ids(&snapshot),
            HashSet::from(["admin-0".to_string(), "admin-1".to_string()])
        );
        let projected = project_snapshot(snapshot.clone());
        assert_eq!(projected["agents"], snapshot["agents"]);
        assert_eq!(projected["settings"], snapshot["settings"]);
        let ids: Vec<_> = projected["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|task| task["id"].as_str())
            .collect();
        assert_eq!(ids, ["user-title-match", "orphan-title-match"]);
    }

    #[test]
    fn layout_pruning_keeps_other_panes_and_maps_selection_and_split_sizes() {
        let center = PanelState {
            panel_name: "StackPanel".into(),
            children: vec![
                tabs(vec![pane("chat:hidden-a"), pane("chat:user")], 0),
                tabs(vec![pane("chat:hidden-b")], 0),
                tabs(vec![pane("route:projects")], 0),
            ],
            info: PanelInfo::stack(vec![px(120.), px(220.), px(340.)], Axis::Horizontal),
        };
        let left = DockState::new(
            tabs(vec![pane("chat:hidden-c"), pane("route:archives")], 0),
            DockPlacement::Left,
            px(240.),
            true,
        );
        let layout = serde_json::to_value(DockAreaState {
            version: Some(1),
            center,
            left_dock: Some(left),
            right_dock: None,
            bottom_dock: None,
        })
        .unwrap();
        let hidden = HashSet::from([
            "hidden-a".to_string(),
            "hidden-b".to_string(),
            "hidden-c".to_string(),
        ]);

        let pruned = prune_layout(&layout, &hidden).unwrap();
        let state: DockAreaState = serde_json::from_value(pruned).unwrap();
        assert_eq!(state.version, Some(1));
        assert_eq!(
            crate::docking::DockWorkspace::saved_ids(&serde_json::to_value(&state).unwrap())
                .unwrap(),
            ["chat:user", "route:archives", "route:projects"]
        );
        assert_eq!(state.center.children.len(), 2);
        assert_eq!(
            state.center.info,
            PanelInfo::stack(vec![px(120.), px(340.)], Axis::Horizontal)
        );
        assert_eq!(state.center.children[0].children.len(), 1);
        assert_eq!(state.center.children[0].info, PanelInfo::tabs(0));
        // The old selection at index 0 was removed; the retained old index 1
        // becomes new active index 0.
        assert_eq!(state.center.children[1].children.len(), 1);
        let left = state.left_dock.unwrap();
        assert_eq!(left.panel().children.len(), 1);
        assert_eq!(left.panel().info, PanelInfo::tabs(0));
    }

    #[test]
    fn all_hidden_center_becomes_valid_empty_tabs_and_empty_dock_disappears() {
        let layout = serde_json::to_value(DockAreaState {
            version: Some(1),
            center: tabs(vec![pane("chat:only")], 0),
            left_dock: Some(DockState::new(
                tabs(vec![pane("chat:only-left")], 0),
                DockPlacement::Left,
                px(200.),
                true,
            )),
            right_dock: None,
            bottom_dock: None,
        })
        .unwrap();
        let hidden = HashSet::from(["only".to_string(), "only-left".to_string()]);
        let pruned = prune_layout(&layout, &hidden).unwrap();
        let state: DockAreaState = serde_json::from_value(pruned).unwrap();
        assert_eq!(state.center, empty_tabs());
        assert!(state.left_dock.is_none());
        assert!(
            crate::docking::DockWorkspace::saved_ids(&serde_json::to_value(state).unwrap())
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn no_matching_internal_chat_returns_original_layout_and_invalid_version_errors() {
        let layout = json!({
            "version":1,
            "center": tabs(vec![pane("chat:ordinary")], 0),
            "futureMetadata":{"retain":true}
        });
        let hidden = HashSet::from(["admin".to_string()]);
        assert_eq!(prune_layout(&layout, &hidden).unwrap(), layout);

        let mut invalid = layout;
        invalid["version"] = json!(99);
        assert!(prune_layout(&invalid, &hidden).is_err());
    }
}
