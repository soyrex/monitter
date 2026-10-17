//! Native inbox routing. The backend validates every approval decision.
use crate::workspace_windows::DesktopContextExt as _;
use crate::{Desktop, approval_prompt, field, rows, notifications::{NotificationBell, NotificationEvent}};
use gpui_kit::{prelude::*, *};
use serde_json::{Value, json};
use std::collections::HashMap;

pub(super) struct WorkspaceNotificationFeed {
    pub bell: Entity<NotificationBell>,
    _subscription: Subscription,
}

fn approval_command(request: &Value, decision: &str, native_mcp: bool) -> Result<(&'static str, Value), String> {
    if !matches!(decision, "approve_once" | "deny")
        || !approval_prompt::allowed_decisions(request).iter().any(|(value, _)| *value == decision)
        || field(request, "id").is_empty() {
        return Err("This request no longer allows that decision. Refresh and review it again.".into());
    }
    let id = field(request, "id");
    if decision == "approve_once" && !approval_prompt::mcp_approval_choices(request).is_empty() {
        if native_mcp { Ok(("resolve_mcp_approval", json!({"approvalId":id,"decision":decision}))) }
        else { Ok(("resolve_input", json!({"approvalId":id,"response":{}}))) }
    } else { Ok(("resolve_approval", json!({"approvalId":id,"decision":decision}))) }
}
impl Desktop {
    fn retained_notification_bells(&self) -> impl Iterator<Item = &Entity<NotificationBell>> {
        self.window.notification_bell.iter().chain(
            self.parked_window_views.values().chain(self.workspace_view_cache.values())
                .filter_map(|view| view.notification_bell.as_ref())
        )
    }

