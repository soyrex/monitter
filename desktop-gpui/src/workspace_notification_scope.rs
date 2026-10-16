//! Projection of task notifications into a saved native workspace layout.

use crate::notifications::NotificationEntry;
use serde_json::Value;
use std::collections::HashSet;

/// Return notifications owned by tasks represented in this saved workspace.
/// Channel notifications stay in the global feed because they have no native
/// workspace association.
pub(super) fn entries_for_layout(
    entries: &[NotificationEntry],
    layout: Option<&Value>,
) -> Vec<NotificationEntry> {
    let Some(layout) = layout else {
        return Vec::new();
    };
    let Ok(pane_ids) = crate::docking::DockWorkspace::saved_ids(layout) else {
        return Vec::new();
    };

    let task_ids: HashSet<&str> = pane_ids
        .iter()
        .filter_map(|id| {
            id.strip_prefix("chat:")
                .or_else(|| crate::git_workspace::task_id(id))
        })
        .collect();

    entries
        .iter()
        .filter(|entry| entry.channel_id.is_none() && task_ids.contains(entry.task_id.as_str()))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notifications::{NotificationKind, NotificationTone};
    use gpui_kit::base::dock::{DockAreaState, PanelInfo, PanelState};
    use serde_json::json;

    fn pane(id: &str) -> PanelState {
        PanelState {
            panel_name: "MonitterNativePane".into(),
            children: vec![],
            info: PanelInfo::panel(json!({ "id": id })),
        }
    }

    fn layout(ids: &[&str]) -> Value {
        let center = PanelState {
            panel_name: "TabPanel".into(),
            children: ids.iter().map(|id| pane(id)).collect(),
            info: PanelInfo::tabs(0),
        };
        serde_json::to_value(DockAreaState {
            version: Some(1),
            center,
            left_dock: None,
            right_dock: None,
            bottom_dock: None,
        })
        .unwrap()
    }

    fn entry(id: &str, task_id: &str) -> NotificationEntry {
        NotificationEntry {
            id: id.into(),
            created_at: 1,
            task_id: task_id.into(),
            channel_id: None,
            origin: "Codex".into(),
            tone: NotificationTone::Info,
            title: id.into(),
            detail: String::new(),
            message_id: None,
            summary: None,
            kind: NotificationKind::Message,
        }
    }

    #[test]
    fn projects_chat_and_git_tasks_once_in_entry_order_including_approvals() {
        let mut approval = entry("approval", "git-task");
        approval.kind = NotificationKind::Approval {
            request: json!({ "id": "approval", "status": "pending" }),
        };
        let entries = vec![
            entry("chat-notice", "chat-task"),
            entry("git-notice", "git-task"),
            approval,
            entry("outside", "other-task"),
        ];

        let projected = entries_for_layout(
            &entries,
            Some(&layout(&[
                "chat:chat-task",
                "git:git-task:1",
                "git:git-task:2",
            ])),
        );

        assert_eq!(
            projected
                .iter()
                .map(|entry| entry.id.as_str())
                .collect::<Vec<_>>(),
            ["chat-notice", "git-notice", "approval"]
        );
        assert!(matches!(
            &projected[2].kind,
            NotificationKind::Approval { .. }
        ));
    }

    #[test]
    fn missing_or_invalid_scopes_and_channel_entries_are_excluded() {
        let mut channel = entry("channel", "chat-task");
        channel.channel_id = Some("channel-id".into());
        let entries = vec![
            entry("task", "chat-task"),
            entry("unscoped", "other"),
            channel,
        ];

        assert!(entries_for_layout(&entries, None).is_empty());
        assert!(entries_for_layout(&entries, Some(&json!({ "version": 2 }))).is_empty());

        let projected = entries_for_layout(&entries, Some(&layout(&["route:terminal"])));
        assert!(projected.is_empty());

        let projected = entries_for_layout(&entries, Some(&layout(&["chat:chat-task"])));
        assert_eq!(
            projected
                .iter()
                .map(|entry| entry.id.as_str())
                .collect::<Vec<_>>(),
            ["task"]
        );
    }
}
