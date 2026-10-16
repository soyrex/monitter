//! UI-only projection of resident internal tasks and saved native panes.
//!
//! The backend snapshot remains authoritative and untouched. These helpers
//! remove only tasks owned by agents explicitly marked `internal: true`.

use gpui_kit::base::dock::{DockAreaState, DockState, PanelInfo, PanelState};
use serde_json::Value;
use std::collections::HashSet;

const NATIVE_PANE: &str = "MonitterNativePane";

// Optional renderer metadata lives beside the native dock schema. Native
// serde restore ignores it, while the workspace preference JSON retains it.
const FOCUSED_PANE: &str = "focusedPaneId";

pub(super) fn remembered_focus(layout: &Value) -> Option<&str> {
    layout.get(FOCUSED_PANE)?.as_str().filter(|id| !id.is_empty())
}
pub(super) fn remember_focus(layout: &mut Value, focus: Option<&str>) {
    let Some(focus) = focus else { return; };
    if crate::docking::DockWorkspace::saved_ids(layout).is_ok_and(|ids| ids.iter().any(|id| id == focus)) {
        layout[FOCUSED_PANE] = Value::String(focus.into());
    }
}
pub(super) fn preserve_focus_metadata(layout: &mut Value, previous: Option<&Value>) {
    if remembered_focus(layout).is_none() {
        remember_focus(layout, previous.and_then(remembered_focus));
    }
}

/// Remembered focus wins; otherwise traverse active tabs in visual layout
/// order before their inactive tabs. Never focus a retained but hidden pane.
pub(super) fn focus_candidates(layout: &Value, remembered: Option<&str>) -> Vec<String> {
    let Ok(valid) = crate::docking::DockWorkspace::saved_ids(layout) else { return Vec::new(); };
    fn walk(panel: &Value, ids: &mut Vec<String>) {
        if panel["panel_name"] == NATIVE_PANE {
            if let Some(id) = panel["info"]["panel"]["id"].as_str() { ids.push(id.into()); }
            return;
        }
        let Some(children) = panel["children"].as_array() else { return; };
        if let Some(active) = panel["info"]["tabs"]["active_index"].as_u64().and_then(|ix| usize::try_from(ix).ok()) {
            if let Some(child) = children.get(active) { walk(child, ids); }
            for (ix, child) in children.iter().enumerate() { if ix != active { walk(child, ids); } }
        } else { for child in children { walk(child, ids); } }
    }
    let mut ids = Vec::new();
    if let Some(focus) = remembered.or_else(|| remembered_focus(layout)).filter(|id| valid.iter().any(|valid| valid.as_str() == *id)) {
        ids.push(focus.into());
    }
    walk(&layout["center"], &mut ids);
    for dock in ["left_dock", "right_dock", "bottom_dock"] {
        // Closed edge docks are restored but are not landing focus targets.
        if layout[dock]["open"].as_bool() == Some(true) { walk(&layout[dock]["panel"], &mut ids); }
    }
    let mut seen = HashSet::new();
    ids.retain(|id| valid.contains(id) && seen.insert(id.clone()));
    ids
}

/// A retained registry can include hidden workspaces; only the live layout
/// proves current ownership. A busy restore must never fall through to Open.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum SidebarTaskDestination {
    Wait,
    Current,
    Workspace(String),
    Open,
}

pub(super) fn sidebar_task_destination(
    pane_id: &str,
    current_layout: &Value,
    workspaces: Option<&monitter_lib::experimental_desktop::NativeWorkspaceSet>,
    ready: bool,
) -> SidebarTaskDestination {
    if !ready { return SidebarTaskDestination::Wait; }
    if crate::docking::DockWorkspace::saved_ids(current_layout)
        .is_ok_and(|ids| ids.iter().any(|id| id == pane_id)) {
        return SidebarTaskDestination::Current;
    }
    if let Some(set) = workspaces {
        for item in &set.items {
            // The current serialized layout is authoritative over its older save.
            if item.id == set.active_id { continue; }
            if item.layout.as_ref().is_some_and(|layout|
                crate::docking::DockWorkspace::saved_ids(layout)
                    .is_ok_and(|ids| ids.iter().any(|id| id == pane_id))) {
                return SidebarTaskDestination::Workspace(item.id.clone());
            }
        }
    }
    SidebarTaskDestination::Open
}

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
    let pane_ids=crate::docking::DockWorkspace::saved_ids(layout)?.into_iter()
        .filter(|id| id.strip_prefix("chat:").or_else(|| crate::git_workspace::task_id(id))
            .is_some_and(|task| internal_task_ids.contains(task))).collect();
    prune_native_pane_ids(layout,&pane_ids)
}

