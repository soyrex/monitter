//! Small worker-built inputs consumed by the desktop shell. Transcript bodies
//! and activity payloads publish directly to their retained child entities.
use serde_json::{Map, Value};

#[derive(Default, PartialEq)]
pub(crate) struct ShellInputs {
    metadata: Value,
    activity: Value,
}
impl ShellInputs {
    pub(crate) fn structural_eq(&self, other: &Self) -> bool { self.metadata == other.metadata }
    pub(crate) fn prepare(snapshot: &Value) -> Self {
        let mut inputs = Map::new();
        inputs.insert("settings".into(), snapshot["settings"].clone());
        for collection in ["tasks", "agents", "projects", "hosts", "channels"] {
            let rows = snapshot[collection].as_array().into_iter().flatten().map(|row| {
                let fields = row.as_object().into_iter().flatten().filter(|(key, value)| {
                    // Prompts and document bodies never participate in shell
                    // rendering. Task activity timestamps are represented by
                    // exact display labels and relative order below, since raw
                    // updatedAt advances on every streamed token flush.
                    !matches!(key.as_str(), "prompt" | "systemPrompt" | "instructions" | "text" | "description")
                        && !(collection == "tasks" && matches!(key.as_str(), "updatedAt" | "lastActivityAt"))
                        && !value.is_object() && !value.is_array()
                }).map(|(key, value)| (key.clone(), value.clone())).collect();
                Value::Object(fields)
            }).collect();
            inputs.insert(collection.into(), Value::Array(rows));
        }
        // The sidebar approval block reads only identity, pending status and
        // whether this is an input form. Payload bodies remain child-owned.
        inputs.insert("approvalRequests".into(), Value::Array(snapshot["approvalRequests"].as_array().into_iter().flatten()
            .map(|request| serde_json::json!({"id":request["id"], "taskId":request["taskId"],
                "status":request["status"], "hasInput":!request["input"].is_null()})).collect()));
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis().min(i64::MAX as u128) as i64;
        let mut ordered: Vec<_> = snapshot["tasks"].as_array().into_iter().flatten()
            .filter(|task| task["archived"] != true && task["internal"] != true).collect();
        let activity_at = |task: &&Value| task["updatedAt"].as_i64().filter(|at| *at > 0)
            .or_else(|| task["createdAt"].as_i64()).unwrap_or(0);
        ordered.sort_by_key(|task| std::cmp::Reverse(activity_at(task)));
        let activity = Value::Array(ordered.iter().map(|task| {
            let age = now.saturating_sub(activity_at(task)).max(0);
            // Match shell::relative_task_time rounding, so continuous tokens
            // stay "now" without invalidating the root on every timestamp.
            let minutes = age.saturating_add(30_000) / 60_000;
            let display = if minutes < 1 { "now".into() } else if minutes < 60 {
                format!("{minutes}m ago")
            } else { format!("{}h ago", minutes.saturating_add(30) / 60) };
            serde_json::json!([task["id"], display, age > 4 * 60 * 60 * 1000])
        }).collect());
        Self { metadata: Value::Object(inputs), activity }
    }
}
