use crate::{
    model::{CatalogModel, Host, ModelCatalog, ModelCatalogCurrent, ReasoningEffortOption},
    probe_output,
    runner::{add_ssh_options, remote_path, resolve_local, resolve_local_provider, ssh_target},
};
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

const TOTAL_TIMEOUT: Duration = Duration::from_secs(12);
const MAX_PAGES: usize = 32;

pub(crate) fn read_codex_catalog(host: &Host, cwd: &str) -> Result<ModelCatalog, String> {
    let mut child = app_server_command(host, cwd)?
        .spawn()
        .map_err(|e| format!("Could not start Codex app-server for read-only model lookup: {e}"))?;
    let result = read_from_child(&mut child);
    stop_child(&mut child);
    result
}

/// Read provider-qualified models from the installed OpenCode CLI. This keeps
/// the picker aligned with the models and providers available on this host.
pub(crate) fn read_opencode_catalog(host: &Host, cwd: &str) -> Result<ModelCatalog, String> {
    let mut command = if host.kind == "local" {
        let mut command = Command::new(resolve_local_provider("opencode", &host.opencode_path)?);
        command.current_dir(cwd);
        command
    } else if host.kind == "ssh" {
        let cli = if host.opencode_path.trim().is_empty() {
            "opencode"
        } else {
            host.opencode_path.trim()
        };
        let mut command = Command::new("ssh");
        add_ssh_options(&mut command, host);
        command.arg(ssh_target(host)?).arg(format!(
            "cd {} && exec {} models",
            remote_path(cwd),
            remote_path(cli)
        ));
        command
    } else {
        return Err("Host kind must be local or ssh.".into());
    };
    if host.kind == "local" {
        command.arg("models");
    }
    let models = parse_opencode_models(&probe_output(command)?);
    if models.is_empty() {
        return Err("OpenCode returned no provider-qualified models. Check its configured providers and sign-in.".into());
    }
    Ok(ModelCatalog {
        models,
        current: ModelCatalogCurrent { model: String::new(), reasoning_effort: None, fast_mode: None },
        source: "opencode models".into(),
        warning: Some("OpenCode controls model capabilities; reasoning effort and Fast mode are unavailable for this harness.".into()),
    })
}

fn parse_opencode_models(output: &str) -> Vec<CatalogModel> {
    let mut ids = output
        .lines()
        .map(str::trim)
        .filter(|line| {
            line.split_once('/').is_some_and(|(provider, model)| {
                !provider.is_empty() && !model.is_empty() && !line.contains(char::is_whitespace)
            })
        })
        .map(str::to_owned)
        .collect::<Vec<_>>();
    ids.sort();
    ids.dedup();
    ids.into_iter()
        .map(|id| CatalogModel {
            name: id.clone(),
            description: id
                .split_once('/')
                .map(|(provider, _)| format!("{provider} via OpenCode"))
                .unwrap_or_default(),
            id,
            reasoning_efforts: vec![],
            default_effort: None,
            supports_fast: false,
            fast_description: None,
        })
        .collect()
}

fn app_server_command(host: &Host, cwd: &str) -> Result<Command, String> {
    let mut command = if host.kind == "local" {
        let mut command = Command::new(resolve_local(&host.codex_path)?);
        command.arg("app-server");
        command.current_dir(cwd);
        command
    } else if host.kind == "ssh" {
        let cli = if host.codex_path.trim().is_empty() {
            "codex"
        } else {
            host.codex_path.trim()
        };
        let mut command = Command::new("ssh");
        add_ssh_options(&mut command, host);
        command.arg(ssh_target(host)?).arg(format!(
            "cd {} && exec {} app-server",
            remote_path(cwd),
            remote_path(cli)
        ));
        command
    } else {
        return Err("Host kind must be local or ssh.".into());
    };
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    Ok(command)
}

