//! Task-scoped resolved approvals and exact-scope saved approval rules.
//!
//! Pending decisions stay in the owning chat's interaction flow. This pane is
//! read-only except for an explicit revoke action on a visible rule.
use crate::{design::ui_scaled_px as px, timestamps::datetime_label};
use futures::channel::oneshot;
use gpui_kit::{
    component::{Disableable, button::*},
    prelude::*,
    *,
};
use monitter_lib::experimental_desktop::DesktopService;
use serde_json::{Value, json};
use std::thread;

const WORKER_STACK_SIZE: usize = 8 * 1024 * 1024;

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn rows(value: &Value, key: &str) -> Vec<Value> {
    value
        .get(key)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// Keep only fields consumed by approval matching, cards, and revocation.
/// Message/event/transcript payloads are never visited or copied. Generation
/// checks use this projection plus the canonical task/revision identity.
fn approval_snapshot(snapshot: &Value, task_id: &str) -> Value {
    fn fields(row: &Value, keys: &[&str]) -> Value {
        Value::Object(
            keys.iter()
                .filter_map(|key| {
                    row.get(*key)
                        .map(|value| ((*key).to_owned(), value.clone()))
                })
                .collect(),
        )
    }
    fn source_rows<'a>(snapshot: &'a Value, key: &str) -> impl Iterator<Item = &'a Value> {
        snapshot[key].as_array().into_iter().flatten()
    }
    let task = source_rows(snapshot, "tasks").find(|task| task["id"].as_str() == Some(task_id));
    let agent_id = task.and_then(|task| task["agentId"].as_str());
    let host_id = task.and_then(|task| task["hostId"].as_str());
    let tasks: Vec<_> = task
        .into_iter()
        .map(|task| fields(task, &["id", "agentId", "hostId", "cwd", "provider"]))
        .collect();
    let agents: Vec<_> = source_rows(snapshot, "agents")
        .filter(|agent| agent_id.is_some() && agent["id"].as_str() == agent_id)
        .map(|agent| fields(agent, &["id", "name"]))
        .collect();
    let hosts: Vec<_> = source_rows(snapshot, "hosts")
        .filter(|host| host_id.is_some() && host["id"].as_str() == host_id)
        .map(|host| fields(host, &["id", "name"]))
        .collect();
    let rules: Vec<_> = source_rows(snapshot, "approvalRules")
        .filter(|rule| {
            task.is_some_and(|task| {
                ["agentId", "hostId", "cwd", "provider"].iter().all(|key| {
                    task[*key].as_str().is_some_and(|scope| {
                        !scope.is_empty() && rule[*key].as_str() == Some(scope)
                    })
                })
            })
        })
        .map(|rule| {
            fields(
                rule,
                &[
                    "id",
                    "agentId",
                    "hostId",
                    "cwd",
                    "provider",
                    "summary",
                    "tool",
                    "lastUsedAt",
                    "useCount",
                    "scopeDescription",
                    "detail",
                ],
            )
        })
        .collect();
    let approvals: Vec<_> = source_rows(snapshot, "approvalRequests")
        .filter(|approval| approval["taskId"].as_str() == Some(task_id))
        .map(|approval| {
            fields(
                approval,
                &[
                    "id",
                    "taskId",
                    "status",
                    "createdAt",
                    "resolvedAt",
                    "ruleId",
                    "summary",
                    "tool",
                    "decision",
                    "provider",
                ],
            )
        })
        .collect();
    json!({
        "settings": fields(&snapshot["settings"], &["interfaceFontSize"]),
        "tasks": tasks, "agents": agents, "hosts": hosts,
        "approvalRules": rules, "approvalRequests": approvals,
    })
}
fn approval_context_changed(
    current_task: &str,
    current_revision: Option<&str>,
    current_snapshot: &Value,
    task: &str,
    revision: Option<&str>,
    snapshot: &Value,
) -> bool {
    current_task != task || current_revision != revision || current_snapshot != snapshot
}

fn matching_rules(snapshot: &Value, task_id: &str) -> Vec<Value> {
    let Some(task) = rows(snapshot, "tasks")
        .into_iter()
        .find(|task| text(task, "id") == task_id)
    else {
        return Vec::new();
    };
    let agent_id = text(&task, "agentId");
    let host_id = text(&task, "hostId");
    let cwd = text(&task, "cwd");
    let provider = text(&task, "provider");
    if agent_id.is_empty() || host_id.is_empty() || cwd.is_empty() || provider.is_empty() {
        return Vec::new();
    }
    let mut rules = rows(snapshot, "approvalRules")
        .into_iter()
        .filter(|rule| {
            let rule_agent = text(rule, "agentId");
            let rule_host = text(rule, "hostId");
            let rule_cwd = text(rule, "cwd");
            let rule_provider = text(rule, "provider");
            !rule_agent.is_empty()
                && !rule_host.is_empty()
                && !rule_cwd.is_empty()
                && !rule_provider.is_empty()
                && rule_agent == agent_id
                && rule_host == host_id
                && rule_cwd == cwd
                && rule_provider == provider
        })
        .collect::<Vec<_>>();
    rules.sort_by(|a, b| text(a, "summary").cmp(&text(b, "summary")));
    rules
}

