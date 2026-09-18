//! Local Codex account-home discovery and child-process configuration.
//!
//! This module deliberately handles directory names only. It never reads
//! credentials or account configuration from a Codex home.

use serde::Serialize;
use std::{
    collections::BTreeSet,
    env,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodexAccount {
    pub home: String,
    pub label: String,
}

fn home_dir() -> PathBuf {
    env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// Resolve a configured account home, the ambient `CODEX_HOME`, or the normal
/// `~/.codex` location. Every accepted result is an existing canonical
/// directory, so a saved task cannot silently resume under another account.
pub fn effective_home(configured: Option<&str>) -> Result<String, String> {
    let raw = configured
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("CODEX_HOME")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        })
        .unwrap_or_else(|| home_dir().join(".codex"));
    canonical_home(&raw)
}

/// Validate an explicitly configured account home. Explicit values are never
/// expanded from `~` or accepted as relative paths because persisted state
/// must be unambiguous across later launches.
pub fn validate_explicit_home(home: Option<&str>) -> Result<Option<String>, String> {
    let Some(home) = home.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let path = Path::new(home);
    if !path.is_absolute() {
        return Err("Codex account home must be an absolute path.".into());
    }
    canonical_home(path).map(Some)
}

fn canonical_home(path: &Path) -> Result<String, String> {
    let canonical = fs::canonicalize(path)
        .map_err(|error| format!("Codex account home is unavailable: {error}"))?;
    if !canonical.is_dir() {
        return Err("Codex account home must be an existing directory.".into());
    }
    Ok(canonical.to_string_lossy().into_owned())
}

/// Applies the resolved account home to a local Codex child command.
pub fn configure_command(command: &mut Command, home: Option<&str>) -> Result<(), String> {
    command.env("CODEX_HOME", effective_home(home)?);
    Ok(())
}

pub fn account_label(home: &str) -> String {
    let path = Path::new(home);
    match path.file_name().and_then(|name| name.to_str()) {
        Some(".codex") => "Default".into(),
        Some(name) if name.starts_with(".codex-") => name[7..].to_string(),
        Some(name) => name.to_string(),
        None => home.to_string(),
    }
}

/// Finds normal Codex-home directories plus canonicalized configured homes.
/// Only directory paths are exposed; no profile, auth, or credential data is
/// ever inspected.
pub fn list_accounts<I>(configured_homes: I) -> Vec<CodexAccount>
where
    I: IntoIterator<Item = Option<String>>,
{
    discover_accounts(&home_dir(), effective_home(None).ok(), configured_homes)
}

fn discover_accounts<I>(root: &Path, default: Option<String>, configured_homes: I) -> Vec<CodexAccount>
where
    I: IntoIterator<Item = Option<String>>,
{
    let mut candidates = vec![root.join(".codex")];
    // The effective default (including ambient CODEX_HOME) is an explicit
    // account selection, not merely a name-pattern discovery candidate.
    if let Some(home) = default {
        candidates.push(PathBuf::from(home));
    }
    if let Ok(entries) = fs::read_dir(&root) {
        candidates.extend(entries.flatten().map(|entry| entry.path()).filter(|path| {
            path.file_name().and_then(|name| name.to_str()).is_some_and(|name| {
                name.starts_with(".codex-")
            }) && account_metadata_exists(path)
        }));
    }
    candidates.extend(configured_homes.into_iter().flatten().map(PathBuf::from));

    let homes = candidates.into_iter().filter_map(|path| canonical_home(&path).ok()).collect::<BTreeSet<_>>();
    homes.into_iter().map(|home| CodexAccount { label: account_label(&home), home }).collect()
}

fn account_metadata_exists(path: &Path) -> bool {
    path.join("config.toml").is_file() || path.join("auth.json").is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_home_must_be_absolute_existing_directory() {
        assert!(validate_explicit_home(Some(".codex")).is_err());
        assert!(validate_explicit_home(Some("/definitely-not-a-codex-home")).is_err());
        let home = validate_explicit_home(Some("/tmp")).unwrap().unwrap();
        assert_eq!(home, fs::canonicalize("/tmp").unwrap().to_string_lossy());
    }

    #[test]
    fn discovery_keeps_personal_with_custom_default_and_deduplicates_saved_homes() {
        let root = env::temp_dir().join(format!("monitter-accounts-{}", uuid::Uuid::new_v4()));
        for name in [".codex", ".codex-work", ".codex-unrelated", "custom"] {
            fs::create_dir_all(root.join(name)).unwrap();
        }
        // Presence is sufficient; discovery must not parse credentials/config.
        fs::write(root.join(".codex-work/auth.json"), "not valid JSON").unwrap();
        let custom = canonical_home(&root.join("custom")).unwrap();
        let accounts = discover_accounts(&root, Some(custom.clone()), [Some(custom.clone())]);
        let homes = accounts.iter().map(|account| account.home.as_str()).collect::<BTreeSet<_>>();
        assert_eq!(homes.len(), 3);
        assert!(homes.contains(custom.as_str()));
        assert!(homes.contains(canonical_home(&root.join(".codex")).unwrap().as_str()));
        assert!(homes.contains(canonical_home(&root.join(".codex-work")).unwrap().as_str()));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn labels_default_and_named_homes() {
        assert_eq!(account_label("/Users/alex/.codex"), "Default");
        assert_eq!(account_label("/Users/alex/.codex-work"), "work");
    }
}
