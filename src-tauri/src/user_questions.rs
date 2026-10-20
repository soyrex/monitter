//! Harness-neutral, task-owned questions exposed through the existing MCP bridge.
//! Questions use durable input records, never permission rules or chat injection.
use crate::{
    Service,
    model::{ApprovalRequest, InputOption, InputQuestion, InteractionInput, Snapshot, id, now},
    runner::RunControl,
};
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

const PREFIX: &str = "user-question:";
const NATIVE_PREFIX: &str = "native-question:";
const WAIT: Duration = Duration::from_secs(20);

fn text(value: &Value, name: &str, max: usize, required: bool) -> Result<String, String> {
    let Some(value) = value.get(name) else {
        return if required {
            Err(format!("{name} is required."))
        } else {
            Ok(String::new())
        };
    };
    let value = value
        .as_str()
        .ok_or_else(|| format!("{name} must be text."))?;
    if value.len() > max || value.contains('\0') || (required && value.trim().is_empty()) {
        return Err(format!(
            "{name} must be nonempty text of at most {max} bytes."
        ));
    }
    Ok(value.into())
}

fn keys(value: &Value, allowed: &[&str]) -> Result<(), String> {
    if value
        .as_object()
        .is_none_or(|v| v.keys().any(|key| !allowed.contains(&key.as_str())))
    {
        return Err("Unsupported question fields.".into());
    }
    Ok(())
}

pub(crate) fn parse(args: &Value) -> Result<(String, String, InteractionInput), String> {
    if args.to_string().len() > 64 * 1024 {
        return Err("Question request exceeds 64 KiB.".into());
    }
    keys(args, &["request_id", "title", "questions"])?;
    let request_id = text(args, "request_id", 128, true)?;
    if request_id.chars().any(char::is_control) {
        return Err("Invalid request_id.".into());
    }
    let title = text(args, "title", 240, false)?;
    let rows = args["questions"]
        .as_array()
        .ok_or("questions must be an array.")?;
    if rows.is_empty() || rows.len() > 8 {
        return Err("Ask between one and eight questions.".into());
    }
    let mut ids = HashSet::new();
    let mut questions = Vec::new();
    for row in rows {
        keys(
            row,
            &[
                "id",
                "header",
                "question",
                "options",
                "isSecret",
                "selectionMode",
            ],
        )?;
        let id = text(row, "id", 128, true)?;
        if !ids.insert(id.clone()) || id.chars().any(char::is_control) {
            return Err("Question IDs must be distinct.".into());
        }
        let mode = row
            .get("selectionMode")
            .and_then(Value::as_str)
            .unwrap_or("single");
        if !matches!(mode, "single" | "multiple")
            || row.get("selectionMode").is_some_and(|v| !v.is_string())
        {
            return Err("Unsupported question selectionMode.".into());
        }
        if row.get("isSecret").is_some_and(|v| !v.is_boolean()) {
            return Err("isSecret must be a boolean.".into());
        }
        let mut options = Vec::new();
        let mut labels = HashSet::new();
        if let Some(rows) = row.get("options") {
            let rows = rows.as_array().ok_or("options must be an array.")?;
            if rows.len() > 32 {
                return Err("A question supports at most 32 options.".into());
            }
            for option in rows {
                keys(option, &["label", "description"])?;
                let label = text(option, "label", 1024, true)?;
                if !labels.insert(label.clone()) {
                    return Err("Option labels must be distinct.".into());
                }
                options.push(InputOption {
                    label,
                    description: text(option, "description", 2048, false)?,
                });
            }
        }
        questions.push(InputQuestion {
            id,
            header: text(row, "header", 240, false)?,
            question: text(row, "question", 8192, true)?,
            is_secret: row["isSecret"] == true,
            selection_mode: Some(mode.into()),
            options,
        });
    }
    Ok((
        request_id,
        if title.trim().is_empty() {
            "Your input is needed".into()
        } else {
            title
        },
        InteractionInput {
            kind: "questions".into(),
            questions,
            schema: None,
            url: None,
        },
    ))
}

