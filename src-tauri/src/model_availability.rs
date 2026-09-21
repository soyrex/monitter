//! Availability oracle for Codex providers.
//!
//! Combines the local Codex CLI's `model/list` (via the resident subprocess
//! bridge in `models::read_codex_catalog`) with OpenAI's authoritative
//! `GET /v1/models` HTTP endpoint when an API key is present in the resolved
//! Codex home. The oracle never rejects a model that either source lists.
//!
//! The HTTP path is best-effort. A failure (timeout, 401, 429, network)
//! silently falls back to the local catalog and appends a `warning` string
//! on the returned `ModelCatalog` so the picker can surface it.

use crate::{
    codex_auth,
    codex_accounts,
    model::{CatalogModel, Host, ModelCatalog, ModelCatalogCurrent},
    models,
};
use std::time::Duration;

/// Total wall-clock cap for the `/v1/models` HTTP call. Chosen to fit
/// comfortably inside the picker refresh path without making
/// `Service::model_catalog` perceptibly slow when it hits a slow network.
const OPENAI_HTTP_TIMEOUT: Duration = Duration::from_secs(4);

/// Read the union catalog: local Codex CLI ∪ `/v1/models` if a key exists.
///
/// Always returns a `ModelCatalog`. When the HTTP call fails or no API key
/// is present, the returned catalog is the local CLI result, possibly with
/// a `warning` describing what was skipped or added.
pub fn read_codex_catalog_with_oracle(
    host: &Host,
    cwd: &str,
    codex_home: Option<&str>,
) -> Result<ModelCatalog, String> {
    let local = models::read_codex_catalog(host, cwd, codex_home)?;
    let resolved_home = codex_home
        .map(str::to_owned)
        .or_else(|| codex_accounts::effective_home(None).ok());
    let Some(home) = resolved_home else {
        return Ok(local);
    };
    match codex_auth::read_api_key(&home) {
        Ok(Some(api_key)) => match read_openai_model_ids(&api_key) {
            Ok(remote_ids) => Ok(union_catalog(local, remote_ids)),
            Err(error) => Ok(catalog_with_warning(
                local,
                format!(
                    "OpenAI /v1/models lookup failed; using local Codex catalog only. {error}"
                ),
            )),
        },
        Ok(None) => {
            // ChatGPT-OAuth account: no API key, local CLI is the only source.
            Ok(local)
        }
        Err(error) => Ok(catalog_with_warning(
            local,
            format!("{error} Using local Codex catalog only."),
        )),
    }
}

fn read_openai_model_ids(api_key: &str) -> Result<Vec<String>, String> {
    let response = ureq::get("https://api.openai.com/v1/models")
        .timeout(OPENAI_HTTP_TIMEOUT)
        .set("Authorization", format!("Bearer {api_key}"))
        .set("User-Agent", "Monitter/0.1")
        .call()
        .map_err(|error| format!("GET https://api.openai.com/v1/models failed: {error}"))?;
    if response.status() != 200 {
        let status = response.status();
        let body = response.into_string().unwrap_or_default();
        return Err(format!(
            "GET https://api.openai.com/v1/models returned HTTP {status}: {}",
            body.chars().take(200).collect::<String>()
        ));
    }
    let body = response
        .into_string()
        .map_err(|error| format!("Could not read /v1/models response body: {error}"))?;
    let parsed: OpenAiModelsResponse = serde_json::from_str(&body)
        .map_err(|error| format!("/v1/models returned non-JSON: {error}"))?;
    Ok(parsed.data.into_iter().map(|entry| entry.id).collect())
}

#[derive(Debug, serde::Deserialize)]
struct OpenAiModelsResponse {
    data: Vec<OpenAiModelEntry>,
}

#[derive(Debug, serde::Deserialize)]
struct OpenAiModelEntry {
    id: String,
}

/// Merge remote model IDs into the local catalog. IDs already present keep
/// their local metadata (description, reasoning efforts, fast tier).
/// IDs only in `/v1/models` are appended with empty metadata so the picker
/// renders them honestly as "no metadata yet".
fn union_catalog(local: ModelCatalog, remote_ids: Vec<String>) -> ModelCatalog {
    let mut by_id: std::collections::BTreeMap<String, CatalogModel> = local
        .models
        .into_iter()
        .map(|m| (m.id.clone(), m))
        .collect();
    let mut added: Vec<String> = remote_ids
        .into_iter()
        .filter(|id| !by_id.contains_key(id))
        .collect();
    added.sort();
    added.dedup();
    for id in &added {
        by_id.insert(
            id.clone(),
            CatalogModel {
                id: id.clone(),
                name: id.clone(),
                description: String::new(),
                reasoning_efforts: vec![],
                default_effort: None,
                supports_fast: false,
                fast_description: None,
            },
        );
    }
    let mut warning = local.warning;
    if !added.is_empty() {
        let extra = if added.len() == 1 {
            format!("Added {} from OpenAI /v1/models.", added[0])
        } else {
            format!(
                "Added {} models from OpenAI /v1/models (e.g. {}).",
                added.len(),
                added[0]
            )
        };
        warning = Some(match warning {
            Some(existing) => format!("{existing} {extra}"),
            None => extra,
        });
    }
    ModelCatalog {
        models: by_id.into_values().collect(),
        current: local.current,
        source: format!("{} + openai-api", local.source),
        warning,
    }
}

