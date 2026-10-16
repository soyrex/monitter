//! Native, draft-first chat creation surface.
//!
//! The pane owns only transient UI state. It never calls `create_task` or
//! `send_message`; pressing Send emits a submission for the shell to execute.
//! The caller should retain this entity across service failures and call
//! `set_error` so the user's selections and message remain in place.
use crate::design::ui_scaled_px as px;
use gpui_kit::{
    component::popover::Popover,
    component::{Disableable, Sizable},
    component::{
        button::*,
        input::{Input, InputEvent, InputState, Textarea, TextareaState},
    },
    prelude::*,
    *,
};
use monitter_lib::experimental_desktop::DesktopService;
use serde_json::{Value, json};
use std::{collections::HashMap, rc::Rc};

const MAX_DRAFT_MESSAGE_BYTES: usize = 32 * 1024;
const MAX_DRAFT_TEXT_FIELD_BYTES: usize = 8 * 1024;
const MAX_DRAFT_ID_BYTES: usize = 256;
const MAX_DRAFT_MODEL_CACHES: usize = 64;
const MAX_DRAFT_STATE_BYTES: usize = 128 * 1024;

fn compact_choice_label(label: impl Into<SharedString>) -> AnyElement {
    div()
        .text_size(px(11.))
        .line_height(relative(1.2))
        .child(label.into())
        .into_any_element()
}

fn compact_choice_button(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Button {
    let label = label.into();
    Button::new(id)
        .small()
        .compact()
        .px(px(9.))
        .py(px(5.))
        .accessibility_label(label.clone())
        .child(compact_choice_label(label))
}

fn field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}
fn rows(snapshot: &Value, key: &str) -> Vec<Value> {
    let values = snapshot
        .get(key)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if key == "agents" {
        values
            .into_iter()
            .filter(|agent| agent.get("internal").and_then(Value::as_bool) != Some(true))
            .collect()
    } else {
        values
    }
}
fn suggested_cwd(snapshot: &Value, agent_id: &str, project_id: &str) -> String {
    let agent = rows(snapshot, "agents")
        .into_iter()
        .find(|agent| field(agent, "id") == agent_id);
    let host_id = agent
        .as_ref()
        .map(|agent| field(agent, "hostId"))
        .unwrap_or_default();
    let workspace = rows(snapshot, "projects")
        .into_iter()
        .find(|project| field(project, "id") == project_id)
        .and_then(|project| project.get("workspaces").and_then(Value::as_array).cloned())
        .unwrap_or_default()
        .into_iter()
        .find(|workspace| field(workspace, "hostId") == host_id);
    workspace
        .map(|workspace| field(&workspace, "cwd"))
        .filter(|cwd| !cwd.is_empty())
        .or_else(|| {
            agent
                .as_ref()
                .map(|agent| field(agent, "cwd"))
                .filter(|cwd| !cwd.is_empty())
        })
        .or_else(|| {
            rows(snapshot, "hosts")
                .into_iter()
                .find(|host| field(host, "id") == host_id)
                .map(|host| field(&host, "defaultCwd"))
        })
        .unwrap_or_default()
}

fn permission_capabilities(agent: &Value) -> (bool, bool, bool) {
    let provider = field(agent, "provider");
    let launch = field(agent.get("acp").unwrap_or(&Value::Null), "command").to_lowercase();
    let key = if provider == "claude" {
        "claude-code"
    } else if provider == "codex" {
        "codex"
    } else if provider == "hermes" || (provider == "acp" && launch.contains("hermes")) {
        "hermes"
    } else if provider == "acp" && launch.rsplit(['/', '\\']).next() == Some("omp") {
        "omp"
    } else if provider == "acp" && (launch.contains("gemini") || launch.contains("agy")) {
        "gemini"
    } else if provider == "acp" && (launch.contains("mcode") || launch.contains("minimax")) {
        "minimax"
    } else if provider == "acp" && launch.contains("mona") {
        "mona"
    } else {
        "opencode"
    };
    // Keep these capabilities aligned with src/lib/agent-permissions.ts.
    match key {
        "hermes" | "omp" => (false, true, false),
        "gemini" => (false, true, true),
        _ => (true, true, true),
    }
}

/// A complete first-turn request. `task_input` follows `create_task`'s
/// `input` contract; `text` is sent to `send_message` only after task creation
/// succeeds. The parent is responsible for ordering those two calls.
#[derive(Clone, Debug)]
pub struct DraftSubmission {
    pub task_input: Value,
    pub text: String,
    /// Set after create_task succeeded but send_message failed. Retries must
    /// reuse this task ID and must never create a second task.
    pub task_id: Option<String>,
}

#[derive(Clone, Debug)]
pub enum DraftCreationEvent {
    /// Form data changed through a user edit or selection. The owner may
    /// debounce a local-only autosave from this signal.
    Changed,
    Send(DraftSubmission),
    Close,
}

impl EventEmitter<DraftCreationEvent> for DraftCreationPane {}

/// Serializable owner-facing snapshot of one unsent chat form. Persistence is
/// intentionally owned by the workspace shell, not this widget.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DraftFormState {
    pub agent_id: String,
    pub project_id: String,
    pub parent_task_id: Option<String>,
    pub title: String,
    pub cwd: String,
    pub message: String,
    pub sandbox: String,
    pub selected_model: String,
    pub selected_effort: Option<String>,
    pub selected_fast_mode: Option<bool>,
    pub model_drafts: HashMap<String, DraftModelSelection>,
    pub created_task_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DraftModelSelection {
    pub model: String,
    pub effort: Option<String>,
    pub fast_mode: Option<bool>,
}

pub struct DraftCreationPane {
    service: DesktopService,
    snapshot: Rc<Value>,
    agent_id: String,
    project_id: String,
    parent_task_id: Option<String>,
    sandbox: String,
    error: Option<String>,
    events: Vec<DraftCreationEvent>,
    busy: bool,
    created_task_id: Option<String>,
    catalog_key: Option<String>,
    catalog_loading_key: Option<String>,
    catalog_error: Option<String>,
    catalog: Option<Value>,
    selected_model: String,
    selected_effort: Option<String>,
    selected_fast_mode: Option<bool>,
    model_drafts: HashMap<String, (String, Option<String>, Option<bool>)>,
    agent_search: Entity<InputState>,
    project_search: Entity<InputState>,
    picker_open: Option<&'static str>,
    options_open: bool,
    native_session: Entity<InputState>,
    title: Entity<InputState>,
    cwd: Entity<InputState>,
    message: Entity<TextareaState>,
    _input_subscriptions: Vec<Subscription>,
}

impl Focusable for DraftCreationPane {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.message.read(cx).focus_handle(cx)
    }
}

