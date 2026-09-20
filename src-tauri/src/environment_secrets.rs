//! Desktop-only environment vault.
//!
//! The encrypted payload lives in one macOS Keychain item.  This module only
//! returns redacted metadata to the renderer and only releases values directly
//! into a local child process environment at launch time.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    process::Command,
    sync::{Mutex, OnceLock},
};

const MAX_ENTRIES: usize = 64;
const MAX_NAME_BYTES: usize = 128;
const MAX_DESCRIPTION_BYTES: usize = 1_024;
const MAX_VALUE_BYTES: usize = 16 * 1024;
const MAX_PAYLOAD_BYTES: usize = 512 * 1024;
const KEYCHAIN_SERVICE: &str = "com.monitter.desktop.environment-secrets";
const KEYCHAIN_ACCOUNT: &str = "default";
static JEV_API_KEY_CACHE: OnceLock<Mutex<Option<String>>> = OnceLock::new();
#[cfg(not(target_os = "macos"))]
const UNSUPPORTED: &str = "Environment & Secrets is supported only on macOS.";

pub(crate) fn should_inject_into_local_user_command(host_kind: &str, internal: bool) -> bool {
    host_kind == "local" && !internal
}

/// This is intentionally only the response shape.  Values never cross the
/// Tauri boundary, are never serialized in a Snapshot, and never enter logs.
#[derive(Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EnvironmentSecretsConfig {
    pub(crate) revision: String,
    pub(crate) entries: Vec<EnvironmentSecretMetadata>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EnvironmentSecretMetadata {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) updated_at: i64,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Vault {
    #[serde(default)]
    entries: Vec<VaultEntry>,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct VaultEntry {
    name: String,
    value: String,
    description: String,
    updated_at: i64,
}

/// The write lock covers read-check-write, giving the opaque revision normal
/// optimistic-concurrency semantics even when two Settings panes save at once.
pub(crate) struct EnvironmentSecretsStore {
    writes: Mutex<()>,
}

impl EnvironmentSecretsStore {
    pub(crate) fn new() -> Self {
        Self {
            writes: Mutex::new(()),
        }
    }

    pub(crate) fn list(&self) -> Result<EnvironmentSecretsConfig, String> {
        let vault = load_vault()?;
        refresh_jev_api_key_cache(&vault);
        Ok(redacted_config(&vault))
    }

    pub(crate) fn set(
        &self,
        revision: String,
        name: String,
        value: String,
        description: String,
    ) -> Result<EnvironmentSecretsConfig, String> {
        let _guard = self
            .writes
            .lock()
            .map_err(|_| "Environment & Secrets is temporarily unavailable.".to_string())?;
        let name = normalize_name(&name)?;
        let value = normalize_value(value)?;
        let description = normalize_description(description)?;
        let mut vault = load_vault()?;
        if revision != revision_for(&vault) {
            return Err("Environment & Secrets changed in another Settings pane. Refresh before saving; your draft was not overwritten.".into());
        }
        let updated_at = crate::model::now();
        if let Some(entry) = vault.entries.iter_mut().find(|entry| entry.name == name) {
            entry.value = value;
            entry.description = description;
            entry.updated_at = updated_at;
        } else {
            if vault.entries.len() >= MAX_ENTRIES {
                return Err("Environment & Secrets has reached its entry limit.".into());
            }
            vault.entries.push(VaultEntry {
                name,
                value,
                description,
                updated_at,
            });
        }
        normalize_vault(&mut vault)
            .map_err(|_| "Environment & Secrets data is invalid.".to_string())?;
        save_vault(&vault)?;
        refresh_jev_api_key_cache(&vault);
        Ok(redacted_config(&vault))
    }

    pub(crate) fn delete(
        &self,
        revision: String,
        name: String,
    ) -> Result<EnvironmentSecretsConfig, String> {
        let _guard = self
            .writes
            .lock()
            .map_err(|_| "Environment & Secrets is temporarily unavailable.".to_string())?;
        let name = normalize_name(&name)?;
        let mut vault = load_vault()?;
        if revision != revision_for(&vault) {
            return Err("Environment & Secrets changed in another Settings pane. Refresh before deleting; your draft was not overwritten.".into());
        }
        let before = vault.entries.len();
        vault.entries.retain(|entry| entry.name != name);
        if vault.entries.len() == before {
            return Err("Environment variable was not found. Refresh before deleting.".into());
        }
        save_vault(&vault)?;
        refresh_jev_api_key_cache(&vault);
        Ok(redacted_config(&vault))
    }

    /// Read at the point a local user harness is launched.  The caller applies
    /// these pairs only through `Command::env`; this module never builds argv,
    /// shell text, or diagnostics from a secret value.
    pub(crate) fn apply_to_command(&self, command: &mut Command) -> Result<(), String> {
        let vault = load_vault()?;
        // A harness launch already paid the Keychain cost. Reuse only the
        // allowlisted Jev key for native classifier calls rather than opening
        // the vault a second time moments later.
        refresh_jev_api_key_cache(&vault);
        apply_pairs_to_command(command, environment_pairs(&vault));
        Ok(())
    }
}

/// Internal-only access for a native service that must make an authenticated
/// outbound request itself. This is deliberately allowlisted and returns no
/// value across the Tauri boundary or into diagnostic output.
pub(crate) fn jev_api_key_for_internal_service() -> Result<String, String> {
    let mut cached = JEV_API_KEY_CACHE
        .get_or_init(|| Mutex::new(None))
        .lock()
        .map_err(|_| "Jev credential cache is temporarily unavailable.".to_string())?;
    if let Some(value) = cached.as_ref() {
        return Ok(value.clone());
    }
    let value = load_vault()?
        .entries
        .into_iter()
        .find(|entry| entry.name == "JEV_API_KEY")
        .map(|entry| entry.value)
        .ok_or_else(|| "Monitter Environment & Secrets has no JEV_API_KEY entry.".to_string())?;
    *cached = Some(value.clone());
    Ok(value)
}

fn refresh_jev_api_key_cache(vault: &Vault) {
    let Ok(mut cached) = JEV_API_KEY_CACHE
        .get_or_init(|| Mutex::new(None))
        .lock()
    else {
        return;
    };
    *cached = vault
        .entries
        .iter()
        .find(|entry| entry.name == "JEV_API_KEY")
        .map(|entry| entry.value.clone());
}

fn apply_pairs_to_command(command: &mut Command, pairs: Vec<(String, String)>) {
    for (name, value) in pairs {
        command.env(name, value);
    }
}

fn normalize_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    let valid = !name.is_empty()
        && name.len() <= MAX_NAME_BYTES
        && name.bytes().enumerate().all(|(index, byte)| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'_' => true,
            b'0'..=b'9' => index != 0,
            _ => false,
        });
    if !valid || reserved_name(name) {
        return Err("Environment variable name is invalid or reserved.".into());
    }
    Ok(name.into())
}