/// Remove exact native pane identities from saved layouts. This leaves backend
/// sessions/tasks and every unrelated native/unknown pane unchanged.
pub(super) fn prune_native_pane_ids(layout: &Value, pane_ids: &HashSet<String>) -> Result<Value,String> {
    if pane_ids.is_empty() {return Ok(layout.clone());}
    let ids=crate::docking::DockWorkspace::saved_ids(layout)?;
    if !ids.iter().any(|id|pane_ids.contains(id)) {return Ok(layout.clone());}
    let mut state: DockAreaState = serde_json::from_value(layout.clone())
        .map_err(|error| format!("Invalid native workspace layout: {error}"))?;

    if prune_panel(&mut state.center, pane_ids) == Prune::Removed {
        state.center = empty_tabs();
    }
    state.left_dock = prune_dock(state.left_dock.take(), pane_ids);
    state.right_dock = prune_dock(state.right_dock.take(), pane_ids);
    state.bottom_dock = prune_dock(state.bottom_dock.take(), pane_ids);

    let mut pruned = serde_json::to_value(state).map_err(|error| error.to_string())?;
    preserve_focus_metadata(&mut pruned, Some(layout));
    Ok(pruned)
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
        && task_ids.contains(id)
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
    fn sidebar_task_destination_prefers_live_layout_then_saved_workspace_order() {
        use monitter_lib::experimental_desktop::{NativeWorkspace, NativeWorkspaceSet};
        let layout = |ids: &[&str]| serde_json::to_value(DockAreaState {
            version: Some(1), center: tabs(ids.iter().map(|id| pane(id)).collect(), 0),
            left_dock: None, right_dock: None, bottom_dock: None,
        }).unwrap();
        let current = layout(&["chat:current", "chat:duplicate"]);
        let mut set = NativeWorkspaceSet { active_id: "current".into(), items: [
            ("current", layout(&["chat:stale-current"])),
            ("first", layout(&["chat:elsewhere", "chat:duplicate"])),
            ("second", layout(&["chat:elsewhere"])),
        ].into_iter().map(|(id, layout)| NativeWorkspace {
            id: id.into(), name: id.into(), icon: "folder".into(), color: "#38BFD2".into(), url: None, layout: Some(layout),
        }).collect() };
        assert_eq!(sidebar_task_destination("chat:current", &current, Some(&set), true), SidebarTaskDestination::Current);
        assert_eq!(sidebar_task_destination("chat:duplicate", &current, Some(&set), true), SidebarTaskDestination::Current);
        assert_eq!(sidebar_task_destination("chat:elsewhere", &current, Some(&set), true), SidebarTaskDestination::Workspace("first".into()));
        set.items.swap(1, 2);
        assert_eq!(sidebar_task_destination("chat:elsewhere", &current, Some(&set), true), SidebarTaskDestination::Workspace("second".into()));
        assert_eq!(sidebar_task_destination("chat:missing", &current, Some(&set), true), SidebarTaskDestination::Open);
        assert_eq!(sidebar_task_destination("chat:stale-current", &current, Some(&set), true), SidebarTaskDestination::Open);
        for id in ["chat:current", "chat:elsewhere", "chat:missing"] {
            assert_eq!(sidebar_task_destination(id, &current, Some(&set), false), SidebarTaskDestination::Wait);
        }
    }

    #[test]
    fn remembered_workspace_focus_covers_browser_terminal_and_empty_panes() {
        let mut layout = serde_json::to_value(DockAreaState { version: Some(1),
            center: tabs(vec![pane("empty:landing"), pane("browser:42"), pane("terminal:session")], 0),
            left_dock: None, right_dock: None, bottom_dock: None }).unwrap();
        for id in ["browser:42", "terminal:session", "empty:landing"] {
            remember_focus(&mut layout, Some(id));
            assert_eq!(focus_candidates(&layout, None).first().map(String::as_str), Some(id));
            let mut resaved = serde_json::to_value(serde_json::from_value::<DockAreaState>(layout.clone()).unwrap()).unwrap();
            preserve_focus_metadata(&mut resaved, Some(&layout));
            assert_eq!(remembered_focus(&resaved), Some(id));
        }
        remember_focus(&mut layout, Some("terminal:session"));
        let pruned = prune_native_pane_ids(&layout, &HashSet::from(["terminal:session".into()])).unwrap();
        assert_eq!(remembered_focus(&pruned), None);
        assert_eq!(focus_candidates(&pruned, Some("terminal:session")), vec!["empty:landing", "browser:42"]);
    }

    #[test]
    fn workspace_focus_fallback_prefers_active_tab_and_never_hidden_workspace_pane() {
        let layout = serde_json::to_value(DockAreaState { version: Some(1),
            center: tabs(vec![pane("empty:landing"), pane("browser:42"), pane("terminal:session")], 2),
            left_dock: None, right_dock: None, bottom_dock: None }).unwrap();
        assert_eq!(focus_candidates(&layout, Some("terminal:other-workspace")),
            vec!["terminal:session", "empty:landing", "browser:42"]);
        let mut layout = layout;
        remember_focus(&mut layout, Some("terminal:other-workspace"));
        assert_eq!(remembered_focus(&layout), None);
    }

    #[test]
    fn terminal_exit_layout_pruning_removes_exact_session_and_preserves_unrelated_panes() {
        let layout=serde_json::to_value(DockAreaState {version:Some(1),center:tabs(vec![pane("terminal:ended"),pane("terminal:other"),pane("chat:ended"),pane("route:projects")],1),left_dock:None,right_dock:None,bottom_dock:None}).unwrap();
        let pruned=prune_native_pane_ids(&layout,&HashSet::from(["terminal:ended".to_owned()])).unwrap();
        let state:DockAreaState=serde_json::from_value(pruned.clone()).unwrap();
        assert_eq!(state.center.info,PanelInfo::tabs(0));
        assert_eq!(crate::docking::DockWorkspace::saved_ids(&pruned).unwrap().into_iter().collect::<HashSet<_>>(),HashSet::from(["terminal:other".into(),"chat:ended".into(),"route:projects".into()]));
        assert_eq!(prune_native_pane_ids(&pruned,&HashSet::from(["terminal:ended".into()])).unwrap(),pruned);
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
