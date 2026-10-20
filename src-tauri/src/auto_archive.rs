//! Deterministic, reversible chat retention. Selection and writes share the
//! state writer lock, so activity arriving after a preview is checked again.
use crate::{
    agent_worktree,
    model::{id, now, RunEvent, ScheduleMode, Snapshot},
    Service,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
};

const DAY_MS: i64 = 86_400_000;
const MAX_DAYS: u64 = 36_500;
const MAX_RETURNED_TASKS: usize = 100;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChatArchiveRule {
    /// None is the global fallback; Some is an exact project override.
    pub project_id: Option<String>,
    pub older_than_days: u32,
    pub enabled: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Validate at both the MCP transport and service boundary.
pub(crate) fn validate_args(tool: &str, args: &Value) -> Result<(), String> {
    let args = args
        .as_object()
        .ok_or("Tool arguments must be an object.")?;
    let allowed: &[&str] = match tool {
        "list_chat_archive_rules" => &[],
        "save_chat_archive_rule" => &["scope", "project_id", "older_than_days", "enabled"],
        "delete_chat_archive_rule" => &["scope", "project_id"],
        "archive_chats" => &["scope", "project_id", "older_than_days", "dry_run"],
        _ => return Err("Unknown chat archive tool.".into()),
    };
    if args.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err("Tool arguments include unsupported fields.".into());
    }
    if tool == "list_chat_archive_rules" {
        return Ok(());
    }
    match args.get("scope").and_then(Value::as_str) {
        Some("global") if !args.contains_key("project_id") => {}
        Some("project")
            if args
                .get("project_id")
                .and_then(Value::as_str)
                .is_some_and(|value| uuid::Uuid::parse_str(value).is_ok()) => {}
        _ => {
            return Err(
                "scope must be global (without project_id) or project (with a UUID project_id)."
                    .into(),
            )
        }
    }
    if tool != "delete_chat_archive_rule"
        && !args
            .get("older_than_days")
            .and_then(Value::as_u64)
            .is_some_and(|days| (1..=MAX_DAYS).contains(&days))
    {
        return Err("older_than_days must be an integer between 1 and 36500.".into());
    }
    for field in ["enabled", "dry_run"] {
        if args.get(field).is_some_and(|value| !value.is_boolean()) {
            return Err(format!("{field} must be a boolean."));
        }
    }
    Ok(())
}

fn require_caller(snapshot: &Snapshot, caller: &str, write: bool) -> Result<(), String> {
    let task = snapshot
        .tasks
        .iter()
        .find(|task| task.id == caller)
        .ok_or("Collaboration caller task was not found.")?;
    if task.archived
        || task.status != "running"
        || !snapshot.agents.iter().any(|agent| {
            agent.id == task.agent_id && !agent.internal && agent.collaboration_enabled
        })
    {
        return Err("Collaboration caller is not eligible.".into());
    }
    if write && task.sandbox == "read-only" {
        return Err("Read-only tasks can list archive rules and preview matches, but cannot change archive state.".into());
    }
    Ok(())
}

fn require_project(snapshot: &Snapshot, project_id: Option<&str>) -> Result<(), String> {
    if project_id.is_some_and(|id| !snapshot.projects.iter().any(|project| project.id == id)) {
        return Err("Project was not found.".into());
    }
    Ok(())
}

fn audit(snapshot: &mut Snapshot, task_id: &str, title: &str, detail: String, time: i64) {
    snapshot.events.push(Arc::new(RunEvent {
        id: id(),
        task_id: task_id.into(),
        kind: "status".into(),
        title: title.into(),
        detail: detail.into(),
        created_at: time,
    }));
}