impl Service {
    pub(crate) fn create_user_question_request(
        &self,
        task_id: &str,
        control: &Arc<RunControl>,
        expected_run: &str,
        expected_turn: Option<&str>,
        request_key: &str,
        summary: &str,
        detail: String,
        interaction: InteractionInput,
    ) -> Result<ApprovalRequest, String> {
        if request_key.is_empty()
            || request_key.len() > 1024
            || summary.len() > 1024
            || detail.len() > 128 * 1024
            || serde_json::to_vec(&interaction)
                .map_err(|e| e.to_string())?
                .len()
                > 64 * 1024
        {
            return Err("Unsupported or excessive user question.".into());
        }
        let key = format!(
            "{NATIVE_PREFIX}{expected_run}:{}",
            json!({"turn":expected_turn,"key":request_key})
        );
        let mut inserted = None;
        let result = self.mutate(Some(task_id.into()), |snapshot| {
            if control.is_cancelled()
                || !control.is_active()
                || control.current_run_id()? != expected_run
                || expected_turn.is_some_and(|turn| !control.matches_app_server_turn(turn))
                || !self
                    .runs
                    .lock()
                    .map_err(|_| "Run registry unavailable.")?
                    .tasks
                    .get(task_id)
                    .is_some_and(|c| Arc::ptr_eq(c, control))
            {
                return Err("This question belongs to an expired turn.".into());
            }
            let task = snapshot
                .tasks
                .iter()
                .find(|t| t.id == task_id && t.status == "running")
                .ok_or("Question task is not running.")?;
            if let Some(existing) = snapshot
                .approval_requests
                .iter()
                .find(|r| r.task_id == task_id && r.run_id == key)
            {
                if existing.input.as_ref() != Some(&interaction) || existing.detail != detail {
                    return Err("Native question ID was reused with different input.".into());
                }
                return Ok(existing.clone());
            }
            if snapshot
                .approval_requests
                .iter()
                .filter(|r| r.task_id == task_id && r.status == "pending" && r.input.is_some())
                .count()
                >= 8
            {
                return Err("Too many pending user questions.".into());
            }
            let request = ApprovalRequest {
                id: id(),
                task_id: task_id.into(),
                provider: task.provider.clone(),
                run_id: key,
                tool: "User input".into(),
                summary: summary.into(),
                detail,
                risk: "unknown".into(),
                status: "pending".into(),
                created_at: now(),
                resolved_at: None,
                decision: None,
                rememberable: false,
                session_scope: None,
                rule_id: None,
                approval_scope: None,
                input: Some(interaction),
                response: None,
            };
            self.app_server_approvals
                .lock()
                .map_err(|_| "Question ownership unavailable.")?
                .insert(request.id.clone(), Arc::downgrade(control));
            inserted = Some(request.id.clone());
            snapshot.approval_requests.push(request.clone());
            Ok(request)
        });
        if result.is_err() {
            if let Some(id) = inserted {
                if let Ok(mut owners) = self.app_server_approvals.lock() {
                    owners.remove(&id);
                }
            }
        }
        result
    }

    /// Exact runtime and generation checks also apply to non-Codex input records.
    pub(crate) fn validate_user_question_owner(
        &self,
        request: &ApprovalRequest,
        snapshot: &Snapshot,
    ) -> Result<(), String> {
        let native = request.run_id.starts_with(NATIVE_PREFIX);
        if !native && !request.run_id.starts_with(PREFIX) {
            return Ok(());
        }
        let (expected_run, metadata) = request
            .run_id
            .strip_prefix(if native { NATIVE_PREFIX } else { PREFIX })
            .and_then(|key| key.split_once(':'))
            .ok_or("Invalid question owner.")?;
        let metadata = if native {
            serde_json::from_str::<Value>(metadata).map_err(|_| "Invalid native question owner.")?
        } else {
            Value::Null
        };
        let runs = self.runs.lock().map_err(|_| "Run registry unavailable.")?;
        let owners = self
            .app_server_approvals
            .lock()
            .map_err(|_| "Question ownership unavailable.")?;
        let owner = owners
            .get(&request.id)
            .and_then(std::sync::Weak::upgrade)
            .ok_or("This question has expired.")?;
        if owner.is_cancelled()
            || !owner.is_active()
            || owner.current_run_id()? != expected_run
            || metadata["turn"]
                .as_str()
                .is_some_and(|turn| !owner.matches_app_server_turn(turn))
            || !runs
                .tasks
                .get(&request.task_id)
                .is_some_and(|current| Arc::ptr_eq(current, &owner))
            || !snapshot.tasks.iter().any(|task| {
                task.id == request.task_id
                    && task.provider == request.provider
                    && task.status == "running"
            })
            || (!native
                && !snapshot
                    .tasks
                    .iter()
                    .find(|task| task.id == request.task_id)
                    .is_some_and(|task| {
                        snapshot.agents.iter().any(|agent| {
                            agent.id == task.agent_id
                                && !agent.internal
                                && agent.collaboration_enabled
                        })
                    }))
        {
            return Err("This question no longer belongs to the active turn.".into());
        }
        Ok(())
    }

    pub(crate) fn ask_user_protocol(&self, task_id: &str, args: Value) -> Result<Value, String> {
        self.ask_user_with_wait(task_id, args, WAIT)
    }

