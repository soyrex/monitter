//! Model discovery for collaboration, with all probes outside workspace locks.
use crate::{
    model::{Agent, Host, ModelCatalog, Snapshot},
    model_catalog_key, Service, MODEL_CATALOG_CACHE_TTL,
};
use serde_json::{json, Value};

#[derive(Clone)]
pub(crate) struct DelegationTarget {
    pub agent: Agent,
    pub host: Host,
    pub cwd: String,
    pub project_id: Option<String>,
}
impl DelegationTarget {
    pub(crate) fn from_snapshot(
        snapshot: &Snapshot,
        caller_task: &str,
        agent_id: &str,
    ) -> Result<Self, String> {
        let caller = snapshot
            .tasks
            .iter()
            .find(|t| t.id == caller_task)
            .ok_or("Caller task was not found.")?;
        let agent = snapshot
            .agents
            .iter()
            .find(|a| a.id == agent_id && !a.internal && a.collaboration_enabled)
            .cloned()
            .ok_or("Recipient agent is unavailable for collaboration.")?;
        let host = snapshot
            .hosts
            .iter()
            .find(|h| h.id == agent.host_id)
            .cloned()
            .ok_or("Agent host was not found.")?;
        let project = caller
            .project_id
            .as_deref()
            .map(|id| {
                snapshot
                    .projects
                    .iter()
                    .find(|p| p.id == id)
                    .ok_or("Project was not found.")
            })
            .transpose()?;
        let cwd = project
            .and_then(|p| p.workspaces.iter().find(|w| w.host_id == agent.host_id))
            .map(|w| w.cwd.clone())
            .unwrap_or_else(|| agent.cwd.clone());
        let cwd = if cwd.trim().is_empty() {
            host.default_cwd.clone()
        } else {
            cwd
        };
        if cwd.trim().is_empty() {
            return Err("Agent or host must specify a task folder.".into());
        }
        Ok(Self {
            agent,
            host,
            cwd,
            project_id: caller.project_id.clone(),
        })
    }
    pub(crate) fn unchanged(&self, snapshot: &Snapshot, caller: &str) -> bool {
        Self::from_snapshot(snapshot, caller, &self.agent.id).is_ok_and(|new| {
            new.agent == self.agent
                && new.host == self.host
                && new.cwd == self.cwd
                && new.project_id == self.project_id
        })
    }
    fn key(&self) -> String {
        model_catalog_key(
            &self.host,
            &self.agent.provider,
            &self.cwd,
            self.agent.codex_home.as_deref(),
            self.agent.acp.as_ref(),
        )
    }
}
impl Service {
    pub(crate) fn delegation_target(
        &self,
        caller: &str,
        agent: &str,
    ) -> Result<DelegationTarget, String> {
        let data = self
            .data
            .lock()
            .map_err(|_| "Monitter state lock failed.")?;
        DelegationTarget::from_snapshot(&data.snapshot, caller, agent)
    }
    pub(crate) fn delegation_cached_catalog(
        &self,
        target: &DelegationTarget,
    ) -> Option<ModelCatalog> {
        self.model_catalogs
            .lock()
            .ok()?
            .get(&target.key())
            .filter(|(when, _)| when.elapsed() < MODEL_CATALOG_CACHE_TTL)
            .map(|(_, catalog)| catalog.clone())
    }
    pub(crate) fn delegation_catalog(
        &self,
        target: &DelegationTarget,
        refresh: bool,
    ) -> Result<ModelCatalog, String> {
        if target.agent.provider == "acp" {
            let launch = target
                .agent
                .acp
                .as_ref()
                .filter(|l| crate::model::valid_acp_launch(l))
                .ok_or("ACP agent has no valid saved launch configuration.")?;
            self.acp_agent_model_catalog(&target.host, launch, &target.cwd, refresh)
        } else {
            self.model_catalog(
                &target.host,
                &target.agent.provider,
                &target.cwd,
                target.agent.codex_home.as_deref(),
                refresh,
            )
        }
    }
    pub(crate) fn list_agent_models_protocol(
        &self,
        caller: &str,
        agent: &str,
        refresh: bool,
    ) -> Result<Value, String> {
        let target = self.delegation_target(caller, agent)?;
        let catalog = self.delegation_catalog(&target, refresh)?;
        self.require_collaboration_caller(caller)?;
        let data = self
            .data
            .lock()
            .map_err(|_| "Monitter state lock failed.")?;
        if !target.unchanged(&data.snapshot, caller) {
            return Err(
                "Recipient configuration changed during model discovery; retry the request.".into(),
            );
        }
        Ok(menu_value(&target.agent, Some(&catalog)))
    }
}
pub(crate) fn menu_value(agent: &Agent, catalog: Option<&ModelCatalog>) -> Value {
    menu_value_with_limit(agent, catalog, usize::MAX)
}
pub(crate) fn summary_value(agent: &Agent, catalog: Option<&ModelCatalog>) -> Value {
    menu_value_with_limit(agent, catalog, 16)
}
fn menu_value_with_limit(agent: &Agent, catalog: Option<&ModelCatalog>, limit: usize) -> Value {
    json!({"to_agent_id":agent.id,"defaultModel":if agent.model.is_empty() { catalog.map(|c| c.current.model.as_str()).unwrap_or("") } else { agent.model.as_str() },"models":catalog.map(|c|c.models.iter().take(limit).map(|m|crate::delegation_models::advertised_model(m,agent)).collect::<Vec<_>>()).unwrap_or_default(),"modelCatalog":{"source":catalog.map(|c|c.source.as_str()).unwrap_or("not-discovered"),"warning":catalog.and_then(|c|c.warning.clone()).or_else(||catalog.is_none().then(||"Use list_agent_models to discover this agent's model menu.".into())),"complete":catalog.is_some_and(|c| !c.models.is_empty() && c.models.len() <= limit)}})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        default_snapshot, task_from_agent, CatalogModel, CreateTaskInput, ModelCatalogCurrent,
    };

    #[test]
    fn delegation_target_rejects_configuration_drift_before_commit() {
        let mut snapshot = default_snapshot();
        let caller_agent = snapshot.agents[0].clone();
        let mut recipient = caller_agent.clone();
        recipient.id = "recipient".into();
        snapshot.agents.push(recipient.clone());
        let caller = task_from_agent(
            &caller_agent,
            &CreateTaskInput {
                agent_id: caller_agent.id.clone(),
                title: "Caller".into(),
                native_session_id: None,
                parent_task_id: None,
                channel_id: None,
                project_id: None,
                cwd: None,
                model_settings: None,
                sandbox: None,
            },
        );
        snapshot.tasks.push(caller.clone());
        let context =
            DelegationTarget::from_snapshot(&snapshot, &caller.id, &recipient.id).unwrap();
        assert!(context.unchanged(&snapshot, &caller.id));
        let before = snapshot.clone();
        snapshot.agents[1].model = "changed-model".into();
        assert!(!context.unchanged(&snapshot, &caller.id));
        snapshot = before.clone();
        snapshot.hosts[0].codex_path = "/different/cli".into();
        assert!(!context.unchanged(&snapshot, &caller.id));
        snapshot = before.clone();
        snapshot.agents[1].collaboration_enabled = false;
        assert!(!context.unchanged(&snapshot, &caller.id));
        snapshot = before;
        snapshot.tasks[0].project_id = Some("missing-project".into());
        assert!(!context.unchanged(&snapshot, &caller.id));
    }

    #[test]
    fn delegation_directory_summary_is_bounded_and_full_menu_preserves_default() {
        let mut agent = default_snapshot().agents.remove(0);
        agent.model.clear();
        let catalog = ModelCatalog {
            models: (0..17)
                .map(|i| CatalogModel {
                    id: format!("model-{i}"),
                    name: format!("Model {i}"),
                    description: String::new(),
                    reasoning_efforts: vec![],
                    default_effort: None,
                    supports_fast: false,
                    fast_description: None,
                })
                .collect(),
            current: ModelCatalogCurrent {
                model: "harness-default".into(),
                reasoning_effort: None,
                fast_mode: None,
            },
            source: "fixture".into(),
            warning: None,
        };
        let summary = summary_value(&agent, Some(&catalog));
        assert_eq!(summary["models"].as_array().unwrap().len(), 16);
        assert_eq!(summary["modelCatalog"]["complete"], false);
        assert_eq!(summary["defaultModel"], "harness-default");
        let full = menu_value(&agent, Some(&catalog));
        assert_eq!(full["models"].as_array().unwrap().len(), 17);
        assert_eq!(full["modelCatalog"]["complete"], true);
        agent.model = "saved-default".into();
        assert_eq!(
            menu_value(&agent, Some(&catalog))["defaultModel"],
            "saved-default"
        );
        assert_eq!(menu_value(&agent, None)["modelCatalog"]["complete"], false);
    }
}