impl Service {
    pub(crate) fn chat_archive_protocol(
        &self,
        caller: &str,
        tool: &str,
        args: &Map<String, Value>,
    ) -> Result<Value, String> {
        validate_args(tool, &Value::Object(args.clone()))?;
        let project_id = args.get("project_id").and_then(Value::as_str);
        let dry_run = args.get("dry_run").and_then(Value::as_bool).unwrap_or(true);
        let write = tool != "list_chat_archive_rules" && (tool != "archive_chats" || !dry_run);
        // Includes read/preview so the caller grant and candidates are checked
        // against the same committed state as any resulting archive mutation.
        self.mutate(None, |snapshot| {
            require_caller(snapshot, caller, write)?;
            require_project(snapshot, project_id)?;
            let time = now();
            match tool {
                "list_chat_archive_rules" => Ok(json!({
                    "rules": snapshot.chat_archive_rules,
                    "projects": snapshot.projects.iter().map(|p| json!({"id":p.id,"name":p.name})).collect::<Vec<_>>(),
                    "checkIntervalSeconds": 15,
                    "ageBasis": "last activity; strict older-than cutoff",
                })),
                "save_chat_archive_rule" => {
                    let days = args["older_than_days"].as_u64().unwrap() as u32;
                    let enabled = args.get("enabled").and_then(Value::as_bool).unwrap_or(true);
                    let existing = snapshot.chat_archive_rules.iter_mut().find(|rule| rule.project_id.as_deref() == project_id);
                    let rule = if let Some(rule) = existing {
                        rule.older_than_days = days;
                        rule.enabled = enabled;
                        rule.updated_at = time;
                        rule.clone()
                    } else {
                        let rule = ChatArchiveRule { project_id: project_id.map(str::to_owned),
                            older_than_days: days, enabled, created_at: time, updated_at: time };
                        rule
                    };
                    // Upsert is by scope, including when imported state had duplicates.
                    snapshot.chat_archive_rules.retain(|saved| saved.project_id != rule.project_id);
                    snapshot.chat_archive_rules.push(rule.clone());
                    audit(snapshot, caller, "Chat archive rule saved", serde_json::to_string(&rule).unwrap(), time);
                    Ok(json!({"rule": rule, "appliesOnNextCheck": enabled}))
                },
                "delete_chat_archive_rule" => {
                    let before = snapshot.chat_archive_rules.len();
                    snapshot.chat_archive_rules.retain(|rule| rule.project_id.as_deref() != project_id);
                    let removed = before != snapshot.chat_archive_rules.len();
                    if removed { audit(snapshot, caller, "Chat archive rule removed", format!("scope: {}; project: {}", args["scope"], project_id.unwrap_or("all")), time); }
                    Ok(json!({"removed": removed}))
                },
                "archive_chats" => {
                    let days = args["older_than_days"].as_u64().unwrap() as u32;
                    let runs = self.runs.lock().map_err(|_| "Monitter run registry lock failed.")?;
                    let residents = runs.tasks.keys().map(String::as_str).collect();
                    let selection = select(snapshot, &residents, Some((project_id, days)), time);
                    let result = selection.report(dry_run, time);
                    if !dry_run { apply(snapshot, &selection, time, Some(caller))?; }
                    Ok(result)
                },
                _ => unreachable!(),
            }
        })
    }

    /// Called by the existing native scheduler. No harness or inference call.
    pub(crate) fn auto_archive_tick(&self) -> Result<usize, String> {
        // A disabled feature should not clone or scan the conversation history.
        if !self
            .committed_data()?
            .snapshot
            .chat_archive_rules
            .iter()
            .any(|rule| rule.enabled)
        {
            return Ok(0);
        }
        self.auto_archive_at(now())
    }

    fn auto_archive_at(&self, time: i64) -> Result<usize, String> {
        self.mutate(None, |snapshot| {
            if !snapshot.chat_archive_rules.iter().any(|rule| rule.enabled) {
                return Ok(0);
            }
            let runs = self
                .runs
                .lock()
                .map_err(|_| "Monitter run registry lock failed.")?;
            let residents = runs.tasks.keys().map(String::as_str).collect();
            let selection = select(snapshot, &residents, None, time);
            apply(snapshot, &selection, time, None)?;
            Ok(selection.matches.len())
        })
    }
}