fn validate_draft_state(state: &DraftFormState, snapshot: &Value) -> Result<(), String> {
    let check_len = |name: &str, value: &str, max: usize| {
        if value.len() > max {
            Err(format!(
                "Saved chat draft {name} exceeds its {max}-byte limit."
            ))
        } else {
            Ok(())
        }
    };

    check_len("agent ID", &state.agent_id, MAX_DRAFT_ID_BYTES)?;
    check_len("project ID", &state.project_id, MAX_DRAFT_ID_BYTES)?;
    check_len("title", &state.title, MAX_DRAFT_TEXT_FIELD_BYTES)?;
    check_len("working folder", &state.cwd, MAX_DRAFT_TEXT_FIELD_BYTES)?;
    check_len("message", &state.message, MAX_DRAFT_MESSAGE_BYTES)?;
    check_len("access mode", &state.sandbox, MAX_DRAFT_ID_BYTES)?;
    check_len("model ID", &state.selected_model, MAX_DRAFT_ID_BYTES)?;
    if let Some(id) = &state.parent_task_id {
        check_len("parent task ID", id, MAX_DRAFT_ID_BYTES)?;
    }
    if let Some(id) = &state.created_task_id {
        check_len("created task ID", id, MAX_DRAFT_ID_BYTES)?;
        if id.is_empty() {
            return Err("Saved chat draft has an empty created task ID.".into());
        }
    }
    if let Some(effort) = &state.selected_effort {
        check_len("reasoning effort", effort, MAX_DRAFT_ID_BYTES)?;
    }
    if state.model_drafts.len() > MAX_DRAFT_MODEL_CACHES {
        return Err("Saved chat draft has too many model selections.".into());
    }

    let agents = rows(snapshot, "agents");
    let projects = rows(snapshot, "projects");
    if !state.agent_id.is_empty()
        && !agents
            .iter()
            .any(|agent| field(agent, "id") == state.agent_id)
    {
        return Err("Saved chat draft agent is no longer available.".into());
    }
    if !state.project_id.is_empty()
        && !projects
            .iter()
            .any(|project| field(project, "id") == state.project_id)
    {
        return Err("Saved chat draft project is no longer available.".into());
    }
    if let Some(parent_id) = &state.parent_task_id {
        if !snapshot["tasks"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|task| field(task, "id") == *parent_id)
        {
            return Err("Saved chat draft parent is no longer available.".into());
        }
    }
    if let Some(created_id) = &state.created_task_id {
        let task = rows(snapshot, "tasks")
            .into_iter()
            .find(|task| field(task, "id") == *created_id)
            .ok_or_else(|| "Saved chat draft task is no longer available.".to_owned())?;
        if !created_task_matches_draft(&task, state, snapshot) {
            return Err(
                "Saved chat draft task no longer matches its agent, project, or working folder."
                    .into(),
            );
        }
    }

    let mut total_bytes = state.agent_id.len()
        + state.project_id.len()
        + state.parent_task_id.as_deref().unwrap_or_default().len()
        + state.title.len()
        + state.cwd.len()
        + state.message.len()
        + state.sandbox.len()
        + state.selected_model.len()
        + state.selected_effort.as_deref().unwrap_or_default().len()
        + state.created_task_id.as_deref().unwrap_or_default().len();
    for (key, selection) in &state.model_drafts {
        check_len("model selection key", key, MAX_DRAFT_ID_BYTES * 2 + 1)?;
        check_len("cached model ID", &selection.model, MAX_DRAFT_ID_BYTES)?;
        if let Some(effort) = &selection.effort {
            check_len("cached reasoning effort", effort, MAX_DRAFT_ID_BYTES)?;
        }
        let known_identity = agents.iter().any(|agent| {
            let agent_id = field(agent, "id");
            key.strip_prefix(&format!("{agent_id}:"))
                .is_some_and(|project_id| {
                    project_id.is_empty()
                        || projects
                            .iter()
                            .any(|project| field(project, "id") == project_id)
                })
        });
        if !known_identity {
            return Err(
                "Saved chat draft contains a model selection for an unknown agent/project.".into(),
            );
        }
        total_bytes += key.len()
            + selection.model.len()
            + selection.effort.as_deref().unwrap_or_default().len();
    }
    if total_bytes > MAX_DRAFT_STATE_BYTES {
        return Err("Saved chat draft exceeds its total size limit.".into());
    }
    Ok(())
}

fn created_task_matches_draft(task: &Value, state: &DraftFormState, snapshot: &Value) -> bool {
    if field(task, "agentId") != state.agent_id {
        return false;
    }
    let task_project = task
        .get("projectId")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if task_project != state.project_id {
        return false;
    }
    let expected_cwd = if !state.project_id.is_empty() {
        suggested_cwd(snapshot, &state.agent_id, &state.project_id)
    } else if !state.cwd.trim().is_empty() {
        state.cwd.trim().to_owned()
    } else {
        suggested_cwd(snapshot, &state.agent_id, "")
    };
    field(task, "cwd") == expected_cwd
}