fn read_from_child(child: &mut Child) -> Result<ModelCatalog, String> {
    let deadline = Instant::now() + TOTAL_TIMEOUT;
    let mut stdin = child
        .stdin
        .take()
        .ok_or("Could not open Codex app-server stdin.")?;
    let stdout = child
        .stdout
        .take()
        .ok_or("Could not open Codex app-server stdout.")?;
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let _ = tx.send(
                line.map_err(|e| e.to_string())
                    .and_then(|l| serde_json::from_str::<Value>(&l).map_err(|e| e.to_string())),
            );
        }
    });
    send(
        &mut stdin,
        json!({"id":1,"method":"initialize","params":{"clientInfo":{"name":"Monitter","version":"0.1"}}}),
    )?;
    response(&rx, 1, deadline)?;
    send(&mut stdin, json!({"method":"initialized"}))?;
    send(
        &mut stdin,
        json!({"id":2,"method":"config/read","params":{}}),
    )?;
    let config = response(&rx, 2, deadline)?;
    let mut models = Vec::new();
    let mut default_model: Option<String> = None;
    let mut cursor: Option<String> = None;
    let mut request_id = 3;
    for _ in 0..MAX_PAGES {
        let mut params = json!({});
        if let Some(value) = cursor {
            params["cursor"] = Value::String(value);
        }
        send(
            &mut stdin,
            json!({"id":request_id,"method":"model/list","params":params}),
        )?;
        let page = response(&rx, request_id, deadline)?;
        request_id += 1;
        let result = page
            .get("result")
            .ok_or("Codex app-server model/list returned an unsupported response.")?;
        let data = result
            .get("data")
            .and_then(Value::as_array)
            .ok_or("Codex app-server model/list omitted result.data.")?;
        if default_model.is_none() {
            default_model = data
                .iter()
                .find(|item| item.get("isDefault").and_then(Value::as_bool) == Some(true))
                .and_then(|item| {
                    item.get("model")
                        .or_else(|| item.get("id"))
                        .and_then(Value::as_str)
                })
                .map(str::to_owned);
        }
        models.extend(
            data.iter()
                .filter(|item| !item.get("hidden").and_then(Value::as_bool).unwrap_or(false))
                .map(parse_model)
                .collect::<Result<Vec<_>, _>>()?,
        );
        cursor = result
            .get("nextCursor")
            .and_then(Value::as_str)
            .map(str::to_owned);
        if cursor.is_none() {
            break;
        }
        if request_id == (MAX_PAGES as i64) + 3 {
            return Err("Codex app-server model/list exceeded the 32-page safety limit.".into());
        }
    }
    let current = config
        .pointer("/result/config")
        .ok_or("Codex app-server config/read omitted result.config.")?;
    Ok(ModelCatalog {
        models,
        current: ModelCatalogCurrent {
            model: current
                .get("model")
                .and_then(Value::as_str)
                .filter(|model| !model.trim().is_empty())
                .map(str::to_owned)
                .or(default_model)
                .unwrap_or_default(),
            reasoning_effort: current
                .get("model_reasoning_effort")
                .and_then(Value::as_str)
                .map(str::to_owned),
            fast_mode: current
                .get("service_tier")
                .and_then(Value::as_str)
                .map(|tier| tier == "priority"),
        },
        source: "codex app-server".into(),
        warning: None,
    })
}