struct Match {
    id: String,
    title: String,
    project_id: Option<String>,
    last_activity_at: i64,
    days: u32,
}
#[derive(Default)]
struct Selection {
    matches: Vec<Match>,
    skipped: BTreeMap<&'static str, usize>,
}
impl Selection {
    fn report(&self, dry_run: bool, time: i64) -> Value {
        json!({"dryRun": dry_run, "evaluatedAt": time, "matchedCount": self.matches.len(),
            "archivedCount": if dry_run { 0 } else { self.matches.len() },
            "skippedProtectedCount": self.skipped.values().sum::<usize>(), "skippedByReason": self.skipped,
            "tasks": self.matches.iter().take(MAX_RETURNED_TASKS).map(|task| json!({
                "id":task.id, "title":task.title, "projectId":task.project_id,
                "lastActivityAt":task.last_activity_at, "olderThanDays":task.days,
            })).collect::<Vec<_>>(), "tasksTruncated": self.matches.len() > MAX_RETURNED_TASKS})
    }
}

fn select(
    snapshot: &Snapshot,
    residents: &HashSet<&str>,
    one_time: Option<(Option<&str>, u32)>,
    time: i64,
) -> Selection {
    let mut activity: HashMap<&str, i64> = snapshot
        .tasks
        .iter()
        .map(|t| (t.id.as_str(), t.updated_at.max(t.created_at)))
        .collect();
    let mut protected: HashMap<&str, &'static str> = HashMap::new();
    let internal_agents: HashSet<&str> = snapshot
        .agents
        .iter()
        .filter(|agent| agent.internal)
        .map(|agent| agent.id.as_str())
        .collect();
    for task in &snapshot.tasks {
        if internal_agents.contains(task.agent_id.as_str()) {
            protected.insert(&task.id, "internal_task");
        }
        if !matches!(
            task.status.as_str(),
            "idle" | "completed" | "error" | "interrupted"
        ) {
            protected.insert(&task.id, "active_task");
        }
        if residents.contains(task.id.as_str()) {
            protected.insert(&task.id, "resident_runtime");
        }
    }
    for message in &snapshot.messages {
        if let Some(last) = activity.get_mut(message.task_id.as_str()) {
            *last = (*last).max(message.created_at);
        }
        if message.stream_status.as_deref() == Some("streaming") {
            protected.insert(&message.task_id, "streaming_message");
        }
    }
    for message in &snapshot.queued_messages {
        if matches!(message.status.as_str(), "queued" | "sending") {
            protected.insert(&message.task_id, "queued_message");
        }
    }
    for request in &snapshot.approval_requests {
        if request.status == "pending" {
            protected.insert(&request.task_id, "pending_approval");
        }
    }
    for item in &snapshot.collaborations {
        for task in [&item.from_task_id, &item.to_task_id] {
            if let Some(last) = activity.get_mut(task.as_str()) {
                *last = (*last).max(item.updated_at);
            }
            if matches!(item.status.as_str(), "queued" | "running") {
                protected.insert(task, "active_collaboration");
            }
        }
    }
    for session in &snapshot.subagent_sessions {
        if let Some(last) = activity.get_mut(session.parent_task_id.as_str()) {
            *last = (*last).max(session.updated_at);
        }
        if !matches!(
            session.status.as_str(),
            "completed" | "error" | "interrupted"
        ) {
            protected.insert(&session.parent_task_id, "active_subagent");
        }
    }
    for plan in &snapshot.work_plans {
        if let Some(last) = activity.get_mut(plan.task_id.as_str()) {
            *last = (*last).max(plan.updated_at);
        }
        if plan.status == "active" {
            protected.insert(&plan.task_id, "active_work_plan");
        }
    }
    for task in &snapshot.pending_throwaway_task_ids {
        protected.insert(task, "pending_schedule_cleanup");
    }
    // Paused persistent schedules also retain their conversation for resume.
    for schedule in &snapshot.schedules {
        if schedule.mode == ScheduleMode::PersistentThread {
            if let Some(task) = snapshot
                .schedule_runs
                .iter()
                .rev()
                .find(|run| run.schedule_id == schedule.id && run.task_id.is_some())
                .and_then(|run| run.task_id.as_deref())
            {
                protected.insert(task, "persistent_schedule");
            }
        }
    }
    let parents: HashMap<&str, &str> = snapshot
        .tasks
        .iter()
        .filter_map(|task| {
            task.parent_task_id
                .as_deref()
                .map(|parent| (task.id.as_str(), parent))
        })
        .collect();
    for task in protected.keys().copied().collect::<Vec<_>>() {
        let mut current = task;
        let mut seen = HashSet::new();
        while let Some(parent) = parents.get(current).copied() {
            if !seen.insert(parent) {
                break;
            }
            protected.entry(parent).or_insert("active_descendant");
            current = parent;
        }
    }
    let global = snapshot
        .chat_archive_rules
        .iter()
        .find(|rule| rule.project_id.is_none());
    let projects: HashMap<&str, &ChatArchiveRule> = snapshot
        .chat_archive_rules
        .iter()
        .filter_map(|rule| rule.project_id.as_deref().map(|p| (p, rule)))
        .collect();
    let mut result = Selection::default();
    for task in &snapshot.tasks {
        if task.archived {
            continue;
        }
        let days = if let Some((project_id, days)) = one_time {
            if project_id.is_some() && task.project_id.as_deref() != project_id {
                continue;
            }
            days
        } else {
            let rule = task
                .project_id
                .as_deref()
                .and_then(|p| projects.get(p).copied())
                .or(global);
            let Some(rule) = rule.filter(|rule| rule.enabled) else {
                continue;
            };
            rule.older_than_days
        };
        // Fail closed for malformed legacy/imported rules and future timestamps.
        if !(1..=MAX_DAYS).contains(&(days as u64)) {
            continue;
        }
        let last = activity[task.id.as_str()];
        if last >= time.saturating_sub(i64::from(days) * DAY_MS) {
            continue;
        }
        if let Some(reason) = protected.get(task.id.as_str()) {
            *result.skipped.entry(reason).or_default() += 1;
            continue;
        }
        if task.agent_worktree.as_ref().is_some_and(|worktree| {
            agent_worktree::can_retire(&task.id, &task.cwd, worktree).is_err()
        }) {
            *result
                .skipped
                .entry("managed_worktree_changes")
                .or_default() += 1;
            continue;
        }
        result.matches.push(Match {
            id: task.id.clone(),
            title: task.title.chars().take(240).collect(),
            project_id: task.project_id.clone(),
            last_activity_at: last,
            days,
        });
    }
    result
}