    fn ask_user_with_wait(
        &self,
        task_id: &str,
        args: Value,
        wait: Duration,
    ) -> Result<Value, String> {
        let (request_key, title, input) = parse(&args)?;
        let (task, _) = self.require_collaboration_caller(task_id)?;
        let control = self
            .runs
            .lock()
            .map_err(|_| "Run registry unavailable.")?
            .tasks
            .get(task_id)
            .cloned()
            .ok_or("Questions require a live task runtime.")?;
        let run = control.current_run_id()?;
        let key = format!("{PREFIX}{run}:{request_key}");
        let mut inserted = None;
        let request = self.mutate(Some(task_id.into()), |snapshot| {
            if control.is_cancelled()
                || !control.is_active()
                || control.current_run_id()? != run
                || !self
                    .runs
                    .lock()
                    .map_err(|_| "Run registry unavailable.")?
                    .tasks
                    .get(task_id)
                    .is_some_and(|c| Arc::ptr_eq(c, &control))
                || !snapshot.tasks.iter().any(|t| {
                    t.id == task_id && t.provider == task.provider && t.status == "running"
                })
                || !snapshot
                    .agents
                    .iter()
                    .any(|a| a.id == task.agent_id && !a.internal && a.collaboration_enabled)
            {
                return Err("Questions require the current active turn.".into());
            }
            if let Some(existing) = snapshot
                .approval_requests
                .iter()
                .find(|r| r.task_id == task_id && r.run_id == key)
            {
                if existing.input.as_ref() != Some(&input) || existing.summary != title {
                    return Err("request_id was already used for different questions.".into());
                }
                return Ok(existing.clone());
            }
            if snapshot
                .approval_requests
                .iter()
                .any(|r| r.task_id == task_id && r.status == "pending" && r.input.is_some())
            {
                return Err(
                    "This task already has a question awaiting a response. Reuse its request_id."
                        .into(),
                );
            }
            if snapshot
                .approval_requests
                .iter()
                .filter(|r| {
                    r.task_id == task_id && r.run_id.starts_with(&format!("{PREFIX}{run}:"))
                })
                .count()
                >= 32
            {
                return Err("This turn has reached its limit of 32 question requests.".into());
            }
            let request = ApprovalRequest {
                id: id(),
                task_id: task_id.into(),
                provider: task.provider.clone(),
                run_id: key,
                tool: "User input".into(),
                summary: title,
                detail: String::new(),
                risk: "unknown".into(),
                status: "pending".into(),
                created_at: now(),
                resolved_at: None,
                decision: None,
                rememberable: false,
                session_scope: None,
                rule_id: None,
                approval_scope: None,
                input: Some(input),
                response: None,
            };
            self.app_server_approvals
                .lock()
                .map_err(|_| "Question ownership unavailable.")?
                .insert(request.id.clone(), Arc::downgrade(&control));
            inserted = Some(request.id.clone());
            snapshot.approval_requests.push(request.clone());
            Ok(request)
        });
        let request = match request {
            Ok(request) => request,
            Err(error) => {
                if let Some(id) = inserted {
                    if let Ok(mut owners) = self.app_server_approvals.lock() {
                        owners.remove(&id);
                    }
                }
                return Err(error);
            }
        };
        let started = Instant::now();
        loop {
            let data = self.committed_data()?;
            let current = data
                .snapshot
                .approval_requests
                .iter()
                .find(|r| r.id == request.id)
                .ok_or("Question no longer exists.")?;
            if let Err(error) = self.validate_user_question_owner(current, &data.snapshot) {
                drop(data);
                let _ = self.expire_approval_request(&request.id);
                return Err(error);
            }
            let result = match current.status.as_str() {
                "approved" => Some(
                    json!({"status":"answered","request_id":request_key,"approvalRequestId":current.id,"response":current.response}),
                ),
                "denied" => Some(
                    json!({"status":"cancelled","request_id":request_key,"approvalRequestId":current.id}),
                ),
                "pending" if started.elapsed() < wait => None,
                "pending" => Some(
                    json!({"status":"pending","request_id":request_key,"approvalRequestId":current.id,
                    "message":"Waiting for the user. Retry ask_user with exactly the same arguments to receive the answer. Do not create another question or infer an answer."}),
                ),
                _ => return Err("This question has expired.".into()),
            };
            if let Some(result) = result {
                return Ok(result);
            }
            drop(data);
            thread::sleep(Duration::from_millis(100));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ApprovalDecision, model::CreateTaskInput};
    fn fixture(provider: &str) -> (Arc<Service>, std::path::PathBuf, String, Arc<RunControl>) {
        let path = std::env::temp_dir().join(format!("monitter-user-question-{}", id()));
        let service = Service::open(None, path.clone()).unwrap();
        let agent = service
            .mutate(None, |s| {
                let a = &mut s.agents[0];
                a.collaboration_enabled = true;
                Ok(a.clone())
            })
            .unwrap();
        let task = service
            .create_task(CreateTaskInput {
                agent_id: agent.id,
                title: "Question test".into(),
                native_session_id: None,
                parent_task_id: None,
                channel_id: None,
                project_id: None,
                cwd: None,
                model_settings: None,
                sandbox: None,
            })
            .unwrap();
        service
            .mutate(None, |s| {
                let t = s.tasks.iter_mut().find(|t| t.id == task.id).unwrap();
                t.provider = provider.into();
                t.status = "running".into();
                Ok(())
            })
            .unwrap();
        let control = service.reserve_run(&task.id).unwrap();
        (service, path, task.id, control)
    }
    fn args() -> Value {
        json!({"request_id":"choice","questions":[{"id":"q","question":"Which option?","options":[{"label":"First"},{"label":"Second"}]}]})
    }
    #[test]
    fn user_question_validation_rejects_ambiguous_excessive_or_spoofed_payloads() {
        assert!(parse(&args()).is_ok());
        for mutation in [
            json!({"task_id":"other"}),
            json!({"request_id":""}),
            json!({"questions":[]}),
            json!({"questions":[{"id":"q","question":"q","selectionMode":"all"}]}),
            json!({"questions":[{"id":"q","question":"q","options":[{"label":"A"},{"label":"A"}]}]}),
        ] {
            let mut value = args();
            for (key, v) in mutation.as_object().unwrap() {
                value[key] = v.clone();
            }
            assert!(parse(&value).is_err());
        }
        let mut huge = args();
        huge["questions"][0]["question"] = json!("x".repeat(8193));
        assert!(parse(&huge).is_err());
    }
    #[test]
    fn user_question_all_harnesses_share_durable_answers_and_exact_retries() {
        for provider in ["codex", "acp", "claude", "opencode", "hermes"] {
            let (s, path, task, _owner) = fixture(provider);
            let first = s.ask_user_with_wait(&task, args(), Duration::ZERO).unwrap();
            assert_eq!(first["status"], "pending");
            assert_eq!(
                s.ask_user_with_wait(&task, args(), Duration::ZERO).unwrap(),
                first
            );
            let id = first["approvalRequestId"].as_str().unwrap();
            let response = json!({"answers":{"q":{"answers":["First"]}}});
            s.resolve_input_request(id, response.clone()).unwrap();
            let result = s.ask_user_with_wait(&task, args(), Duration::ZERO).unwrap();
            assert_eq!(result["status"], "answered");
            assert_eq!(result["response"], response);
            let mut changed = args();
            changed["questions"][0]["question"] = json!("Changed");
            assert!(
                s.ask_user_with_wait(&task, changed, Duration::ZERO)
                    .is_err()
            );
            let snap = s.snapshot().unwrap();
            assert_eq!(snap.approval_requests.len(), 1);
            assert!(!snap.approval_requests[0].rememberable);
            assert!(s.resolve_input_request(id, response).is_err());
            drop(s);
            let _ = std::fs::remove_dir_all(path);
        }
    }
    #[test]
    fn user_question_cancel_unblocks_and_is_not_an_answer_or_permission_grant() {
        let (s, path, task, _owner) = fixture("claude");
        let pending = s.ask_user_with_wait(&task, args(), Duration::ZERO).unwrap();
        let id = pending["approvalRequestId"].as_str().unwrap();
        assert!(
            s.resolve_approval_request(id, ApprovalDecision::ApproveOnce)
                .is_err()
        );
        let mut second = args();
        second["request_id"] = json!("other");
        assert!(s.ask_user_with_wait(&task, second, Duration::ZERO).is_err());
        s.resolve_approval_request(id, ApprovalDecision::Deny)
            .unwrap();
        assert_eq!(
            s.ask_user_with_wait(&task, args(), Duration::ZERO).unwrap()["status"],
            "cancelled"
        );
        assert!(s.snapshot().unwrap().approval_rules.is_empty());
        drop(s);
        let _ = std::fs::remove_dir_all(path);
    }
    #[test]
    fn user_question_replaced_runtime_and_revoked_capability_cannot_receive_answers() {
        for replace in [false, true] {
            let (s, path, task, _owner) = fixture("opencode");
            let pending = s.ask_user_with_wait(&task, args(), Duration::ZERO).unwrap();
            if replace {
                s.runs
                    .lock()
                    .unwrap()
                    .tasks
                    .insert(task.clone(), RunControl::new(false));
            } else {
                s.mutate(None, |s| {
                    for a in &mut s.agents {
                        a.collaboration_enabled = false;
                    }
                    Ok(())
                })
                .unwrap();
            }
            let id = pending["approvalRequestId"].as_str().unwrap();
            assert!(
                s.resolve_input_request(id, json!({"answers":{"q":{"answers":["First"]}}}))
                    .is_err()
            );
            assert_eq!(s.snapshot().unwrap().approval_requests[0].status, "pending");
            drop(s);
            let _ = std::fs::remove_dir_all(path);
        }
    }
}