fn reserved_name(name: &str) -> bool {
    matches!(
        name,
        "PATH" | "HOME" | "SHELL" | "TMPDIR" | "CODEX_HOME" | "OPENCODE_CONFIG_CONTENT"
    ) || name.starts_with("MONITTER_")
}

fn normalize_value(value: String) -> Result<String, String> {
    if value.len() > MAX_VALUE_BYTES || value.contains('\0') {
        return Err("Environment variable value exceeds the allowed size or format.".into());
    }
    Ok(value)
}

fn normalize_description(description: String) -> Result<String, String> {
    let description = description.trim().to_owned();
    if description.len() > MAX_DESCRIPTION_BYTES || description.chars().any(char::is_control) {
        return Err("Environment variable description exceeds the allowed size or format.".into());
    }
    Ok(description)
}

fn normalize_vault(vault: &mut Vault) -> Result<(), ()> {
    if vault.entries.len() > MAX_ENTRIES {
        return Err(());
    }
    let mut names = HashSet::new();
    for entry in &mut vault.entries {
        entry.name = normalize_name(&entry.name).map_err(|_| ())?;
        entry.value = normalize_value(std::mem::take(&mut entry.value)).map_err(|_| ())?;
        entry.description =
            normalize_description(std::mem::take(&mut entry.description)).map_err(|_| ())?;
        if entry.updated_at < 0 || !names.insert(entry.name.clone()) {
            return Err(());
        }
    }
    vault
        .entries
        .sort_by(|left, right| left.name.cmp(&right.name));
    let bytes = serde_json::to_vec(vault).map_err(|_| ())?;
    (bytes.len() <= MAX_PAYLOAD_BYTES).then_some(()).ok_or(())
}

fn redacted_config(vault: &Vault) -> EnvironmentSecretsConfig {
    EnvironmentSecretsConfig {
        revision: revision_for(vault),
        entries: vault
            .entries
            .iter()
            .map(|entry| EnvironmentSecretMetadata {
                name: entry.name.clone(),
                description: entry.description.clone(),
                updated_at: entry.updated_at,
            })
            .collect(),
    }
}