fn apply(
    snapshot: &mut Snapshot,
    selection: &Selection,
    time: i64,
    caller: Option<&str>,
) -> Result<(), String> {
    let ids: HashSet<&str> = selection
        .matches
        .iter()
        .map(|task| task.id.as_str())
        .collect();
    let managed = snapshot
        .tasks
        .iter()
        .filter(|task| ids.contains(task.id.as_str()))
        .filter_map(|task| {
            task.agent_worktree
                .as_ref()
                .map(|worktree| (&task.id, &task.cwd, worktree))
        })
        .collect::<Vec<_>>();
    for (task_id, cwd, worktree) in &managed {
        agent_worktree::can_retire(task_id, cwd, worktree)?;
    }
    for (task_id, cwd, worktree) in managed {
        agent_worktree::retire(task_id, cwd, worktree)?;
    }
    for task in &mut snapshot.tasks {
        if ids.contains(task.id.as_str()) {
            task.archived = true;
            task.updated_at = time;
        }
    }
    for task in &selection.matches {
        audit(snapshot, &task.id, "Chat auto-archived", format!("Inactive for more than {} days. Last activity: {}. Trigger: {}. History preserved; chat can be restored.", task.days, task.last_activity_at, caller.unwrap_or("recurring rule")), time);
    }
    Ok(())
}

#[cfg(test)]
#[path = "auto_archive_tests.rs"]
mod tests;