fn parse_model(value: &Value) -> Result<CatalogModel, String> {
    let id = value
        .get("model")
        .or_else(|| value.get("id"))
        .and_then(Value::as_str)
        .ok_or("Codex app-server model omitted id.")?
        .to_owned();
    let tiers = value
        .get("serviceTiers")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let fast = tiers
        .iter()
        .find(|tier| tier.get("id").and_then(Value::as_str) == Some("priority"));
    Ok(CatalogModel {
        id: id.clone(),
        name: value
            .get("displayName")
            .and_then(Value::as_str)
            .unwrap_or(&id)
            .into(),
        description: value
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .into(),
        reasoning_efforts: value
            .get("supportedReasoningEfforts")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|item| {
                Some(ReasoningEffortOption {
                    id: item.get("reasoningEffort")?.as_str()?.into(),
                    description: item
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .into(),
                })
            })
            .collect(),
        default_effort: value
            .get("defaultReasoningEffort")
            .and_then(Value::as_str)
            .map(str::to_owned),
        supports_fast: fast.is_some(),
        fast_description: fast
            .and_then(|tier| tier.get("description"))
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}
fn send(stdin: &mut impl Write, value: Value) -> Result<(), String> {
    serde_json::to_writer(&mut *stdin, &value).map_err(|e| e.to_string())?;
    stdin
        .write_all(b"\n")
        .and_then(|_| stdin.flush())
        .map_err(|e| e.to_string())
}
fn response(
    rx: &mpsc::Receiver<Result<Value, String>>,
    id: i64,
    deadline: Instant,
) -> Result<Value, String> {
    loop {
        let left = deadline
            .checked_duration_since(Instant::now())
            .ok_or("Timed out waiting for Codex app-server model lookup.")?;
        let value = rx
            .recv_timeout(left)
            .map_err(|_| "Codex app-server exited or timed out during model lookup.")??;
        if value.get("id").and_then(Value::as_i64) == Some(id) {
            if let Some(error) = value
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(Value::as_str)
            {
                return Err(format!("Codex app-server request failed: {error}"));
            }
            return Ok(value);
        }
    }
}
fn stop_child(child: &mut Child) {
    #[cfg(unix)]
    unsafe {
        let _ = libc::kill(-(child.id() as i32), libc::SIGTERM);
    }
    let until = Instant::now() + Duration::from_millis(300);
    while Instant::now() < until {
        if child.try_wait().ok().flatten().is_some() {
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
    #[cfg(unix)]
    unsafe {
        let _ = libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_live_catalog_fast_as_priority_not_deprecated_fast() {
        let model = parse_model(&json!({"model":"gpt-test","displayName":"Test","description":"desc","supportedReasoningEfforts":[{"reasoningEffort":"high","description":"deep"}],"defaultReasoningEffort":"high","additionalSpeedTiers":["fast"],"serviceTiers":[{"id":"priority","name":"Fast","description":"2x"}]})).unwrap();
        assert!(model.supports_fast);
        assert_eq!(model.fast_description.as_deref(), Some("2x"));
        assert_eq!(model.reasoning_efforts[0].id, "high");
    }

    #[test]
    fn no_service_tier_means_fast_is_not_supported() {
        let model = parse_model(&json!({"model":"older","displayName":"Older","description":"","supportedReasoningEfforts":[],"defaultReasoningEffort":"medium","serviceTiers":[]})).unwrap();
        assert!(!model.supports_fast);
        assert_eq!(model.fast_description, None);
    }

    #[test]
    fn parses_provider_qualified_opencode_models() {
        let models = parse_opencode_models(
            "opencode-go/mimo-v2.5\nopenai/gpt-5.6-luna\nnot a model\nopencode-go/mimo-v2.5\n",
        );
        assert_eq!(models.len(), 2);
        assert!(models
            .iter()
            .any(|model| model.id == "opencode-go/mimo-v2.5"));
        assert!(models.iter().any(|model| model.id == "openai/gpt-5.6-luna"));
        assert!(models
            .iter()
            .all(|model| model.reasoning_efforts.is_empty()));
    }

    #[test]
    #[ignore = "read-only live Codex app-server verification"]
    fn live_local_catalog() {
        let host = Host {
            id: "local".into(),
            name: "local".into(),
            kind: "local".into(),
            address: "localhost".into(),
            user: String::new(),
            port: 0,
            identity_file: String::new(),
            default_cwd: "/tmp".into(),
            codex_path: String::new(),
            claude_path: String::new(),
            opencode_path: String::new(),
            hermes_path: String::new(),
        };
        let catalog = read_codex_catalog(&host, "/tmp").unwrap();
        println!(
            "local models={} fast={}",
            catalog.models.len(),
            catalog
                .models
                .iter()
                .filter(|model| model.supports_fast)
                .count()
        );
        assert!(!catalog.models.is_empty());
    }

    #[test]
    #[ignore = "read-only live SSH Codex app-server verification"]
    fn live_mira_catalog() {
        let host = Host {
            id: "mira".into(),
            name: "mira".into(),
            kind: "ssh".into(),
            address: "mira".into(),
            user: String::new(),
            port: 0,
            identity_file: String::new(),
            default_cwd: "/tmp".into(),
            codex_path: "/home/alex/.npm-global/bin/codex".into(),
            claude_path: String::new(),
            opencode_path: String::new(),
            hermes_path: String::new(),
        };
        let catalog = read_codex_catalog(&host, "/tmp").unwrap();
        println!(
            "mira models={} fast={}",
            catalog.models.len(),
            catalog
                .models
                .iter()
                .filter(|model| model.supports_fast)
                .count()
        );
        assert!(!catalog.models.is_empty());
    }
}