fn resolved_approvals(
    snapshot: &Value,
    task_id: &str,
) -> Vec<(crate::transcript::ApprovalRow, Value)> {
    let mut approvals = rows(snapshot, "approvalRequests")
        .into_iter()
        .filter(|approval| {
            text(approval, "taskId") == task_id
                && text(approval, "status") != "pending"
                && (approval["resolvedAt"].is_number() || text(approval, "status") != "")
        })
        .collect::<Vec<_>>();
    let mut approvals = approvals
        .into_iter()
        .map(|value| {
            let row = crate::transcript::ApprovalRow {
                id: text(&value, "id"),
                task_id: text(&value, "taskId"),
                status: text(&value, "status"),
                created_at: value["createdAt"].as_i64().unwrap_or_default(),
                resolved_at: value["resolvedAt"].as_i64(),
                rule_id: value["ruleId"].as_str().map(str::to_owned),
                summary: text(&value, "summary"),
                tool: text(&value, "tool"),
            };
            (row, value)
        })
        .collect::<Vec<_>>();
    approvals.sort_by_key(|(row, _)| row.at());
    approvals
}

pub struct PaneApprovals {
    service: DesktopService,
    snapshot: Value,
    revision: Option<String>,
    task_id: String,
    generation: u64,
    revoking_rule: Option<String>,
    failed_rule: Option<String>,
    error: Option<String>,
}

