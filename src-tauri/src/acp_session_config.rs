//! Generic, data-driven ACP session selectors. Nothing here guesses model names
//! or permission semantics from an agent brand or an option's display label.
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigValue {
    pub value: String,
    pub name: String,
    pub description: Option<String>,
    pub group: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigOption {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub category: Option<String>,
    pub current_value: String,
    pub options: Vec<ConfigValue>,
}

pub fn model_catalog(session: &Value) -> Result<crate::model::ModelCatalog, String> {
    use crate::model::{CatalogModel, ModelCatalog, ModelCatalogCurrent};
    let options = parse_options(&session["configOptions"])?;
    let model = options
        .iter()
        .find(|option| option.category.as_deref() == Some("model"));
    let (choices, current) = if let Some(model) = model {
        (
            model
                .options
                .iter()
                .map(|o| {
                    (
                        o.value.clone(),
                        o.name.clone(),
                        o.description.clone().unwrap_or_default(),
                    )
                })
                .collect::<Vec<_>>(),
            model.current_value.clone(),
        )
    } else {
        let available = session["models"]["availableModels"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        if available.len() > 4096 {
            return Err("Too many advertised ACP models.".into());
        }
        let choices = available
            .iter()
            .map(|m| {
                Ok((
                    text(&m["modelId"], 512)?,
                    text(&m["name"], 512)?,
                    optional_text(&m["description"], 4096).unwrap_or_default(),
                ))
            })
            .collect::<Result<Vec<_>, String>>()?;
        (
            choices,
            optional_text(&session["models"]["currentModelId"], 512).unwrap_or_default(),
        )
    };
    let warning = choices.is_empty().then(|| "Models become available after the ACP agent initializes a chat and advertises its choices. Its default model is used otherwise.".into());
    Ok(ModelCatalog {
        models: choices
            .into_iter()
            .map(|(id, name, description)| CatalogModel {
                id,
                name,
                description,
                reasoning_efforts: vec![],
                default_effort: None,
                supports_fast: false,
                fast_description: None,
            })
            .collect(),
        current: ModelCatalogCurrent {
            model: current,
            reasoning_effort: None,
            fast_mode: None,
        },
        source: "ACP session advertisement".into(),
        warning,
    })
}

fn text(value: &Value, limit: usize) -> Result<String, String> {
    value
        .as_str()
        .filter(|s| s.len() <= limit)
        .map(str::to_owned)
        .ok_or_else(|| "ACP session option contains a missing or oversized string.".into())
}
fn optional_text(value: &Value, limit: usize) -> Option<String> {
    text(value, limit).ok()
}

/// Parse only understood select controls, preserving order and group labels.
/// Unsupported control types retain the agent's defaults and are not offered.
pub fn parse_options(value: &Value) -> Result<Vec<ConfigOption>, String> {
    if value.is_null() {
        return Ok(vec![]);
    }
    let rows = value
        .as_array()
        .ok_or("ACP configOptions must be an array.")?;
    if rows.len() > 64 {
        return Err("Too many ACP session controls (maximum 64).".into());
    }
    let mut ids = HashSet::new();
    let mut result = Vec::new();
    let mut choice_count = 0;
    for row in rows {
        if row["type"] != "select" {
            continue;
        }
        let id = text(&row["id"], 512)?;
        if !ids.insert(id.clone()) {
            return Err("Duplicate ACP session control IDs.".into());
        }
        let choices = row["options"]
            .as_array()
            .ok_or("ACP select control is missing its options.")?;
        let mut options = Vec::new();
        let mut values = HashSet::new();
        for choice in choices {
            let group = choice.get("group");
            let group_name = group.map(|_| text(&choice["name"], 512)).transpose()?;
            let nested = if group.is_some() {
                choice["options"]
                    .as_array()
                    .ok_or("ACP option group is missing its options.")?
                    .as_slice()
            } else {
                std::slice::from_ref(choice)
            };
            for option in nested {
                choice_count += 1;
                if choice_count > 4096 {
                    return Err("Too many ACP session choices (maximum 4096).".into());
                }
                let value = text(&option["value"], 512)?;
                if !values.insert(value.clone()) {
                    return Err("Duplicate ACP session choice values.".into());
                }
                options.push(ConfigValue {
                    value,
                    name: text(&option["name"], 512)?,
                    description: optional_text(&option["description"], 4096),
                    group: group_name.clone(),
                });
            }
        }
        result.push(ConfigOption {
            id,
            name: text(&row["name"], 512)?,
            description: optional_text(&row["description"], 4096),
            category: optional_text(&row["category"], 512),
            current_value: text(&row["currentValue"], 512)?,
            options,
        });
    }
    Ok(result)
}

pub fn selection_params(
    options: &[ConfigOption],
    session_id: &str,
    config_id: &str,
    value: &str,
) -> Result<Value, String> {
    let option = options
        .iter()
        .find(|o| o.id == config_id)
        .ok_or("This session option is no longer available.")?;
    if !option.options.iter().any(|o| o.value == value) {
        return Err("This value is not advertised by the ACP agent for that option.".into());
    }
    Ok(json!({"sessionId":session_id,"configId":config_id,"value":value}))
}

/// An empty configured model means use the agent default, without a request.
/// Nonempty user selections must never be silently ignored or converted to flags.
pub fn configured_model_request(
    session_result: &Value,
    session_id: &str,
    model: &str,
) -> Result<Option<(&'static str, Value)>, String> {
    if model.is_empty() {
        return Ok(None);
    }
    let options = parse_options(&session_result["configOptions"])?;
    if let Some(option) = options
        .iter()
        .find(|o| o.category.as_deref() == Some("model"))
    {
        let params = selection_params(&options, session_id, &option.id, model)?;
        return Ok((option.current_value != model).then_some(("session/set_config_option", params)));
    }
    // Legacy ACP SDKs exposed models separately. Only use this older method
    // when the agent explicitly returned its availableModels, never by brand.
    if let Some(models) = session_result["models"]["availableModels"].as_array() {
        if models.iter().any(|m| m["modelId"].as_str() == Some(model)) {
            return Ok(
                (session_result["models"]["currentModelId"].as_str() != Some(model)).then_some((
                    "session/set_model",
                    json!({"sessionId":session_id,"modelId":model}),
                )),
            );
        }
        return Err("The saved model is not advertised by this ACP agent.".into());
    }
    Err("This ACP agent did not advertise model selection. Clear the configured model to use its default.".into())
}

/// ACP owns permission semantics. YOLO prefers the live session's exact
/// full-access mode and otherwise permits only advertised one-time approvals;
/// Monitter never guesses from an agent name or selects durable authority.
pub enum PermissionConfiguration {
    NoChange,
    Request(&'static str, Value),
    /// The requested bypass was unavailable. The caller may use only the
    /// agent's advertised one-time permission option for an explicit YOLO run.
    UnavailableYolo(String),
}

pub fn configured_permission_request(
    session_result: &Value,
    session_id: &str,
    sandbox: &str,
) -> Result<PermissionConfiguration, String> {
    if sandbox == "harness-configured" {
        return Ok(PermissionConfiguration::NoChange);
    }
    if sandbox != "yolo" {
        return Err("This ACP task has an unsupported permission policy.".into());
    }
    let options = parse_options(&session_result["configOptions"])?;
    let Some(mode) = options
        .iter()
        .find(|option| option.category.as_deref() == Some("mode"))
    else {
        return Ok(PermissionConfiguration::UnavailableYolo(
            "This ACP agent did not advertise a permission mode selector.".into(),
        ));
    };
    if !mode
        .options
        .iter()
        .any(|option| option.value == "bypassPermissions")
    {
        return Ok(PermissionConfiguration::UnavailableYolo(
            "This ACP agent did not advertise bypassPermissions.".into(),
        ));
    }
    let params = selection_params(&options, session_id, &mode.id, "bypassPermissions")?;
    Ok((mode.current_value != "bypassPermissions")
        .then_some(PermissionConfiguration::Request(
            "session/set_config_option",
            params,
        ))
        .unwrap_or(PermissionConfiguration::NoChange))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn options() -> Value {
        json!([
            {"id":"_custom_selector","name":"Unknown category","category":"_custom","type":"select","currentValue":"a","options":[{"value":"a","name":"A"}]},
            {"id":"opaque-model-id","name":"Choose model","category":"model","type":"select","currentValue":"default","options":[{"group":"provider","name":"Provider group","options":[{"value":"default","name":"Default"},{"value":"other/model","name":"Other model"}]}]},
            {"id":"future","type":"unrecognized","currentValue":{}},
            {"id":"boolean-unadvertised","type":"boolean","currentValue":true}
        ])
    }
    #[test]
    fn unknown_categories_and_grouped_choices_preserve_agent_order() {
        let parsed = parse_options(&options()).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].category.as_deref(), Some("_custom"));
        assert_eq!(
            parsed[1].options[1].group.as_deref(),
            Some("Provider group")
        );
        assert_eq!(
            selection_params(&parsed, "session", "opaque-model-id", "other/model").unwrap()
                ["configId"],
            "opaque-model-id"
        );
        assert!(selection_params(&parsed, "session", "opaque-model-id", "invented").is_err());
    }
    #[test]
    fn configured_models_use_advertised_ids_only() {
        let result = json!({"configOptions":options()});
        assert!(configured_model_request(&json!({}), "s", "")
            .unwrap()
            .is_none());
        assert!(configured_model_request(&json!({}), "s", "model").is_err());
        assert!(configured_model_request(&result, "s", "default")
            .unwrap()
            .is_none());
        let (method, params) = configured_model_request(&result, "s", "other/model")
            .unwrap()
            .unwrap();
        assert_eq!(method, "session/set_config_option");
        assert_eq!(params["configId"], "opaque-model-id");
        assert!(configured_model_request(&result, "s", "missing").is_err());
    }
    #[test]
    fn duplicate_ids_and_invalid_values_are_not_selectable() {
        let mut value = options();
        value[1]["id"] = value[0]["id"].clone();
        assert!(parse_options(&value).is_err());
        assert!(parse_options(&json!({})).is_err());
        let oversized = Value::Array((0..65).map(|_| json!({})).collect());
        assert!(parse_options(&oversized).is_err());
    }
    #[test]
    fn legacy_models_require_explicit_advertisement() {
        let result = json!({"models":{"currentModelId":"first","availableModels":[{"modelId":"first"},{"modelId":"second"}]}});
        assert_eq!(
            configured_model_request(&result, "s", "second")
                .unwrap()
                .unwrap()
                .0,
            "session/set_model"
        );
        assert!(configured_model_request(&result, "s", "other").is_err());
    }

    #[test]
    fn yolo_requires_an_advertised_bypass_permission_mode() {
        let result = json!({"configOptions":[{
            "id":"mode","name":"Mode","category":"mode","type":"select",
            "currentValue":"default","options":[
                {"value":"default","name":"Manual"},
                {"value":"bypassPermissions","name":"Bypass permissions"}
            ]
        }]});
        assert!(matches!(
            configured_permission_request(&result, "s", "harness-configured").unwrap(),
            PermissionConfiguration::NoChange
        ));
        let PermissionConfiguration::Request(method, params) =
            configured_permission_request(&result, "s", "yolo").unwrap()
        else {
            panic!("expected an advertised bypass request");
        };
        assert_eq!(method, "session/set_config_option");
        assert_eq!(params["configId"], "mode");
        assert_eq!(params["value"], "bypassPermissions");
        assert!(matches!(
            configured_permission_request(&json!({}), "s", "yolo").unwrap(),
            PermissionConfiguration::UnavailableYolo(_)
        ));
    }

    #[test]
    fn opencode_mode_choices_do_not_masquerade_as_bypass_permissions() {
        let result = json!({"configOptions":[{
            "id":"mode","name":"Session Mode","category":"mode","type":"select",
            "currentValue":"orchestrator","options":[
                {"value":"orchestrator","name":"orchestrator"},
                {"value":"build","name":"build"},
                {"value":"plan","name":"plan"}
            ]
        }]});
        assert!(matches!(
            configured_permission_request(&result, "session", "yolo").unwrap(),
            PermissionConfiguration::UnavailableYolo(_)
        ));
    }
}