    pub(super) fn sync_notification_bell(&mut self, cx: &mut Context<Self>) {
        if let Some(service) = self.service.clone() {
            for (task_id, message_id) in self.notification_model.take_summary_requests() {
                let service = service.clone();
                let request_task = task_id.clone();
                let request_message = message_id.clone();
                let work = cx.background_executor().spawn(async move {
                    service.invoke("summarize_notification", json!({"taskId":request_task,"messageId":request_message}))
                });
                cx.spawn(async move |view, cx| {
                    let result = work.await;
                    let _ = view.update(cx, |view, cx| {
                        if let Ok(result) = result {
                            if result["source"] == "admin" && result["taskId"] == task_id && result["messageId"] == message_id {
                                if let Some(summary) = result["summary"].as_str() {
                                    view.notification_model.apply_summary(&task_id, &message_id, summary.to_owned());
                                }
                            }
                        }
                        view.sync_notification_bell(cx);
                    });
                }).detach();
            }
        }
        let entries = self.notification_model.entries(&self.snapshot);
        for bell in self.retained_notification_bells() {
            bell.update(cx, |bell, cx| bell.set_entries(entries.clone(), cx));
        }
        self.sync_workspace_notification_entries(cx);
    }
    pub(super) fn ensure_notification_bell(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<NotificationBell> {
        if let Some(bell) = &self.window.notification_bell { return bell.clone(); }
        let bell = cx.new(NotificationBell::new);
        let subscription = self.subscribe_notification_feed(&bell, None, window, cx);
        self.window.notification_subscription = Some(subscription);
        self.window.notification_bell = Some(bell.clone());
        self.sync_notification_bell(cx);
        bell
    }
    fn subscribe_notification_feed(&self, bell: &Entity<NotificationBell>, workspace: Option<String>, window: &mut Window, cx: &mut Context<Self>) -> Subscription {
        cx.subscribe_workspace(bell, window, move |view, _, event, window, cx| {
            match event {
                NotificationEvent::Dismiss { id } => view.notification_model.dismiss(id),
                NotificationEvent::ReviewChannel { channel_id } => {
                    view.palette_channels(window, cx);
                    if let Some(pane) = &view.window.channels {
                        pane.update(cx, |pane, cx| pane.select(Some(channel_id.clone()), window, cx));
                    }
                    view.notification_model.mark_channel_read(channel_id);
                }
                NotificationEvent::Review { task_id, request_id } => {
                    if !rows(&view.snapshot, "tasks").iter().any(|task| field(task, "id") == *task_id) { return; }
                    if let Some(id) = request_id {
                        if !rows(&view.snapshot, "approvalRequests").iter().any(|request|
                            field(request, "id") == *id && field(request, "taskId") == *task_id
                                && field(request, "status") == "pending") {
                            view.sync_notification_bell(cx);
                            return;
                        }
                    }
                    // Reuse sidebar navigation: reveal the existing pane, including
                    // switching to its named workspace, instead of opening a duplicate.
                    if let Some(workspace) = &workspace {
                        if !view.navigate_workspace_notification(workspace, task_id.clone(), window, cx) { return; }
                    } else {
                        view.navigate_sidebar_task(task_id.clone(), window, cx);
                    }
                    if let Some(service) = view.service.clone() {
                        let pane = view.ensure_chat_pane(task_id, service, window, cx);
                        if let Some(id) = request_id {
                            pane.update(cx, |pane, cx| pane.review_notification_request(id.clone(), window, cx));
                        }
                    }
                    if workspace.is_none() { view.notification_model.mark_task_read(task_id); }
                }
                NotificationEvent::Decide { approval_id, task_id, decision } => {
                    view.decide_notification(approval_id, task_id, decision, cx);
                }
            }
            view.sync_notification_bell(cx);
            cx.notify();
        })
    }
    fn sync_workspace_notification_entries(&mut self, cx: &mut Context<Self>) {
        let entries = self.notification_model.entries(&self.snapshot);
        self.workspace_notifications.retain(|id, _| self.named_workspaces.as_ref().is_some_and(|set|
            id != &set.active_id && set.items.iter().any(|item| &item.id == id)));
        for (id, feed) in &self.workspace_notifications {
            let Some(item) = self.named_workspaces.as_ref().and_then(|set| set.items.iter().find(|item| &item.id == id)) else { continue; };
            let filtered = crate::workspace_notification_scope::entries_for_layout(&entries, item.layout.as_ref());
            feed.bell.update(cx, |bell, cx| {
                bell.set_workspace_name(&item.name, cx);
                bell.set_entries(filtered, cx);
            });
        }
    }
    /// Retain one renderer per inactive tab, backed by the same unread model and request progress.
    pub(super) fn workspace_notification_pills(&mut self, window: &mut Window, cx: &mut Context<Self>) -> HashMap<String, Entity<NotificationBell>> {
        let global = self.ensure_notification_bell(window, cx);
        self.sync_workspace_notification_entries(cx);
        let entries = self.notification_model.entries(&self.snapshot);
        let items: Vec<_> = self.named_workspaces.as_ref().map(|set| set.items.iter()
            .filter(|item| item.id != set.active_id).cloned().collect()).unwrap_or_default();
        let mut pills = HashMap::new();
        for item in items {
            let filtered = crate::workspace_notification_scope::entries_for_layout(&entries, item.layout.as_ref());
            if filtered.is_empty() { continue; }
            if !self.workspace_notifications.contains_key(&item.id) {
                let bell = cx.new(|cx| NotificationBell::for_workspace(item.id.clone(), item.name.clone(), global.read(cx)));
                bell.update(cx, |bell, cx| bell.set_entries(filtered, cx));
                let subscription = self.subscribe_notification_feed(&bell, Some(item.id.clone()), window, cx);
                self.workspace_notifications.insert(item.id.clone(), WorkspaceNotificationFeed { bell, _subscription: subscription });
            }
            pills.insert(item.id.clone(), self.workspace_notifications[&item.id].bell.clone());
        }
        pills
    }
    fn set_notification_request_state(&mut self, id: &str, busy: bool, error: Option<String>, cx: &mut Context<Self>) {
        for bell in self.retained_notification_bells() {
            bell.update(cx, |bell, cx| bell.set_request_state(id, busy, error.clone(), cx));
        }
        for feed in self.workspace_notifications.values() {
            feed.bell.update(cx, |bell, cx| bell.set_request_state(id, busy, error.clone(), cx));
        }
    }
    fn decide_notification(&mut self, id: &str, task_id: &str, decision: &str, cx: &mut Context<Self>) {
        let request = rows(&self.snapshot, "approvalRequests").into_iter()
            .find(|request| field(request, "id") == id && field(request, "taskId") == task_id);
        let Some(request) = request else { self.sync_notification_bell(cx); return; };
        let Some(service) = self.service.clone() else {
            self.set_notification_request_state(id, false, Some("Backend unavailable. Try again when connected.".into()), cx);
            return;
        };
        if let Err(error) = approval_command(&request, decision, false) {
            self.set_notification_request_state(id, false, Some(error), cx);
            return;
        }
        self.set_notification_request_state(id, true, None, cx);
        let id = id.to_owned();
        let decision = decision.to_owned();
        let task = cx.background_executor().spawn(async move {
            let native = decision == "approve_once" && !approval_prompt::mcp_approval_choices(&request).is_empty()
                && service.invoke("read_mcp_approval_support", json!({})).ok()
                    .is_some_and(|value| value["nativeScopes"].as_bool() == Some(true));
            let (command, args) = approval_command(&request, &decision, native)?;
            service.invoke(command, args)
        });
        cx.spawn(async move |view, cx| {
            let result = task.await;
            let _ = view.update(cx, |view, cx| {
                let error = result.err();
                // Keep success busy until the authoritative pending row disappears.
                view.set_notification_request_state(&id, error.is_none(), error.clone(), cx);
                view.refresh(cx);
                cx.notify();
            });
        }).detach();
    }
}
#[cfg(test)]
mod tests {
    use super::approval_command;
    use serde_json::json;
    #[::core::prelude::v1::test]
    fn notification_decisions_validate_pending_and_scope() {
        let request = json!({"id":"request", "status":"pending", "input":null});
        assert_eq!(approval_command(&request,"approve_once",false).unwrap().0,"resolve_approval");
        assert_eq!(approval_command(&request,"deny",false).unwrap().1["decision"],"deny");
        assert!(approval_command(&request,"approve_always",false).is_err());
        assert!(approval_command(&json!({"id":"request","status":"approved"}),"approve_once",false).is_err());
        assert!(approval_command(&json!({"id":"request","status":"pending","input":{"kind":"text"}}),"approve_once",false).is_err());
    }
}