impl DraftCreationPane {
    pub fn new(
        service: DesktopService,
        snapshot: Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let agents = rows(&snapshot, "agents");
        let agent_id = agents
            .first()
            .map(|agent| field(agent, "id"))
            .unwrap_or_default();
        let agent = agents.iter().find(|agent| field(agent, "id") == agent_id);
        let sandbox = agent
            .map(|agent| field(agent, "sandbox"))
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "read-only".into());
        let cwd = cx.new(|cx| InputState::new(window, cx).placeholder("Working folder"));
        cwd.update(cx, |input, cx| {
            input.set_value(suggested_cwd(&snapshot, &agent_id, ""), window, cx)
        });
        let agent_search = cx.new(|cx| InputState::new(window, cx).placeholder("Search agents…"));
        let project_search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search projects…"));
        let native_session =
            cx.new(|cx| InputState::new(window, cx).placeholder("Resume an idle native session"));
        let agent_search_changes =
            cx.subscribe(&agent_search, |_, _, _: &InputEvent, cx| cx.notify());
        let project_search_changes =
            cx.subscribe(&project_search, |_, _, _: &InputEvent, cx| cx.notify());
        let title = cx.new(|cx| InputState::new(window, cx).placeholder("Optional task title"));
        let message = cx.new(|cx| {
            TextareaState::new(window, cx).placeholder("Describe what you want to work on…")
        });
        let title_changes = cx.subscribe(&title, |pane, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                pane.emit_changed(cx);
            }
        });
        let cwd_changes = cx.subscribe(&cwd, |pane, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                pane.emit_changed(cx);
            }
        });
        let message_changes = cx.subscribe(&message, |pane, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                pane.emit_changed(cx);
            }
            if matches!(
                event,
                InputEvent::PressEnter {
                    secondary: true,
                    shift: false
                }
            ) {
                pane.send(cx);
            }
        });
        Self {
            service,
            snapshot: Rc::new(snapshot),
            agent_id,
            project_id: String::new(),
            parent_task_id: None,
            sandbox,
            error: None,
            events: Vec::new(),
            busy: false,
            created_task_id: None,
            catalog_key: None,
            catalog_loading_key: None,
            catalog_error: None,
            catalog: None,
            selected_model: String::new(),
            selected_effort: None,
            selected_fast_mode: None,
            model_drafts: HashMap::new(),
            agent_search,
            project_search,
            native_session,
            picker_open: None,
            options_open: false,
            title,
            cwd,
            message,
            _input_subscriptions: vec![
                title_changes,
                cwd_changes,
                message_changes,
                agent_search_changes,
                project_search_changes,
            ],
        }
    }

    fn emit_changed(&mut self, cx: &mut Context<Self>) {
        let event = DraftCreationEvent::Changed;
        self.events.push(event.clone());
        cx.emit(event);
    }

    fn emit_changed_if(&mut self, changed: bool, cx: &mut Context<Self>) {
        if changed {
            self.emit_changed(cx);
        }
    }

    /// Initialize this new form from the exact route context. A parent chat
    /// supplies its agent/project when explicit route values are absent.
    pub fn initialize_context(
        &mut self,
        agent_id: Option<&str>,
        project_id: Option<&str>,
        parent_task_id: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if self.busy || self.created_task_id.is_some() {
            return Err("This draft has already started submission.".into());
        }

        let parent = parent_task_id
            .map(|id| {
                rows(&self.snapshot, "tasks")
                    .into_iter()
                    .find(|task| field(task, "id") == id)
                    .ok_or_else(|| "The parent chat is no longer available.".to_owned())
            })
            .transpose()?;
        let requested_agent_id = agent_id
            .or_else(|| {
                parent
                    .as_ref()
                    .map(|task| task["agentId"].as_str().unwrap_or_default())
            })
            .filter(|id| !id.is_empty())
            .map(str::to_owned);
        if let Some(id) = requested_agent_id.as_deref() {
            if !rows(&self.snapshot, "agents")
                .iter()
                .any(|agent| field(agent, "id") == id)
            {
                return Err("The selected agent is no longer available.".into());
            }
        }
        let next_agent_id = requested_agent_id
            .or_else(|| {
                rows(&self.snapshot, "agents")
                    .first()
                    .map(|agent| field(agent, "id"))
            })
            .unwrap_or_default();
        let selected_agent = rows(&self.snapshot, "agents")
            .into_iter()
            .find(|agent| field(agent, "id") == next_agent_id);
        let next_project_id = project_id
            .or_else(|| parent.as_ref().and_then(|task| task["projectId"].as_str()))
            .unwrap_or_default()
            .to_owned();
        if !next_project_id.is_empty()
            && !rows(&self.snapshot, "projects")
                .iter()
                .any(|project| field(project, "id") == next_project_id)
        {
            return Err("The selected project is no longer available.".into());
        }

        self.save_model_draft();
        self.agent_id = next_agent_id;
        self.project_id = next_project_id;
        self.parent_task_id = parent_task_id.map(str::to_owned);
        self.sandbox = selected_agent
            .as_ref()
            .map(|agent| field(agent, "sandbox"))
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "read-only".into());
        let cwd = suggested_cwd(&self.snapshot, &self.agent_id, &self.project_id);
        self.cwd
            .update(cx, |input, cx| input.set_value(cwd, window, cx));
        let identity = self.catalog_identity();
        let (model, effort, fast) = self
            .model_drafts
            .get(&identity)
            .cloned()
            .unwrap_or_default();
        self.selected_model = model;
        self.selected_effort = effort;
        self.selected_fast_mode = fast;
        self.catalog = None;
        self.catalog_key = None;
        self.catalog_loading_key = None;
        self.catalog_error = None;
        self.error = None;
        cx.notify();
        Ok(())
    }

    /// Capture all user-authored form values for the pane owner to persist.
    /// Oversize state fails visibly rather than being truncated.
    pub fn export_state(&self, cx: &App) -> Result<DraftFormState, String> {
        let model_drafts = self
            .model_drafts
            .iter()
            .map(|(key, (model, effort, fast_mode))| {
                (
                    key.clone(),
                    DraftModelSelection {
                        model: model.clone(),
                        effort: effort.clone(),
                        fast_mode: *fast_mode,
                    },
                )
            })
            .collect();
        let state = DraftFormState {
            agent_id: self.agent_id.clone(),
            project_id: self.project_id.clone(),
            parent_task_id: self.parent_task_id.clone(),
            title: self.title.read(cx).value().to_string(),
            cwd: self.cwd.read(cx).value().to_string(),
            message: self.message.read(cx).value().to_string(),
            sandbox: self.sandbox.clone(),
            selected_model: self.selected_model.clone(),
            selected_effort: self.selected_effort.clone(),
            selected_fast_mode: self.selected_fast_mode,
            model_drafts,
            created_task_id: self.created_task_id.clone(),
        };
        validate_draft_state(&state, &self.snapshot)?;
        Ok(state)
    }

    /// Restore only a complete, bounded state whose agent/project/parent
    /// identities still exist in the current authoritative snapshot.
    pub fn restore_state(
        &mut self,
        state: DraftFormState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        validate_draft_state(&state, &self.snapshot)?;
        if self.busy {
            return Err("Cannot restore a draft while submission is in progress.".into());
        }

        self.agent_id = state.agent_id;
        self.project_id = state.project_id;
        self.parent_task_id = state.parent_task_id;
        self.sandbox = state.sandbox;
        self.selected_model = state.selected_model;
        self.selected_effort = state.selected_effort;
        self.selected_fast_mode = state.selected_fast_mode;
        self.model_drafts = state
            .model_drafts
            .into_iter()
            .map(|(key, selection)| {
                (
                    key,
                    (selection.model, selection.effort, selection.fast_mode),
                )
            })
            .collect();
        self.created_task_id = state.created_task_id;
        self.title
            .update(cx, |input, cx| input.set_value(state.title, window, cx));
        self.cwd
            .update(cx, |input, cx| input.set_value(state.cwd, window, cx));
        self.message
            .update(cx, |input, cx| input.set_value(state.message, window, cx));
        self.catalog = None;
        self.catalog_key = None;
        self.catalog_loading_key = None;
        self.catalog_error = None;
        self.error = None;
        cx.notify();
        Ok(())
    }

    /// Replace available agents and projects after a service snapshot update.
    /// Existing selections are retained while still present.
    pub fn set_snapshot(&mut self, snapshot: Rc<Value>, cx: &mut Context<Self>) {
        let changed = ["agents", "projects", "hosts", "settings", "nativeSessions"]
            .iter().any(|key| self.snapshot[*key] != snapshot[*key])
            || [self.created_task_id.as_ref(), self.parent_task_id.as_ref()].into_iter().flatten().any(|id| {
                let task = |value: &Value| value["tasks"].as_array().into_iter().flatten()
                    .find(|task| task["id"].as_str() == Some(id)).cloned();
                task(&self.snapshot) != task(&snapshot)
            });
        self.snapshot = snapshot;
        if !changed { return; }
        if let Some(created_id) = self.created_task_id.clone() {
            if let Some(task) = rows(&self.snapshot, "tasks")
                .into_iter()
                .find(|task| field(task, "id") == created_id)
            {
                let matches = self
                    .current_draft_state(cx)
                    .is_ok_and(|state| created_task_matches_draft(&task, &state, &self.snapshot));
                if !matches {
                    self.created_task_id = None;
                    self.error = Some(
                        "The created chat does not match this draft. Retry was disabled.".into(),
                    );
                }
            }
        }
        if !rows(&self.snapshot, "agents")
            .iter()
            .any(|agent| field(agent, "id") == self.agent_id)
        {
            self.agent_id = rows(&self.snapshot, "agents")
                .first()
                .map(|agent| field(agent, "id"))
                .unwrap_or_default();
        }
        if !self.project_id.is_empty()
            && !rows(&self.snapshot, "projects")
                .iter()
                .any(|project| field(project, "id") == self.project_id)
        {
            self.project_id.clear();
        }
        cx.notify();
    }

    /// Select an agent from a sidebar shortcut and refresh the model catalog.
    pub fn select_agent(&mut self, agent_id: String, window: &mut Window, cx: &mut Context<Self>) {
        if !rows(&self.snapshot, "agents")
            .iter()
            .any(|agent| field(agent, "id") == agent_id)
        {
            return;
        }
        let changed = self.agent_id != agent_id;
        if !changed {
            return;
        }
        self.save_model_draft();
        self.agent_id = agent_id;
        self.sandbox = self
            .selected_agent()
            .map(|agent| field(&agent, "sandbox"))
            .unwrap_or_default();
        let cwd = suggested_cwd(&self.snapshot, &self.agent_id, &self.project_id);
        self.cwd
            .update(cx, |input, cx| input.set_value(cwd, window, cx));
        self.error = None;
        self.emit_changed(cx);
        cx.notify();
    }

    /// Link an unsent child draft without creating a task or starting a run.
    /// Existing draft text is never silently reassigned to another parent.
    pub fn set_parent_task(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if self.busy || self.created_task_id.is_some() {
            return Err("Finish the existing draft before delegating another task.".into());
        }
        if self.parent_task_id.as_deref() != Some(id)
            && !self.message.read(cx).value().trim().is_empty()
        {
            return Err("Your existing unsent draft is preserved. Send or clear it before opening a delegated task.".into());
        }
        let parent = rows(&self.snapshot, "tasks")
            .into_iter()
            .find(|task| field(task, "id") == id)
            .ok_or_else(|| "The parent chat is no longer available.".to_owned())?;
        let changed = self.parent_task_id.as_deref() != Some(id)
            || self.project_id != field(&parent, "projectId")
            || self.cwd.read(cx).value().as_ref() != field(&parent, "cwd");
        self.parent_task_id = Some(id.to_owned());
        self.project_id = field(&parent, "projectId");
        let cwd = field(&parent, "cwd");
        self.cwd
            .update(cx, |input, cx| input.set_value(cwd, window, cx));
        self.emit_changed_if(changed, cx);
        cx.notify();
        Ok(())
    }

    fn save_model_draft(&mut self) {
        self.model_drafts.insert(
            self.catalog_identity(),
            (
                self.selected_model.clone(),
                self.selected_effort.clone(),
                self.selected_fast_mode,
            ),
        );
    }

    /// Drain events emitted since the previous call.
    pub fn take_events(&mut self) -> Vec<DraftCreationEvent> {
        std::mem::take(&mut self.events)
    }

    /// Clear or display the result of the parent's create/send operation.
    /// On error, all editor and selection state remains untouched.
    pub fn set_error(&mut self, error: Option<String>, cx: &mut Context<Self>) {
        self.error = error;
        cx.notify();
    }

    /// Lock the form while the parent creates/sends the first turn. The local
    /// guard also prevents a second click before the parent receives the event.
    pub fn set_busy(&mut self, busy: bool, cx: &mut Context<Self>) {
        self.busy = busy;
        cx.notify();
    }

    /// Retain a task created before a failed first message so a retry sends to
    /// that task instead of creating another durable task.
    pub fn set_created_task_id(&mut self, task_id: Option<String>, cx: &mut Context<Self>) {
        let requested = task_id;
        let accepted = requested.clone().filter(|id| {
            id.len() <= MAX_DRAFT_ID_BYTES
                && !id.is_empty()
                && rows(&self.snapshot, "tasks")
                    .into_iter()
                    .find(|task| field(task, "id") == *id)
                    .map_or(true, |task| {
                        self.current_draft_state(cx).is_ok_and(|state| {
                            created_task_matches_draft(&task, &state, &self.snapshot)
                        })
                    })
        });
        let rejected = requested.is_some() && accepted.is_none();
        if self.created_task_id == accepted {
            if rejected {
                self.error =
                    Some("The created chat does not match this draft. Retry was disabled.".into());
                cx.notify();
            }
            return;
        }
        self.created_task_id = accepted;
        if rejected {
            self.error =
                Some("The created chat does not match this draft. Retry was disabled.".into());
        }
        self.emit_changed(cx);
        cx.notify();
    }

    fn current_draft_state(&self, cx: &App) -> Result<DraftFormState, String> {
        let model_drafts = self
            .model_drafts
            .iter()
            .map(|(key, (model, effort, fast_mode))| {
                (
                    key.clone(),
                    DraftModelSelection {
                        model: model.clone(),
                        effort: effort.clone(),
                        fast_mode: *fast_mode,
                    },
                )
            })
            .collect();
        Ok(DraftFormState {
            agent_id: self.agent_id.clone(),
            project_id: self.project_id.clone(),
            parent_task_id: self.parent_task_id.clone(),
            title: self.title.read(cx).value().to_string(),
            cwd: self.cwd.read(cx).value().to_string(),
            message: self.message.read(cx).value().to_string(),
            sandbox: self.sandbox.clone(),
            selected_model: self.selected_model.clone(),
            selected_effort: self.selected_effort.clone(),
            selected_fast_mode: self.selected_fast_mode,
            model_drafts,
            created_task_id: None,
        })
    }

    fn selected_agent(&self) -> Option<Value> {
        rows(&self.snapshot, "agents")
            .into_iter()
            .find(|agent| field(agent, "id") == self.agent_id)
    }

    fn send(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(agent) = self.selected_agent() else {
            self.error = Some("Choose an available agent before sending.".into());
            cx.notify();
            return;
        };
        let text = self.message.read(cx).value().to_string();
        if text.trim().is_empty() {
            self.error = Some("Write a message before sending.".into());
            cx.notify();
            return;
        }
        let project_id = (!self.project_id.is_empty()).then(|| self.project_id.clone());
        if self.parent_task_id.as_ref().is_some_and(|id| {
            !rows(&self.snapshot, "tasks")
                .iter()
                .any(|task| field(task, "id") == *id)
        }) {
            self.error = Some(
                "The parent chat is no longer available. Remove the parent link before sending."
                    .into(),
            );
            cx.notify();
            return;
        }
        let cwd = self.cwd.read(cx).value().to_string();
        let title = self.title.read(cx).value().to_string();
        let (chosen_model, selected_effort, selected_fast_mode) = self
            .model_drafts
            .get(&self.catalog_identity())
            .cloned()
            .unwrap_or_else(|| {
                (
                    self.selected_model.clone(),
                    self.selected_effort.clone(),
                    self.selected_fast_mode,
                )
            });
        let model_settings = (!chosen_model.is_empty() || selected_effort.is_some() || selected_fast_mode.is_some()).then(
            || json!({"model": chosen_model, "reasoningEffort": selected_effort, "fastMode": selected_fast_mode}),
        );
        let sandbox = if self.sandbox.is_empty() {
            field(&agent, "sandbox")
        } else {
            self.sandbox.clone()
        };
        let (can_read_only, _, can_full_access) = permission_capabilities(&agent);
        let sandbox = if (sandbox == "read-only" && !can_read_only)
            || (sandbox == "yolo" && !can_full_access)
        {
            if field(&agent, "provider") == "codex" {
                "workspace-write".to_owned()
            } else {
                "harness-configured".to_owned()
            }
        } else {
            sandbox
        };
        let working_directory = if project_id.is_some() || cwd.trim().is_empty() {
            Value::Null
        } else {
            json!(cwd.trim())
        };
        let task_input = json!({
            "agentId": self.agent_id,
            "title": if title.trim().is_empty() { "New chat" } else { title.trim() },
            "projectId": project_id,
            "cwd": working_directory,
            "sandbox": sandbox,
            "modelSettings": model_settings,
            "nativeSessionId": if self.native_session.read(cx).value().trim().is_empty() { Value::Null } else { json!(self.native_session.read(cx).value().trim()) },
            "parentTaskId": self.parent_task_id,
            "channelId": Value::Null
        });
        self.error = None;
        let event = DraftCreationEvent::Send(DraftSubmission {
            task_input,
            text,
            task_id: self.created_task_id.clone(),
        });
        self.busy = true;
        self.events.push(event.clone());
        cx.emit(event);
        cx.notify();
    }

    fn catalog_identity(&self) -> String {
        format!("{}:{}", self.agent_id, self.project_id)
    }

    fn select_model(&mut self, model_id: String, cx: &mut Context<Self>) {
        let previous = (
            self.selected_model.clone(),
            self.selected_effort.clone(),
            self.selected_fast_mode,
        );
        self.selected_effort = self
            .catalog
            .as_ref()
            .and_then(|catalog| catalog["models"].as_array())
            .and_then(|models| models.iter().find(|model| field(model, "id") == model_id))
            .and_then(|model| model["defaultEffort"].as_str())
            .map(str::to_owned);
        self.selected_model = model_id;
        self.selected_fast_mode = None;
        self.save_model_draft();
        self.emit_changed_if(
            previous
                != (
                    self.selected_model.clone(),
                    self.selected_effort.clone(),
                    self.selected_fast_mode,
                ),
            cx,
        );
        cx.notify();
    }

    fn select_effort(&mut self, effort: Option<String>, cx: &mut Context<Self>) {
        if self.selected_effort == effort {
            return;
        }
        self.selected_effort = effort;
        self.save_model_draft();
        self.emit_changed(cx);
        cx.notify();
    }

    fn select_fast_mode(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.selected_fast_mode == Some(enabled) {
            return;
        }
        self.selected_fast_mode = Some(enabled);
        self.save_model_draft();
        self.emit_changed(cx);
        cx.notify();
    }

    fn request_catalog(&mut self, cx: &mut Context<Self>) {
        self.request_catalog_refresh(false, cx);
    }

    fn request_catalog_refresh(&mut self, force: bool, cx: &mut Context<Self>) {
        let Some(agent) = self.selected_agent() else {
            return;
        };
        let key = self.catalog_identity();
        if (!force && self.catalog_key.as_deref() == Some(&key))
            || self.catalog_loading_key.as_deref() == Some(&key)
        {
            return;
        }
        let had_draft = force || self.model_drafts.contains_key(&key);
        if !force {
            if let Some((model, effort, fast)) = self.model_drafts.get(&key).cloned() {
                self.selected_model = model;
                self.selected_effort = effort;
                self.selected_fast_mode = fast;
            }
        }
        let target = json!({"agentId":self.agent_id,"projectId":if self.project_id.is_empty(){Value::Null}else{json!(self.project_id)},"codexHome":agent.get("codexHome").cloned().unwrap_or(Value::Null),"refresh":force});
        let service = self.service.clone();
        self.catalog_loading_key = Some(key.clone());
        self.catalog_error = None;
        if !force { self.catalog = None; }
        let task = cx
            .background_executor()
            .spawn(async move { service.invoke("get_model_catalog", json!({"target":target})) });
        cx.spawn(async move |view, cx| {
            let result = task.await;
            let _ = view.update(cx, |view, cx| {
                if view.catalog_loading_key.as_deref() != Some(&key) {
                    return;
                }
                view.catalog_loading_key = None;
                match result {
                    Ok(catalog) => {
                        view.catalog_key = Some(key.clone());
                        if !had_draft {
                            view.selected_model = catalog
                                .get("current")
                                .and_then(|v| v.get("model"))
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_owned();
                            view.selected_effort = catalog
                                .get("current")
                                .and_then(|v| v.get("reasoningEffort"))
                                .and_then(Value::as_str)
                                .map(str::to_owned);
                            view.selected_fast_mode = catalog
                                .get("current")
                                .and_then(|v| v.get("fastMode"))
                                .and_then(Value::as_bool);
                            view.model_drafts.insert(
                                key.clone(),
                                (
                                    view.selected_model.clone(),
                                    view.selected_effort.clone(),
                                    view.selected_fast_mode,
                                ),
                            );
                        }
                        view.catalog = Some(catalog);
                    }
                    Err(error) => view.catalog_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

impl DraftCreationPane {
    fn browse_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose working folder".into()),
        });
        cx.spawn_in(window, async move |view, cx| {
            if let Ok(Ok(Some(paths))) = prompt.await {
                if let Some(path) = paths.first() {
                    let path = path.to_string_lossy().to_string();
                    let _ = view.update_in(cx, |pane, window, cx| {
                        if !pane.busy && pane.created_task_id.is_none() {
                            pane.cwd
                                .update(cx, |input, cx| input.set_value(path, window, cx));
                        }
                    });
                }
            }
        })
        .detach();
    }
    fn picker_trigger(
        &self,
        kind: &'static str,
        label: String,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let owner = cx.entity();
        let opener = owner.clone();
        Popover::new(SharedString::from(format!("draft-{kind}-picker")))
            .anchor(Anchor::TopLeft)
            .overlay_closable(true)
            .open(self.picker_open == Some(kind))
            .on_open_change(move |open, window, cx| {
                opener.update(cx, |pane, cx| {
                    pane.picker_open = if *open { Some(kind) } else { None };
                    if *open {
                        let input = if kind == "agent" {
                            &pane.agent_search
                        } else {
                            &pane.project_search
                        };
                        input.read(cx).focus_handle(cx).focus(window, cx);
                    }
                    cx.notify();
                })
            })
            .trigger(
                compact_choice_button(
                    SharedString::from(format!("draft-{kind}-trigger")),
                    format!("{label} ▾"),
                )
                .disabled(self.busy || self.created_task_id.is_some()),
            )
            .content(move |_, _, cx| owner.update(cx, |pane, cx| pane.render_picker(kind, cx)))
            .into_any_element()
    }
    fn render_picker(&mut self, kind: &str, cx: &mut Context<Self>) -> AnyElement {
        let search = if kind == "agent" {
            self.agent_search.clone()
        } else {
            self.project_search.clone()
        };
        let query = search.read(cx).value().to_lowercase();
        let mut menu = div()
            .id("draft-picker-options")
            .w(px(300.))
            .max_h(px(360.))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(4.))
            .p(px(8.))
            .child(Input::new(&search).small());
        if kind == "project" {
            menu = menu.child(
                compact_choice_button("draft-no-project", "No project").on_click(cx.listener(
                    |pane, _, window, cx| {
                        pane.save_model_draft();
                        pane.project_id.clear();
                        pane.picker_open = None;
                        pane.message.read(cx).focus_handle(cx).focus(window, cx);
                        let cwd = suggested_cwd(&pane.snapshot, &pane.agent_id, "");
                        pane.cwd
                            .update(cx, |input, cx| input.set_value(cwd, window, cx));
                        pane.emit_changed(cx);
                        cx.notify();
                    },
                )),
            );
        }
        for row in rows(
            &self.snapshot,
            if kind == "agent" {
                "agents"
            } else {
                "projects"
            },
        ) {
            let id = field(&row, "id");
            let label = field(&row, "name");
            if !label.to_lowercase().contains(&query) {
                continue;
            }
            let agent = kind == "agent";
            menu = menu.child(
                compact_choice_button(
                    SharedString::from(format!("draft-picker-{kind}-{id}")),
                    label,
                )
                .on_click(cx.listener(move |pane, _, window, cx| {
                    if agent {
                        pane.select_agent(id.clone(), window, cx);
                        pane.picker_open = Some("project");
                        pane.project_search
                            .read(cx)
                            .focus_handle(cx)
                            .focus(window, cx);
                    } else {
                        pane.save_model_draft();
                        pane.project_id = id.clone();
                        pane.picker_open = None;
                        pane.message.read(cx).focus_handle(cx).focus(window, cx);
                        let cwd = suggested_cwd(&pane.snapshot, &pane.agent_id, &pane.project_id);
                        pane.cwd
                            .update(cx, |input, cx| input.set_value(cwd, window, cx));
                        pane.emit_changed(cx);
                        cx.notify();
                    }
                })),
            );
        }
        menu.into_any_element()
    }
    fn menu_trigger(
        &self,
        kind: &'static str,
        label: String,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let owner = cx.entity();
        Popover::new(SharedString::from(format!("draft-{kind}-menu")))
            .anchor(Anchor::BottomLeft)
            .overlay_closable(true)
            .trigger(
                compact_choice_button(
                    SharedString::from(format!("draft-{kind}-trigger")),
                    format!("{label} ▾"),
                )
                .disabled(self.busy),
            )
            .on_open_change({
                let owner = owner.clone();
                move |open, _, cx| {
                    if *open && kind == "model" {
                        owner.update(cx, |pane, cx| pane.request_catalog_refresh(true, cx));
                    }
                }
            })
            .content(move |_, _, cx| {
                owner.update(cx, |pane, cx| {
                    if kind == "model" {
                        pane.render_model_menu(cx)
                    } else {
                        pane.render_access_menu(cx)
                    }
                })
            })
            .into_any_element()
    }
    fn render_model_menu(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let mut model_choices = div().flex().flex_wrap().gap(px(5.)).child(
            compact_choice_button(
                "draft-model-default",
                if self.selected_model.is_empty() {
                    "✓ Harness default"
                } else {
                    "Harness default"
                },
            )
            .disabled(self.busy || self.created_task_id.is_some())
            .on_click(cx.listener(|pane, _, _, cx| pane.select_model(String::new(), cx))),
        );
        if let Some(catalog) = &self.catalog {
            if let Some(models) = catalog["models"].as_array() {
                for model in models {
                    let id = field(model, "id");
                    let name = field(model, "name");
                    model_choices = model_choices.child(
                        compact_choice_button(
                            SharedString::from(format!("draft-model-{id}")),
                            if self.selected_model == id {
                                format!("✓ {name}")
                            } else {
                                name
                            },
                        )
                        .disabled(self.busy || self.created_task_id.is_some())
                        .on_click(
                            cx.listener(move |pane, _, _, cx| pane.select_model(id.clone(), cx)),
                        ),
                    );
                }
            }
        }
        let mut model_panel = div()
            .flex()
            .flex_col()
            .gap(px(5.))
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(0x8c9690))
                    .child("Model"),
            )
            .child(model_choices);
        if self.catalog_loading_key.as_deref() == Some(&self.catalog_identity()) {
            model_panel = model_panel.child(
                div()
                    .text_color(rgb(0x9aa39d))
                    .child("Loading available models…"),
            );
        }
        if let Some(error) = &self.catalog_error {
            model_panel = model_panel.child(
                div()
                    .text_color(rgb(0xe0a66a))
                    .child(format!("Model catalog unavailable: {error}")),
            );
        }
        if let Some(catalog) = &self.catalog {
            if let Some(model) = catalog["models"].as_array().and_then(|models| {
                models
                    .iter()
                    .find(|model| field(model, "id") == self.selected_model)
            }) {
                let efforts = model["reasoningEfforts"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                if !efforts.is_empty() {
                    let mut effort_choices = div().flex().flex_wrap().gap(px(5.)).child(
                        compact_choice_button(
                            "draft-effort-default",
                            if self.selected_effort.is_none() {
                                "✓ Default effort"
                            } else {
                                "Default effort"
                            },
                        )
                        .disabled(self.busy || self.created_task_id.is_some())
                        .on_click(cx.listener(|pane, _, _, cx| pane.select_effort(None, cx))),
                    );
                    for effort in efforts {
                        let id = field(&effort, "id");
                        effort_choices = effort_choices.child(
                            compact_choice_button(
                                SharedString::from(format!("draft-effort-{id}")),
                                if self.selected_effort.as_deref() == Some(&id) {
                                    format!("✓ {id}")
                                } else {
                                    id.clone()
                                },
                            )
                            .disabled(self.busy || self.created_task_id.is_some())
                            .on_click(cx.listener(
                                move |pane, _, _, cx| pane.select_effort(Some(id.clone()), cx),
                            )),
                        );
                    }
                    model_panel = model_panel.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(5.))
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(rgb(0x8c9690))
                                    .child("Reasoning effort"),
                            )
                            .child(effort_choices),
                    );
                }
                if model["supportsFast"].as_bool() == Some(true) {
                    let fast = self.selected_fast_mode.unwrap_or(false);
                    model_panel = model_panel.child(
                        compact_choice_button(
                            "draft-fast-mode",
                            if fast { "✓ Fast mode" } else { "Fast mode" },
                        )
                        .disabled(self.busy)
                        .on_click(
                            cx.listener(move |pane, _, _, cx| pane.select_fast_mode(!fast, cx)),
                        ),
                    );
                }
            }
        }
        div()
            .id("draft-model-options")
            .w(px(350.))
            .max_h(px(400.))
            .overflow_y_scroll()
            .p(px(10.))
            .child(model_panel)
            .into_any_element()
    }
    fn render_access_menu(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let provider = self
            .selected_agent()
            .as_ref()
            .map(|agent| field(agent, "provider"))
            .unwrap_or_default();
        let sandbox = self.sandbox.clone();
        let auto_value = if provider == "codex" {
            "workspace-write"
        } else {
            "harness-configured"
        };
        let (supports_read_only, supports_auto_edits, supports_full_access) = self
            .selected_agent()
            .as_ref()
            .map(permission_capabilities)
            .unwrap_or((false, false, false));
        let selected_access = if sandbox == "read-only" && supports_read_only {
            "read-only"
        } else if sandbox == "yolo" && supports_full_access {
            "yolo"
        } else {
            auto_value
        };
        let access = div()
            .flex()
            .flex_wrap()
            .gap(px(5.))
            .child(
                compact_choice_button(
                    "draft-access-read-only",
                    if selected_access == "read-only" {
                        "✓ Read only"
                    } else {
                        "Read only"
                    },
                )
                .disabled(self.busy || self.created_task_id.is_some() || !supports_read_only)
                .on_click(cx.listener(|pane, _, _, cx| {
                    if pane.sandbox == "read-only" {
                        return;
                    }
                    pane.sandbox = "read-only".into();
                    pane.emit_changed(cx);
                    cx.notify();
                })),
            )
            .child(
                compact_choice_button(
                    "draft-access-auto-edits",
                    if selected_access == auto_value {
                        "✓ Auto-approve edits"
                    } else {
                        "Auto-approve edits"
                    },
                )
                .disabled(self.busy || self.created_task_id.is_some() || !supports_auto_edits)
                .on_click(cx.listener(move |pane, _, _, cx| {
                    if pane.sandbox == auto_value {
                        return;
                    }
                    pane.sandbox = auto_value.into();
                    pane.emit_changed(cx);
                    cx.notify();
                })),
            )
            .child(
                compact_choice_button(
                    "draft-access-full",
                    if selected_access == "yolo" {
                        "✓ Full access"
                    } else {
                        "Full access"
                    },
                )
                .disabled(self.busy || self.created_task_id.is_some() || !supports_full_access)
                .on_click(cx.listener(|pane, _, _, cx| {
                    if pane.sandbox == "yolo" {
                        return;
                    }
                    pane.sandbox = "yolo".into();
                    pane.emit_changed(cx);
                    cx.notify();
                })),
            );
        div()
            .w(px(300.))
            .p(px(10.))
            .child(access)
            .into_any_element()
    }
}