fn catalog_with_warning(mut catalog: ModelCatalog, warning: String) -> ModelCatalog {
    catalog.warning = Some(match catalog.warning {
        Some(existing) => format!("{existing} {warning}"),
        None => warning,
    });
    catalog
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ReasoningEffortOption;

    fn empty_catalog() -> ModelCatalog {
        ModelCatalog {
            models: vec![],
            current: ModelCatalogCurrent {
                model: String::new(),
                reasoning_effort: None,
                fast_mode: None,
            },
            source: "codex app-server".into(),
            warning: None,
        }
    }

    fn model(id: &str, description: &str, supports_fast: bool) -> CatalogModel {
        CatalogModel {
            id: id.into(),
            name: id.into(),
            description: description.into(),
            reasoning_efforts: vec![ReasoningEffortOption {
                id: "high".into(),
                description: "deep reasoning".into(),
            }],
            default_effort: Some("high".into()),
            supports_fast,
            fast_description: supports_fast.then(|| "2x".into()),
        }
    }

    #[test]
    fn union_adds_models_only_in_remote() {
        let local = ModelCatalog {
            models: vec![model("gpt-a", "rich", true)],
            current: ModelCatalogCurrent {
                model: "gpt-a".into(),
                reasoning_effort: None,
                fast_mode: None,
            },
            source: "codex app-server".into(),
            warning: None,
        };
        let merged = union_catalog(local, vec!["gpt-a".into(), "gpt-b".into(), "gpt-c".into()]);
        assert_eq!(merged.models.len(), 3);
        let gpt_b = merged.models.iter().find(|m| m.id == "gpt-b").unwrap();
        assert_eq!(gpt_b.name, "gpt-b");
        assert_eq!(gpt_b.description, "");
        assert!(!gpt_b.supports_fast);
        let warning = merged.warning.unwrap();
        assert!(warning.contains("Added 2 models"), "{warning}");
        assert!(warning.contains("gpt-b"), "{warning}");
    }

    #[test]
    fn union_preserves_local_metadata_for_overlap() {
        let local = ModelCatalog {
            models: vec![model("gpt-a", "rich description", true)],
            current: ModelCatalogCurrent {
                model: "gpt-a".into(),
                reasoning_effort: None,
                fast_mode: None,
            },
            source: "codex app-server".into(),
            warning: None,
        };
        let merged = union_catalog(local, vec!["gpt-a".into()]);
        assert_eq!(merged.models.len(), 1);
        let m = &merged.models[0];
        assert_eq!(m.description, "rich description");
        assert_eq!(m.default_effort.as_deref(), Some("high"));
        assert!(m.supports_fast);
        assert_eq!(m.reasoning_efforts.len(), 1);
    }

    #[test]
    fn union_dedupes_remote_ids() {
        let merged = union_catalog(empty_catalog(), vec!["x".into(), "x".into(), "y".into()]);
        assert_eq!(merged.models.len(), 2);
    }

    #[test]
    fn union_source_label_includes_openai_when_added() {
        let merged = union_catalog(empty_catalog(), vec!["new-model".into()]);
        assert_eq!(merged.source, "codex app-server + openai-api");
    }

    #[test]
    fn union_keeps_source_label_when_no_additions() {
        let local = ModelCatalog {
            models: vec![model("gpt-a", "rich", true)],
            current: ModelCatalogCurrent {
                model: "gpt-a".into(),
                reasoning_effort: None,
                fast_mode: None,
            },
            source: "codex app-server".into(),
            warning: None,
        };
        let merged = union_catalog(local, vec!["gpt-a".into()]);
        assert_eq!(merged.source, "codex app-server + openai-api");
        assert!(merged.warning.is_none());
    }

    #[test]
    fn catalog_with_warning_appends_existing() {
        let mut cat = empty_catalog();
        cat.warning = Some("first".into());
        cat = catalog_with_warning(cat, "second".into());
        assert_eq!(cat.warning.as_deref(), Some("first second"));
    }

    #[test]
    fn catalog_with_warning_creates_when_empty() {
        let cat = catalog_with_warning(empty_catalog(), "only".into());
        assert_eq!(cat.warning.as_deref(), Some("only"));
    }
}
