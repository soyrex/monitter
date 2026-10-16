//! Restore-time validation for process-local terminal and Neovim sessions.

use serde_json::Value;
use std::collections::HashSet;

/// Remove saved session panes that have no live session in the authoritative
/// backend listings. A malformed Neovim listing is an error, never evidence
/// that every saved pane is missing.
pub(super) fn prune_missing_session_panes(
    layout: &Value,
    terminals: &[Value],
    neovim: &[Value],
) -> Result<Value, String> {
    if neovim.iter().any(|session| {
        session["id"].as_str().is_none() || session["exited"].as_bool().is_none()
    }) {
        return Err("Neovim list response is invalid.".into());
    }

    let missing = crate::docking::DockWorkspace::saved_ids(layout)?
        .into_iter()
        .filter(|id| {
            (crate::terminal_workspace::parse_terminal_pane_id(id).is_some()
                && matches!(
                    crate::terminal_workspace::lookup_terminal_session(id, terminals),
                    crate::terminal_workspace::TerminalSessionLookup::Missing
                ))
                || crate::neovim_workspace::parse_id(id).is_some_and(|(session_id, _)| {
                    !neovim.iter().any(|session| {
                        session["id"].as_str() == Some(session_id)
                            && session["exited"].as_bool() == Some(false)
                    })
                })
        })
        .collect::<HashSet<_>>();
    crate::workspace_visibility::prune_native_pane_ids(layout, &missing)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::base::dock::{DockAreaState, PanelInfo, PanelState};
    use serde_json::json;

    const NVIM: &str = "neovim:session-1:1001";
    const NVIM_EXITED: &str = "neovim:session-2:2002";
    const NVIM_MISSING: &str = "neovim:session-3:3003";
    const TERMINAL_MISSING: &str = "terminal:550e8400-e29b-41d4-a716-446655440000";
    const TERMINAL_LIVE: &str = "terminal:550e8400-e29b-41d4-a716-446655440001";

    fn pane(id: &str) -> PanelState {
        PanelState {
            panel_name: "MonitterNativePane".into(),
            children: Vec::new(),
            info: PanelInfo::panel(json!({"id": id})),
        }
    }

    fn layout(ids: &[&str]) -> Value {
        serde_json::to_value(DockAreaState {
            version: Some(1),
            center: PanelState {
                panel_name: "TabPanel".into(),
                children: ids.iter().map(|id| pane(id)).collect(),
                info: PanelInfo::tabs(0),
            },
            left_dock: None,
            right_dock: None,
            bottom_dock: None,
        })
        .unwrap()
    }

    fn ids(layout: &Value) -> HashSet<String> {
        crate::docking::DockWorkspace::saved_ids(layout)
            .unwrap()
            .into_iter()
            .collect()
    }

    #[test]
    fn prunes_missing_terminal_and_neovim_but_keeps_live_retained_sessions() {
        let layout = layout(&[NVIM, NVIM_MISSING, TERMINAL_MISSING, TERMINAL_LIVE]);
        let terminals = [json!({
            "id":"550e8400-e29b-41d4-a716-446655440001",
            "status":"running"
        })];
        let neovim = [json!({"id":"session-1","exited":false})];

        let restored = prune_missing_session_panes(&layout, &terminals, &neovim).unwrap();
        assert_eq!(ids(&restored), HashSet::from([NVIM.into(), TERMINAL_LIVE.into()]));
    }

    #[test]
    fn prunes_exited_neovim_session() {
        let layout = layout(&[NVIM_EXITED, NVIM]);
        let neovim = [
            json!({"id":"session-1","exited":false}),
            json!({"id":"session-2","exited":true}),
        ];

        let restored = prune_missing_session_panes(&layout, &[], &neovim).unwrap();
        assert_eq!(ids(&restored), HashSet::from([NVIM.into()]));
    }

    #[test]
    fn rejects_malformed_neovim_listing_without_pruning() {
        let layout = layout(&[NVIM]);
        let malformed = [json!({"id":"session-1"})];

        assert_eq!(
            prune_missing_session_panes(&layout, &[], &malformed).unwrap_err(),
            "Neovim list response is invalid."
        );
        assert_eq!(ids(&layout), HashSet::from([NVIM.into()]));
    }
}