impl Render for DraftCreationPane {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.request_catalog(cx);
        let agent_name = self
            .selected_agent()
            .map(|a| field(&a, "name"))
            .unwrap_or_else(|| "Choose an agent".into());
        let project_name = rows(&self.snapshot, "projects")
            .iter()
            .find(|p| field(p, "id") == self.project_id)
            .map(|p| field(p, "name"))
            .unwrap_or_else(|| "No project".into());
        let agent_choices = self.picker_trigger("agent", agent_name, cx);
        let project_choices = self.picker_trigger("project", project_name, cx);
        let root = div()
            .id("native-chat-draft")
            .flex()
            .key_context("MonitterChat")
            .size_full()
            .min_h_0()
            .overflow_y_scroll()
            .flex_col()
            .items_center()
            .px(px(32.))
            .py(px(36.))
            .font_family("Montserrat");
        let mut content = div()
            .w_full()
            .max_w(px(700.))
            .flex()
            .flex_shrink_0()
            .flex_col()
            .gap(px(12.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(0x3f9d6a))
                            .child("NEW CHAT"),
                    )
                    .child(
                        div()
                            .text_size(px(26.))
                            .font_weight(FontWeight::MEDIUM)
                            .child("What would you like to work on?"),
                    )
                    .child(
                        div()
                            .text_color(rgb(0x9aa39d))
                            .child("Ask a question, explore a project, or describe a change. Your agent starts when you send."),
                    ),
            )
            .child(div().flex().gap(px(12.)).child(div().flex_1().min_w_0().flex().flex_col().gap(px(5.)).child(div().text_size(px(11.)).text_color(rgb(0x8c9690)).child("Agent")).child(agent_choices)).child(div().flex_1().min_w_0().flex().flex_col().gap(px(5.)).child(div().text_size(px(11.)).text_color(rgb(0x8c9690)).child("Project")).child(project_choices)));
        if let Some(parent_id) = self.parent_task_id.clone() {
            let title = rows(&self.snapshot, "tasks")
                .iter()
                .find(|task| field(task, "id") == parent_id)
                .map(|task| field(task, "title"))
                .unwrap_or_else(|| "Unavailable parent chat".into());
            content = content.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(format!("Delegated from: {title}")),
                    )
                    .child(
                        compact_choice_button("draft-remove-parent", "Remove parent link")
                            .disabled(self.busy || self.created_task_id.is_some())
                            .on_click(cx.listener(|pane, _, _, cx| {
                                pane.parent_task_id = None;
                                pane.emit_changed(cx);
                                cx.notify();
                            })),
                    ),
            );
        }
        if self.project_id.is_empty() {
            content = content.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(5.))
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(0x8c9690))
                            .child("Working folder"),
                    )
                    .child(
                        Input::new(&self.cwd)
                            .small()
                            .text_size(px(12.5))
                            .px(px(9.))
                            .py(px(7.))
                            .disabled(self.busy || self.created_task_id.is_some()),
                    ),
            );
            if self.selected_agent().is_some_and(|agent| {
                rows(&self.snapshot, "hosts").iter().any(|host| {
                    field(host, "id") == field(&agent, "hostId") && field(host, "kind") == "local"
                })
            }) {
                content = content.child(
                    compact_choice_button("draft-browse-folder", "Browse working folder…")
                        .disabled(self.busy || self.created_task_id.is_some())
                        .on_click(
                            cx.listener(|pane, _, window, cx| pane.browse_folder(window, cx)),
                        ),
                );
            }
        }
        let access = self.menu_trigger("access", format!("Access · {}", self.sandbox), cx);
        let model = self.menu_trigger(
            "model",
            if self.selected_model.is_empty() {
                "Harness default".into()
            } else {
                self.selected_model.clone()
            },
            cx,
        );
        content = content.child(
            div()
                .flex()
                .flex_col()
                .border_1()
                .rounded_lg()
                .border_color(rgb(0x465049))
                .child(
                    Textarea::new(&self.message)
                        .small()
                        .text_size(px(13.))
                        .line_height(relative(1.55))
                        .px(px(9.))
                        .py(px(7.))
                        .h(px(96.))
                        .disabled(self.busy),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .p(px(7.))
                        .child(access)
                        .child(model)
                        .child(
                            compact_choice_button(
                                "draft-send",
                                if self.busy { "Sending…" } else { "Send" },
                            )
                            .with_variant(ButtonVariant::Primary)
                            .disabled(self.busy)
                            .on_click(cx.listener(|pane, _, _, cx| pane.send(cx))),
                        ),
                ),
        );
        let mut suggestions = div().flex().flex_wrap().gap(px(5.));
        for (id, label, message) in [
            (
                "review",
                "Review this project",
                "Review this project and suggest the next concrete step.",
            ),
            (
                "investigate",
                "Investigate an issue",
                "Investigate the current issue and report what you find.",
            ),
            (
                "plan",
                "Plan work",
                "Plan the implementation before making changes.",
            ),
        ] {
            suggestions = suggestions.child(
                compact_choice_button(SharedString::from(format!("draft-suggestion-{id}")), label)
                    .disabled(self.busy)
                    .on_click(cx.listener(move |pane, _, window, cx| {
                        pane.message
                            .update(cx, |input, cx| input.set_value(message, window, cx));
                        pane.message.read(cx).focus_handle(cx).focus(window, cx);
                    })),
            );
        }
        content = content.child(suggestions).child(
            compact_choice_button(
                "draft-options",
                if self.options_open {
                    "▾ Chat options"
                } else {
                    "▸ Chat options"
                },
            )
            .text()
            .on_click(cx.listener(|pane, _, _, cx| {
                pane.options_open = !pane.options_open;
                cx.notify();
            })),
        );
        if self.options_open {
            content = content.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(5.))
                    .child("Task title · optional")
                    .child(
                        Input::new(&self.title)
                            .small()
                            .disabled(self.busy || self.created_task_id.is_some()),
                    )
                    .child("Existing native session ID · optional")
                    .child(
                        Input::new(&self.native_session)
                            .small()
                            .disabled(self.busy || self.created_task_id.is_some()),
                    ),
            );
        }
        if let Some(error) = &self.error {
            content = content.child(div().text_color(rgb(0xe06b69)).child(error.clone()));
        }
        root.child(content)
    }
}