fn revision_for(vault: &Vault) -> String {
    let encoded = serde_json::to_vec(vault).unwrap_or_default();
    format!("{:x}", Sha256::digest(encoded))
}

fn environment_pairs(vault: &Vault) -> Vec<(String, String)> {
    vault
        .entries
        .iter()
        .map(|entry| (entry.name.clone(), entry.value.clone()))
        .collect()
}

#[cfg(target_os = "macos")]
fn load_vault() -> Result<Vault, String> {
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT)
        .map_err(|_| "Cannot access the macOS Keychain for Environment & Secrets.".to_string())?;
    let raw = match entry.get_password() {
        Ok(raw) => raw,
        Err(keyring::Error::NoEntry) => return Ok(Vault::default()),
        Err(_) => return Err("Cannot access the macOS Keychain for Environment & Secrets.".into()),
    };
    if raw.len() > MAX_PAYLOAD_BYTES {
        return Err("Stored Environment & Secrets data is invalid. It was not changed.".into());
    }
    let mut vault = serde_json::from_str::<Vault>(&raw).map_err(|_| {
        "Stored Environment & Secrets data is invalid. It was not changed.".to_string()
    })?;
    normalize_vault(&mut vault).map_err(|_| {
        "Stored Environment & Secrets data is invalid. It was not changed.".to_string()
    })?;
    Ok(vault)
}

#[cfg(target_os = "macos")]
fn save_vault(vault: &Vault) -> Result<(), String> {
    let raw = serde_json::to_string(vault)
        .map_err(|_| "Cannot prepare Environment & Secrets for storage.".to_string())?;
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT)
        .map_err(|_| "Cannot access the macOS Keychain for Environment & Secrets.".to_string())?;
    entry
        .set_password(&raw)
        .map_err(|_| "Cannot save Environment & Secrets to the macOS Keychain.".to_string())
}

#[cfg(not(target_os = "macos"))]
fn load_vault() -> Result<Vault, String> {
    Err(UNSUPPORTED.into())
}

#[cfg(not(target_os = "macos"))]
fn save_vault(_: &Vault) -> Result<(), String> {
    Err(UNSUPPORTED.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization_rejects_reserved_and_invalid_names_without_values() {
        for name in [
            "",
            "1START",
            "HAS-DASH",
            "PATH",
            "MONITTER_TOKEN",
            "OPENCODE_CONFIG_CONTENT",
        ] {
            assert!(normalize_name(name).is_err(), "{name}");
        }
        assert_eq!(normalize_name("APP_TOKEN").unwrap(), "APP_TOKEN");
    }

    #[test]
    fn redaction_and_injection_helpers_have_separate_shapes() {
        let mut vault = Vault {
            entries: vec![VaultEntry {
                name: "APP_TOKEN".into(),
                value: "fixture-secret".into(),
                description: "API token".into(),
                updated_at: 42,
            }],
        };
        normalize_vault(&mut vault).unwrap();
        let metadata = redacted_config(&vault);
        let rendered = serde_json::to_string(&metadata).unwrap();
        assert!(!rendered.contains("fixture-secret"));
        assert_eq!(
            environment_pairs(&vault),
            vec![("APP_TOKEN".into(), "fixture-secret".into())]
        );
    }

    #[test]
    fn revision_changes_when_only_the_secret_value_changes() {
        let first = Vault {
            entries: vec![VaultEntry {
                name: "APP_TOKEN".into(),
                value: "first".into(),
                description: "API token".into(),
                updated_at: 1,
            }],
        };
        let mut second = first.clone();
        second.entries[0].value = "second".into();
        assert_ne!(revision_for(&first), revision_for(&second));
        assert_eq!(
            redacted_config(&first).entries,
            redacted_config(&second).entries
        );
    }

    #[test]
    fn only_local_user_harnesses_are_eligible_for_injection() {
        assert!(should_inject_into_local_user_command("local", false));
        assert!(!should_inject_into_local_user_command("ssh", false));
        assert!(!should_inject_into_local_user_command("local", true));
    }

    #[test]
    fn command_injection_uses_environment_without_touching_arguments() {
        let mut command = Command::new("fixture-provider");
        command.arg("--stdio");
        apply_pairs_to_command(
            &mut command,
            vec![("APP_TOKEN".into(), "fixture-secret".into())],
        );
        assert_eq!(
            command
                .get_args()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
            vec!["--stdio"]
        );
        assert!(command.get_envs().any(|(name, value)| {
            name == "APP_TOKEN" && value.is_some_and(|value| value == "fixture-secret")
        }));
    }
}
