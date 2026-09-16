//! Boundary regressions for ACP state validation and advertised model controls.

use crate::{
    acp_session_config,
    model::{AcpLaunch, ModelCatalog},
    Service,
};
use serde_json::json;
use std::{fs, sync::Arc};

fn service() -> (Arc<Service>, std::path::PathBuf) {
    let root = std::env::temp_dir().join(format!("monitter-acp-boundary-{}", crate::id()));
    (Service::open(None, root.clone()).unwrap(), root)
}

#[test]
fn lan_save_agent_rejects_malformed_acp_launch() {
    let (service, root) = service();
    let mut agent = service.snapshot().unwrap().agents[0].clone();
    agent.provider = "acp".into();
    agent.sandbox = "harness-configured".into();
    agent.acp = Some(AcpLaunch {
        command: "bad\0command".into(),
        args: vec![],
    });
    let result = service.lan_invoke("save_agent", json!({"agent": agent}));
    assert!(
        result.is_err(),
        "malformed ACP launch must not enter durable state"
    );
    service.cleanup();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn lan_save_non_acp_agent_clears_stale_acp_launch() {
    let (service, root) = service();
    let mut agent = service.snapshot().unwrap().agents[0].clone();
    agent.provider = "codex".into();
    agent.sandbox = "read-only".into();
    agent.acp = Some(AcpLaunch {
        command: "stale-acp".into(),
        args: vec!["--acp".into()],
    });
    service
        .lan_invoke("save_agent", json!({"agent": agent}))
        .unwrap();
    let saved = service.snapshot().unwrap().agents[0].acp.clone();
    assert!(
        saved.is_none(),
        "non-ACP agents must not retain ACP launch data"
    );
    service.cleanup();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn acp_model_catalog_preserves_opaque_choices_without_native_controls() {
    let session = json!({"configOptions":[
        {"id":"opaque-model", "name":"Model", "category":"model", "type":"select",
         "currentValue":"default", "options":[
            {"value":"vendor/one", "name":"One"},
            {"value":"vendor/two", "name":"Two"}
         ]}
    ]});
    let catalog: ModelCatalog = acp_session_config::model_catalog(&session).unwrap();
    assert_eq!(catalog.current.model, "default");
    assert_eq!(
        catalog
            .models
            .iter()
            .map(|m| m.id.as_str())
            .collect::<Vec<_>>(),
        ["vendor/one", "vendor/two"]
    );
    assert!(catalog
        .models
        .iter()
        .all(|m| m.reasoning_efforts.is_empty() && !m.supports_fast));
}
