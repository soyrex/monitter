//! Pure projection for the new-tab existing-chat picker.
//!
//! This intentionally indexes only task metadata. Message bodies, transcripts,
//! and other private conversation contents are not part of picker search.

use serde_json::Value;
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionEntry {
    pub id: String,
    pub title: String,
    pub agent_name: String,
    pub project_name: String,
    /// Lowercase title/agent/project/model/cwd metadata for local filtering.
    pub search_text: String,
    pub updated_at: i64,
    pub status: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PickerGroups {
    pub running: Vec<SessionEntry>,
    pub recent: Vec<SessionEntry>,
}

/// Build running and recent picker groups from the current task, agent, and
/// project snapshots. Search is applied before returning either group, so an
/// old session remains discoverable regardless of any UI display limit.
pub fn build_entries(
    tasks: &[Value],
    agents: &[Value],
    projects: &[Value],
    query: &str,
) -> PickerGroups {
    let agent_names: HashMap<&str, &str> = agents
        .iter()
        .filter_map(|agent| {
            let id = nonempty(agent.get("id")?.as_str()?)?;
            let name = nonempty(agent.get("name")?.as_str()?)?;
            Some((id, name))
        })
        .collect();
    let project_names: HashMap<&str, &str> = projects
        .iter()
        .filter_map(|project| {
            let id = nonempty(project.get("id")?.as_str()?)?;
            let name = nonempty(project.get("name")?.as_str()?)?;
            Some((id, name))
        })
        .collect();
    let mut groups = PickerGroups::default();
    for task in tasks {
        if task.get("archived").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let Some(id) = task.get("id").and_then(Value::as_str).and_then(nonempty) else {
            continue;
        };

        let title = task
            .get("title")
            .and_then(Value::as_str)
            .and_then(nonempty)
            .unwrap_or("Untitled chat")
            .to_owned();
        let agent_name = task
            .get("agentId")
            .and_then(Value::as_str)
            .and_then(|agent_id| agent_names.get(agent_id).copied())
            .or_else(|| {
                task.get("archivedAgentName")
                    .and_then(Value::as_str)
                    .and_then(nonempty)
            })
            .unwrap_or("Agent")
            .to_owned();
        let project_name = task
            .get("projectId")
            .and_then(Value::as_str)
            .and_then(|project_id| project_names.get(project_id).copied())
            .unwrap_or_default()
            .to_owned();
        let model = task
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let cwd = task.get("cwd").and_then(Value::as_str).unwrap_or_default();
        let search_text =
            format!("{title}\n{agent_name}\n{project_name}\n{model}\n{cwd}").to_lowercase();

        let status = task
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let entry = SessionEntry {
            id: id.to_owned(),
            title,
            agent_name,
            project_name,
            search_text,
            updated_at: task
                .get("updatedAt")
                .and_then(Value::as_i64)
                .unwrap_or_default(),
            status,
        };
        if entry.status == "running" {
            groups.running.push(entry);
        } else {
            groups.recent.push(entry);
        }
    }

    groups.running.retain(|entry| matches_query(entry, query));
    groups.recent.retain(|entry| matches_query(entry, query));
    groups.running.sort_by(session_order);
    groups.recent.sort_by(session_order);
    groups
}

/// Match all whitespace-separated query terms against the entry's precomputed
/// lowercase metadata. This lets the UI filter entries as the user types
/// without retaining full task payloads.
pub fn matches_query(entry: &SessionEntry, query: &str) -> bool {
    query
        .split_whitespace()
        .map(str::to_lowercase)
        .all(|term| entry.search_text.contains(&term))
}

fn nonempty(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn session_order(left: &SessionEntry, right: &SessionEntry) -> std::cmp::Ordering {
    right
        .updated_at
        .cmp(&left.updated_at)
        .then_with(|| left.id.cmp(&right.id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn task(id: &str, status: &str, updated_at: i64) -> Value {
        json!({
            "id": id,
            "title": format!("Chat {id}"),
            "status": status,
            "updatedAt": updated_at,
        })
    }

    #[test]
    fn running_sessions_are_separate_and_newest_first() {
        let groups = build_entries(
            &[
                task("idle", "idle", 100),
                task("run-old", "running", 200),
                task("run-new", "running", 300),
            ],
            &[],
            &[],
            "",
        );
        assert_eq!(ids(&groups.running), ["run-new", "run-old"]);
        assert_eq!(ids(&groups.recent), ["idle"]);
    }

    #[test]
    fn archived_and_invalid_ids_are_excluded() {
        let groups = build_entries(
            &[
                task("visible", "idle", 1),
                json!({"id":"archived","archived":true,"status":"running"}),
                json!({"id":"","status":"idle"}),
                json!({"status":"idle"}),
            ],
            &[],
            &[],
            "",
        );
        assert!(groups.running.is_empty());
        assert_eq!(ids(&groups.recent), ["visible"]);
    }

    #[test]
    fn older_sessions_are_searched_without_a_recent_cap() {
        let mut tasks = (0..80)
            .map(|index| task(&format!("new-{index:02}"), "idle", 1000 - index))
            .collect::<Vec<_>>();
        tasks.push(
            json!({"id":"old","title":"Needle in the archive","status":"idle","updatedAt":1}),
        );
        let groups = build_entries(&tasks, &[], &[], "needle archive");
        assert_eq!(ids(&groups.recent), ["old"]);
    }

    #[test]
    fn all_terms_match_metadata_case_insensitively() {
        let groups = build_entries(
            &[
                json!({"id":"s","title":"Quarterly Review","status":"idle","updatedAt":1,"agentId":"a","projectId":"p","model":"Model-X","cwd":"/Users/alex/Work"}),
            ],
            &[json!({"id":"a","name":"Research Agent"})],
            &[json!({"id":"p","name":"Claims Portal"})],
            "  RESEARCH claims MODEL-X  ",
        );
        assert_eq!(groups.recent.len(), 1);
        assert_eq!(groups.recent[0].agent_name, "Research Agent");
        assert_eq!(groups.recent[0].project_name, "Claims Portal");
    }

    #[test]
    fn precomputed_entries_can_be_refiltered_without_task_payloads() {
        let groups = build_entries(
            &[
                json!({"id":"s","title":"Quarterly Review","status":"idle","updatedAt":1,"model":"Model-X","cwd":"/Users/alex/Work"}),
            ],
            &[],
            &[],
            "",
        );
        let entry = &groups.recent[0];
        assert!(matches_query(entry, "  MODEL-x work "));
        assert!(!matches_query(entry, "model missing"));
    }

    #[test]
    fn missing_agent_uses_archived_name_or_agent_fallback() {
        let groups = build_entries(
            &[
                json!({"id":"named","title":"","status":"idle","updatedAt":2,"archivedAgentName":"Former Agent"}),
                json!({"id":"missing","status":"idle","updatedAt":1}),
            ],
            &[],
            &[],
            "",
        );
        let named = groups
            .recent
            .iter()
            .find(|entry| entry.id == "named")
            .unwrap();
        assert_eq!(named.agent_name, "Former Agent");
        assert_eq!(named.title, "Untitled chat");
        assert_eq!(groups.recent[1].agent_name, "Agent");
    }

    #[test]
    fn timestamp_ties_use_stable_id_order() {
        let groups = build_entries(
            &[
                task("z", "idle", 10),
                task("a", "idle", 10),
                task("m", "idle", 10),
            ],
            &[],
            &[],
            "",
        );
        assert_eq!(ids(&groups.recent), ["a", "m", "z"]);
    }

    #[test]
    fn a_task_appears_in_only_one_group() {
        let groups = build_entries(&[task("same", "running", 5)], &[], &[], "");
        assert_eq!(groups.running.len(), 1);
        assert!(groups.recent.is_empty());
        assert_eq!(groups.running[0].id, "same");
    }

    fn ids(entries: &[SessionEntry]) -> Vec<&str> {
        entries.iter().map(|entry| entry.id.as_str()).collect()
    }
}