#[cfg(test)]
mod state_tests {
    use super::{DraftFormState, DraftModelSelection, validate_draft_state};
    use serde_json::{Value, json};
    use std::collections::HashMap;

    fn snapshot() -> Value {
        json!({
            "agents": [{"id":"agent-a"}],
            "projects": [{"id":"project-a"}],
            "tasks": [{"id":"parent-a"}]
        })
    }

    fn state() -> DraftFormState {
        DraftFormState {
            agent_id: "agent-a".into(),
            project_id: "project-a".into(),
            parent_task_id: Some("parent-a".into()),
            title: "Title".into(),
            cwd: "/tmp/work".into(),
            message: "unsent prompt".into(),
            sandbox: "workspace-write".into(),
            selected_model: "model-a".into(),
            selected_effort: Some("medium".into()),
            selected_fast_mode: Some(true),
            model_drafts: HashMap::from([(
                "agent-a:project-a".into(),
                DraftModelSelection {
                    model: "model-a".into(),
                    effort: Some("medium".into()),
                    fast_mode: Some(true),
                },
            )]),
            created_task_id: None,
        }
    }

    #[::core::prelude::v1::test]
    fn bounded_state_keeps_full_form_and_agent_project_model_cache() {
        let original = state();
        validate_draft_state(&original, &snapshot()).unwrap();
        let roundtrip = original.clone();
        assert_eq!(roundtrip, original);
        assert_eq!(roundtrip.message, "unsent prompt");
        assert_eq!(
            roundtrip.model_drafts["agent-a:project-a"].fast_mode,
            Some(true)
        );
    }