impl PaneApprovals {
    pub fn new(
        service: DesktopService,
        snapshot: &Value,
        task_id: impl Into<String>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Self {
        let task_id = task_id.into();
        Self {
            service,
            snapshot: approval_snapshot(snapshot, &task_id),
            revision: None,
            task_id,
            generation: 0,
            revoking_rule: None,
            failed_rule: None,
            error: None,
        }
    }

    pub fn set_context(
        &mut self,
        snapshot: &Value,
        revision: Option<&str>,
        task_id: impl Into<String>,
        cx: &mut Context<Self>,
    ) {
        let task_id = task_id.into();
        let snapshot = approval_snapshot(snapshot, &task_id);
        if approval_context_changed(
            &self.task_id,
            self.revision.as_deref(),
            &self.snapshot,
            &task_id,
            revision,
            &snapshot,
        ) {
            self.generation = self.generation.wrapping_add(1);
            self.revoking_rule = None;
            self.failed_rule = None;
            self.error = None;
        }
        self.snapshot = snapshot;
        self.revision = revision.map(str::to_owned);
        self.task_id = task_id;
        cx.notify();
    }

    fn revoke(&mut self, rule_id: String, cx: &mut Context<Self>) {
        if self.revoking_rule.is_some()
            || self.failed_rule.as_deref() == Some(rule_id.as_str())
            || !matching_rules(&self.snapshot, &self.task_id)
                .iter()
                .any(|rule| text(rule, "id") == rule_id)
        {
            return;
        }
        self.error = None;
        self.revoking_rule = Some(rule_id.clone());
        let generation = self.generation;
        let task_id = self.task_id.clone();
        let revision = self.revision.clone();
        let service = self.service.clone();
        let failed_rule_id = rule_id.clone();
        let worker_task_id = task_id.clone();
        let (sender, receiver) = oneshot::channel();
        let spawned = thread::Builder::new()
            .name("monitter-approvals-revoke".into())
            .stack_size(WORKER_STACK_SIZE)
            .spawn(move || {
                let result = service
                    .invoke("revoke_approval_rule", json!({"ruleId": rule_id}))
                    .map(|snapshot| approval_snapshot(&snapshot, &worker_task_id));
                let _ = sender.send(result);
            });
        if let Err(error) = spawned {
            self.revoking_rule = None;
            self.error = Some(format!("Could not start approval rule revocation: {error}"));
            cx.notify();
            return;
        }
        cx.spawn(async move |entity, cx| {
            let result = receiver.await.unwrap_or_else(|_| {
                Err("Approval rule revocation worker ended without a result.".into())
            });
            let _ = entity.update(cx, |pane, cx| {
                if pane.generation != generation
                    || pane.task_id != task_id
                    || pane.revision != revision
                {
                    return;
                }
                pane.revoking_rule = None;
                match result {
                    Ok(snapshot) => {
                        pane.snapshot = snapshot;
                        pane.error = None;
                    }
                    Err(error) => pane.error = Some(error),
                }
                if pane.error.is_some() {
                    pane.failed_rule = Some(failed_rule_id);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub fn render_body(
        &self,
        palette: crate::design::Palette,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let interface_ratio = self.snapshot["settings"]["interfaceFontSize"]
            .as_f64()
            .unwrap_or(14.0)
            .clamp(8.0, 32.0) as f32
            / 14.0;
        let detail_size = px(11.0 * interface_ratio);
        let mut body = div()
            .id("task-approvals-body")
            .flex()
            .flex_col()
            .gap_3()
            .text_size(detail_size)
            .text_color(rgb(palette.ink));

        if let Some(error) = self.error.as_deref() {
            body = body.child(
                div()
                    .text_size(detail_size)
                    .text_color(rgb(0xc44c79))
                    .child(format!("Approval rule was not revoked: {error}")),
            );
        }

        // Match the Tauri approval-history panel hierarchy: title and scope
        // explanation first, followed by saved rules (when present), then the
        // resolved history. Pending decisions remain in the chat flow.
        body = body.child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .pb_2()
                .border_b_1()
                .border_color(rgb(palette.line))
                .child(
                    div()
                        .text_size(px(14.0 * interface_ratio))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Approvals"),
                )
                .child(
                    div()
                        .text_size(detail_size)
                        .text_color(rgb(palette.muted))
                        .child("Saved rules for this agent, host and folder, plus this chat's resolved requests."),
                ),
        );

        let rules = matching_rules(&self.snapshot, &self.task_id);
        if !rules.is_empty() {
            let mut saved_rules = div()
                .flex()
                .flex_col()
                .gap_2()
                .p_2()
                .border_1()
                .border_color(rgb(palette.line))
                .rounded_md()
                .bg(crate::design::surface_color(palette.card))
                .child(
                    div()
                        .text_size(px(12.0 * interface_ratio))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Saved rules"),
                )
                .child(
                    div()
                        .text_size(px(10.0 * interface_ratio))
                        .text_color(rgb(palette.muted))
                        .child("Revoking affects future requests only; work already approved keeps running."),
                );
            for rule in rules {
                let id = text(&rule, "id");
                let busy = self.revoking_rule.as_deref() == Some(id.as_str());
                let failed = self.failed_rule.as_deref() == Some(id.as_str());
                let agent_name = rows(&self.snapshot, "agents")
                    .into_iter()
                    .find(|agent| text(agent, "id") == text(&rule, "agentId"))
                    .map(|agent| text(&agent, "name"))
                    .filter(|name| !name.is_empty())
                    .unwrap_or_else(|| "Unknown agent".into());
                let host_name = rows(&self.snapshot, "hosts")
                    .into_iter()
                    .find(|host| text(host, "id") == text(&rule, "hostId"))
                    .map(|host| text(&host, "name"))
                    .filter(|name| !name.is_empty())
                    .unwrap_or_else(|| "Unknown host".into());
                let summary = text(&rule, "summary");
                let tool = text(&rule, "tool");
                let last_used = rule["lastUsedAt"]
                    .as_i64()
                    .map(|timestamp| format!("Used {}", datetime_label(timestamp)))
                    .unwrap_or_else(|| "Never used".into());
                let uses = rule["useCount"].as_u64().unwrap_or_default();
                let cwd = text(&rule, "cwd");
                let folder = if cwd.is_empty() {
                    "Unknown folder"
                } else {
                    cwd.as_str()
                };
                let scope_description = text(&rule, "scopeDescription");
                let scope = if scope_description.is_empty() {
                    tool.as_str()
                } else {
                    scope_description.as_str()
                };
                let mut metadata = format!("Folder: {} · {} · {}", folder, scope, last_used);
                if uses > 0 {
                    metadata.push_str(&format!(
                        " · {uses} use{}",
                        if uses == 1 { "" } else { "s" }
                    ));
                }
                let card =
                    div()
                        .p_2()
                        .border_1()
                        .border_color(rgb(palette.line))
                        .rounded_md()
                        .bg(crate::design::surface_color(palette.card))
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(div().font_weight(FontWeight::SEMIBOLD).child(
                            if summary.is_empty() {
                                tool.clone()
                            } else {
                                summary
                            },
                        ))
                        .child(
                            div()
                                .font_family(crate::design::FONT_MONO)
                                .text_size(px(10.0 * interface_ratio))
                                .text_color(rgb(palette.muted))
                                .child(format!(
                                    "{agent_name} · {host_name} · {}",
                                    text(&rule, "provider")
                                )),
                        )
                        .child(
                            div()
                                .font_family(crate::design::FONT_MONO)
                                .text_size(px(10.0 * interface_ratio))
                                .text_color(rgb(palette.muted))
                                .child(metadata),
                        );
                let detail = text(&rule, "detail");
                if !detail.is_empty() {
                    saved_rules = saved_rules.child(
                        card.child(
                            div()
                                .text_size(detail_size)
                                .text_color(rgb(palette.muted))
                                .child(detail),
                        )
                        .child(
                            Button::new(SharedString::from(format!("revoke-approval-rule-{id}")))
                                .label(if busy { "Revoking…" } else { "Revoke" })
                                .disabled(self.revoking_rule.is_some() || failed)
                                .on_click(
                                    cx.listener(move |pane, _, _, cx| pane.revoke(id.clone(), cx)),
                                ),
                        ),
                    );
                } else {
                    saved_rules = saved_rules.child(
                        card.child(
                            Button::new(SharedString::from(format!("revoke-approval-rule-{id}")))
                                .label(if busy { "Revoking…" } else { "Revoke" })
                                .disabled(self.revoking_rule.is_some() || failed)
                                .on_click(
                                    cx.listener(move |pane, _, _, cx| pane.revoke(id.clone(), cx)),
                                ),
                        ),
                    );
                }
            }
            body = body.child(saved_rules);
        }

        let approvals = resolved_approvals(&self.snapshot, &self.task_id);
        if approvals.is_empty() {
            body = body.child(
                div()
                    .text_color(rgb(palette.muted))
                    .child("No resolved approvals for this chat."),
            );
        }
        for (approval_row, approval) in approvals {
            let status = text(&approval, "status");
            let decision = text(&approval, "decision");
            let label = match (status.as_str(), decision.as_str()) {
                ("approved", "approve_always") if !approval["ruleId"].is_null() => {
                    "Approved by saved rule"
                }
                ("approved", "approve_session") => "Approved for this session",
                ("approved", "approve_always") => "Always approved",
                ("approved", _) => "Approved",
                ("denied", _) => "Denied",
                ("expired", _) => "Expired",
                ("unsupported", _) => "Unsupported",
                _ => "Resolved",
            };
            let when = approval_row.at();
            let rule = approval_row
                .rule_id
                .as_deref()
                .map(|id| format!(" · rule {id}"));
            let card = div()
                .p_3()
                .bg(crate::design::surface_color(palette.card))
                .rounded_md()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(format!("{label}: {}", approval_row.summary)),
                )
                .child(
                    div()
                        .text_size(detail_size)
                        .text_color(rgb(palette.muted))
                        .child(format!(
                            "{} · {} · {}",
                            text(&approval, "provider"),
                            approval_row.tool,
                            datetime_label(when)
                        )),
                )
                .when_some(rule, |card, rule| {
                    card.child(
                        div()
                            .text_size(detail_size)
                            .text_color(rgb(palette.muted))
                            .child(rule),
                    )
                });
            body = body.child(card);
        }

        body.into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{approval_context_changed, approval_snapshot, matching_rules, resolved_approvals};
    use serde_json::json;

    fn scoped_snapshot() -> serde_json::Value {
        json!({
            "tasks": [{
                "id": "task-a", "agentId": "agent-a", "hostId": "host-a",
                "cwd": "/work/a", "provider": "codex"
            }],
            "approvalRequests": [
                {"id":"pending","taskId":"task-a","status":"pending","createdAt":1},
                {"id":"resolved-later","taskId":"task-a","status":"denied","createdAt":2,"resolvedAt":8},
                {"id":"resolved-first","taskId":"task-a","status":"approved","createdAt":3,"resolvedAt":5,"ruleId":"rule-a"},
                {"id":"other-task","taskId":"task-b","status":"approved","createdAt":4,"resolvedAt":4}
            ],
            "approvalRules": [
                {"id":"exact","agentId":"agent-a","hostId":"host-a","cwd":"/work/a","provider":"codex","tool":"read"},
                {"id":"other-agent","agentId":"agent-b","hostId":"host-a","cwd":"/work/a","provider":"codex"},
                {"id":"other-host","agentId":"agent-a","hostId":"host-b","cwd":"/work/a","provider":"codex"},
                {"id":"other-folder","agentId":"agent-a","hostId":"host-a","cwd":"/work/b","provider":"codex"},
                {"id":"other-provider","agentId":"agent-a","hostId":"host-a","cwd":"/work/a","provider":"claude"},
                {"id":"missing-scope","agentId":"agent-a","hostId":"host-a","cwd":"/work/a"}
            ]
        })
    }

    #[::core::prelude::v1::test]
    fn approval_history_is_resolved_and_task_scoped_in_timestamp_order() {
        let approvals = resolved_approvals(&scoped_snapshot(), "task-a");
        let ids = approvals
            .iter()
            .map(|(row, _)| row.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["resolved-first", "resolved-later"]);
        assert_eq!(approvals[0].0.rule_id.as_deref(), Some("rule-a"));
    }

    #[::core::prelude::v1::test]
    fn saved_rules_require_exact_nonempty_agent_host_folder_and_provider_scope() {
        let rules = matching_rules(&scoped_snapshot(), "task-a");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0]["id"], "exact");
    }

    #[::core::prelude::v1::test]
    fn incomplete_selected_task_scope_does_not_match_empty_rule_fields() {
        let snapshot = json!({
            "tasks": [{"id":"task-a","agentId":"","hostId":"","cwd":"","provider":""}],
            "approvalRules": [{"id":"empty","agentId":"","hostId":"","cwd":"","provider":""}]
        });
        assert!(matching_rules(&snapshot, "task-a").is_empty());
    }
    #[::core::prelude::v1::test]
    fn large_transcripts_are_excluded_without_changing_approval_results_or_fences() {
        let mut source = scoped_snapshot();
        source["settings"] =
            json!({"interfaceFontSize":16,"largeUnusedAppearanceData":"x".repeat(1024)});
        source["agents"] = json!([{"id":"agent-a","name":"Agent A","description":"unused"}]);
        source["hosts"] = json!([{"id":"host-a","name":"Host A","sshPrivatePayload":"unused"}]);
        source["approvalRules"][0]["summary"] = json!("Read source files");
        source["approvalRules"][0]["detail"] = json!("Allowed within /work/a");
        source["approvalRequests"][2]["decision"] = json!("approve_always");
        let baseline = approval_snapshot(&source, "task-a");
        let expected_rules = matching_rules(&source, "task-a");
        let expected_approvals: Vec<_> = resolved_approvals(&source, "task-a")
            .into_iter()
            .map(|(row, _)| {
                let at = row.at();
                (row.id, row.status, row.rule_id, at)
            })
            .collect();
        let payload = "history payload ".repeat(128);
        source["messages"] = json!(
            (0..10_000)
                .map(|index| json!({"id":index,"taskId":"task-a","text":payload}))
                .collect::<Vec<_>>()
        );
        source["events"] = json!([{"taskId":"task-a","detail":payload}]);
        source["subagentTranscripts"] = json!({"child":[{"text":payload}]});
        source["tasks"][0]["modelSettings"] = json!({"unused":payload});
        let projected = approval_snapshot(&source, "task-a");
        assert_eq!(projected, baseline);
        assert!(serde_json::to_vec(&projected).unwrap().len() < 4096);
        assert_eq!(matching_rules(&projected, "task-a"), expected_rules);
        let actual_approvals: Vec<_> = resolved_approvals(&projected, "task-a")
            .into_iter()
            .map(|(row, _)| {
                let at = row.at();
                (row.id, row.status, row.rule_id, at)
            })
            .collect();
        assert_eq!(actual_approvals, expected_approvals);
        assert!(!approval_context_changed(
            "task-a",
            Some("e:1"),
            &baseline,
            "task-a",
            Some("e:1"),
            &projected
        ));
        assert!(approval_context_changed(
            "task-a",
            Some("e:1"),
            &baseline,
            "task-a",
            Some("e:2"),
            &projected
        ));
        assert!(approval_context_changed(
            "task-a",
            Some("e:1"),
            &baseline,
            "task-b",
            Some("e:1"),
            &projected
        ));
        source["approvalRequests"][2]["decision"] = json!("approve_session");
        assert!(approval_context_changed(
            "task-a",
            Some("e:1"),
            &baseline,
            "task-a",
            Some("e:1"),
            &approval_snapshot(&source, "task-a")
        ));
        source["tasks"][0]["cwd"] = json!("/work/changed");
        let moved = approval_snapshot(&source, "task-a");
        assert!(matching_rules(&moved, "task-a").is_empty());
        assert!(approval_context_changed(
            "task-a",
            Some("e:1"),
            &baseline,
            "task-a",
            Some("e:1"),
            &moved
        ));
    }
}