    #[::core::prelude::v1::test]
    fn restore_validation_rejects_unknown_agent_project_and_parent() {
        let mut invalid = state();
        invalid.agent_id = "deleted-agent".into();
        assert!(validate_draft_state(&invalid, &snapshot()).is_err());

        let mut invalid = state();
        invalid.project_id = "deleted-project".into();
        assert!(validate_draft_state(&invalid, &snapshot()).is_err());

        let mut invalid = state();
        invalid.parent_task_id = Some("deleted-parent".into());
        assert!(validate_draft_state(&invalid, &snapshot()).is_err());
    }

    #[::core::prelude::v1::test]
    fn restore_validation_refuses_oversize_or_unknown_model_cache() {
        let mut invalid = state();
        invalid.message = "x".repeat(32 * 1024 + 1);
        assert!(validate_draft_state(&invalid, &snapshot()).is_err());

        let mut invalid = state();
        invalid.model_drafts.insert(
            "missing-agent:project-a".into(),
            DraftModelSelection {
                model: "model-b".into(),
                effort: None,
                fast_mode: None,
            },
        );
        assert!(validate_draft_state(&invalid, &snapshot()).is_err());
    }

    #[::core::prelude::v1::test]
    fn restored_retry_id_must_match_authoritative_task_identity_and_folder() {
        let mut live_snapshot = snapshot();
        live_snapshot["agents"][0]["hostId"] = json!("host-a");
        live_snapshot["agents"][0]["cwd"] = json!("/agent-work");
        live_snapshot["projects"][0]["workspaces"] =
            json!([{"hostId":"host-a","cwd":"/project-work"}]);
        live_snapshot["tasks"].as_array_mut().unwrap().push(json!({
            "id":"created-a", "agentId":"agent-a", "projectId":"project-a", "cwd":"/project-work"
        }));
        let mut saved = state();
        saved.created_task_id = Some("created-a".into());
        validate_draft_state(&saved, &live_snapshot).unwrap();

        live_snapshot["tasks"][1]["agentId"] = json!("other-agent");
        assert!(validate_draft_state(&saved, &live_snapshot).is_err());
        live_snapshot["tasks"][1]["agentId"] = json!("agent-a");
        live_snapshot["tasks"][1]["cwd"] = json!("/wrong-folder");
        assert!(validate_draft_state(&saved, &live_snapshot).is_err());

        let stale = snapshot();
        saved.created_task_id = Some("missing-task".into());
        assert!(validate_draft_state(&saved, &stale).is_err());
    }
}
