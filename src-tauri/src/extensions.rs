//! Private, desktop-only configuration for optional MCP servers and managed
//! prompt skills.  This deliberately has no relationship to `Snapshot`: the
//! values can include credentials and must never enter LAN or visitor views.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

const FILE_NAME: &str = "extensions.json";
const MAX_SERVERS: usize = 32;
const MAX_SKILLS: usize = 64;
const MAX_TOTAL_BYTES: usize = 1024 * 1024;
const MAX_NAME: usize = 128;
const MAX_DESCRIPTION: usize = 4096;
const MAX_COMMAND: usize = 4096;
const MAX_ARGS: usize = 64;
const MAX_ENV: usize = 64;
const MAX_HEADERS: usize = 64;
const MAX_VALUE: usize = 4096;
const MAX_SKILL_CONTENT: usize = 128 * 1024;

#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ExtensionConfig {
    /// Response-only optimistic-concurrency token. It is derived from the
    /// private payload and is never persisted as user configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) revision: Option<String>,
    #[serde(default)]
    pub(crate) mcp_servers: Vec<McpServerConfig>,
    #[serde(default)]
    pub(crate) skills: Vec<ManagedSkill>,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct McpServerConfig {
    pub(crate) id: String,
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) enabled: bool,
    #[serde(default)]
    pub(crate) agent_ids: Vec<String>,
    pub(crate) transport: McpTransport,
    #[serde(default)]
    pub(crate) command: String,
    #[serde(default)]
    pub(crate) args: Vec<String>,
    #[serde(default)]
    pub(crate) env: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) url: String,
    #[serde(default)]
    pub(crate) headers: BTreeMap<String, String>,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum McpTransport {
    Stdio,
    Http,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ManagedSkill {
    pub(crate) id: String,
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) description: String,
    #[serde(default)]
    pub(crate) enabled: bool,
    #[serde(default)]
    pub(crate) agent_ids: Vec<String>,
    /// When enabled, this skill applies to every current and future agent.
    /// Missing values remain false so legacy configurations keep their
    /// explicit-agent-only semantics.
    #[serde(default)]
    pub(crate) all_agents: bool,
    /// Optional provenance for skills imported from a shared catalog.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) source_url: Option<String>,
    #[serde(default)]
    pub(crate) content: String,
}

/// A distinct file makes it structurally impossible for extension credentials
/// to be copied into the normal Snapshot persistence or its LAN projection.
pub(crate) struct ExtensionStore {
    path: PathBuf,
}

impl ExtensionStore {
    pub(crate) fn open(dir: &Path) -> Result<Self, String> {
        if let Ok(metadata) = fs::symlink_metadata(dir) {
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err("Private extension storage is unsafe.".into());
            }
        } else {
            fs::create_dir_all(dir)
                .map_err(|_| "Cannot prepare private extension storage.".to_string())?;
        }
        private_dir(dir)?;
        Ok(Self {
            path: dir.join(FILE_NAME),
        })
    }

    pub(crate) fn load(&self, agent_ids: &HashSet<String>) -> Result<ExtensionConfig, String> {
        match fs::symlink_metadata(&self.path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                Err("Private extension configuration is unsafe; it was not read.".into())
            }
            Ok(metadata) => {
                if !metadata.is_file() {
                    return Err(
                        "Private extension configuration is unsafe; it was not read.".into(),
                    );
                }
                if metadata.len() > MAX_TOTAL_BYTES as u64 {
                    return Err("Private extension configuration exceeds the allowed size; it was not changed.".into());
                }
                let raw = fs::read(&self.path)
                    .map_err(|_| "Cannot read private extension configuration.".to_string())?;
                let config: ExtensionConfig = serde_json::from_slice(&raw).map_err(|_| {
                    "Private extension configuration is corrupt; it was not overwritten."
                        .to_string()
                })?;
                let mut config = normalize(config, agent_ids, true)?;
                config.revision = Some(revision_for(&config)?);
                Ok(config)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let mut config = ExtensionConfig::default();
                config.revision = Some(revision_for(&config)?);
                Ok(config)
            }
            Err(_) => Err("Cannot inspect private extension configuration.".into()),
        }
    }

    pub(crate) fn save(
        &self,
        config: ExtensionConfig,
        agent_ids: &HashSet<String>,
    ) -> Result<ExtensionConfig, String> {
        // Never let a save conceal a malformed or unsafe existing private file.
        // The caller can surface the read error and leave recovery deliberate.
        let _ = self.load(agent_ids)?;
        let mut config = normalize(config, agent_ids, false)?;
        config.revision = None;
        let json = serde_json::to_vec_pretty(&config)
            .map_err(|_| "Cannot encode private extension configuration.".to_string())?;
        if json.len() > MAX_TOTAL_BYTES {
            return Err("Private extension configuration exceeds the allowed size.".into());
        }
        if matches!(fs::symlink_metadata(&self.path), Ok(metadata) if metadata.file_type().is_symlink())
        {
            return Err("Private extension configuration is unsafe; it was not changed.".into());
        }
        let temp = self
            .path
            .with_extension(format!("tmp-{}", uuid::Uuid::new_v4()));
        let mut file = private_create(&temp)?;
        if let Err(_) = file.write_all(&json).and_then(|_| file.sync_all()) {
            let _ = fs::remove_file(&temp);
            return Err("Cannot write private extension configuration.".into());
        }
        drop(file);
        if fs::rename(&temp, &self.path).is_err() {
            let _ = fs::remove_file(&temp);
            return Err("Cannot atomically save private extension configuration.".into());
        }
        private_file(&self.path)?;
        if let Some(parent) = self.path.parent() {
            File::open(parent)
                .and_then(|dir| dir.sync_all())
                .map_err(|_| "Cannot make private extension configuration durable.".to_string())?;
        }
        config.revision = Some(revision_for(&config)?);
        Ok(config)
    }
}

pub(crate) fn normalize_for_save(
    config: ExtensionConfig,
    known_agents: &HashSet<String>,
) -> Result<ExtensionConfig, String> {
    normalize(config, known_agents, false)
}

pub(crate) fn revision_matches(incoming: &ExtensionConfig, loaded: &ExtensionConfig) -> bool {
    incoming.revision.is_some() && incoming.revision == loaded.revision
}

fn revision_for(config: &ExtensionConfig) -> Result<String, String> {
    let mut payload = config.clone();
    payload.revision = None;
    let bytes = serde_json::to_vec(&payload)
        .map_err(|_| "Cannot calculate private extension configuration revision.".to_string())?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn normalize(
    mut config: ExtensionConfig,
    known_agents: &HashSet<String>,
    prune_unknown_agents: bool,
) -> Result<ExtensionConfig, String> {
    if config.mcp_servers.len() > MAX_SERVERS || config.skills.len() > MAX_SKILLS {
        return Err("Too many extension entries.".into());
    }
    let mut ids = HashSet::new();
    let mut names = HashSet::new();
    for server in &mut config.mcp_servers {
        normalize_id(&mut server.id, "MCP server", &mut ids)?;
        normalize_name(&mut server.name, "MCP server", &mut names)?;
        if server.name.eq_ignore_ascii_case("monitter") {
            return Err("The MCP server name Monitter is reserved.".into());
        }
        normalize_agents(&mut server.agent_ids, known_agents, prune_unknown_agents)?;
        bounded_required(&mut server.command, MAX_COMMAND, "MCP stdio command")?;
        bounded_list(&mut server.args, MAX_ARGS, "MCP arguments")?;
        bounded_map(&mut server.env, MAX_ENV, "MCP environment")?;
        bounded_optional(&mut server.url, MAX_VALUE, "MCP URL")?;
        bounded_map(&mut server.headers, MAX_HEADERS, "MCP headers")?;
        match server.transport {
            McpTransport::Stdio if server.command.is_empty() => {
                return Err("MCP stdio command is required.".into())
            }
            McpTransport::Http => validate_http_url(&server.url)?,
            _ => {}
        }
    }
    for skill in &mut config.skills {
        normalize_id(&mut skill.id, "Skill", &mut ids)?;
        normalize_name(&mut skill.name, "Skill", &mut names)?;
        normalize_agents(&mut skill.agent_ids, known_agents, prune_unknown_agents)?;
        bounded_optional(&mut skill.description, MAX_DESCRIPTION, "Skill description")?;
        if let Some(source_url) = &mut skill.source_url {
            bounded_optional(source_url, MAX_VALUE, "Skill source URL")?;
        }
        if skill.content.is_empty()
            || skill.content.len() > MAX_SKILL_CONTENT
            || skill.content.contains('\0')
        {
            return Err("Skill content must be nonempty and within the allowed size.".into());
        }
    }
    Ok(config)
}

/// Returns the explicit agent selection union for changed MCP entries. Skills
/// do not create executable tools and therefore do not affect approvals.
pub(crate) fn changed_mcp_agent_ids(
    previous: &ExtensionConfig,
    next: &ExtensionConfig,
) -> HashSet<String> {
    let before = previous
        .mcp_servers
        .iter()
        .map(|server| (server.id.clone(), server))
        .collect::<HashMap<_, _>>();
    let after = next
        .mcp_servers
        .iter()
        .map(|server| (server.id.clone(), server))
        .collect::<HashMap<_, _>>();
    before
        .keys()
        .chain(after.keys())
        .cloned()
        .collect::<HashSet<_>>()
        .into_iter()
        .filter(|id| before.get(id) != after.get(id))
        .flat_map(|id| {
            before
                .get(&id)
                .into_iter()
                .chain(after.get(&id))
                .flat_map(|server| server.agent_ids.iter().cloned())
        })
        .collect()
}

fn normalize_id(id: &mut String, kind: &str, seen: &mut HashSet<String>) -> Result<(), String> {
    *id = id.trim().to_string();
    let valid = uuid::Uuid::parse_str(id)
        .map(|parsed| parsed.to_string() == *id)
        .unwrap_or(false);
    if !valid || !seen.insert(id.clone()) {
        return Err(format!("{kind} IDs must be unique UUIDs."));
    }
    Ok(())
}

fn normalize_name(name: &mut String, kind: &str, seen: &mut HashSet<String>) -> Result<(), String> {
    *name = name.trim().to_string();
    if name.is_empty() || name.chars().count() > MAX_NAME || !seen.insert(name.to_lowercase()) {
        return Err(format!(
            "{kind} names must be unique and within the allowed length."
        ));
    }
    Ok(())
}

fn normalize_agents(
    ids: &mut Vec<String>,
    known: &HashSet<String>,
    prune_unknown: bool,
) -> Result<(), String> {
    let mut unique = HashSet::new();
    let mut normalized = Vec::with_capacity(ids.len());
    for id in ids.iter_mut() {
        *id = id.trim().to_string();
        if !known.contains(id) {
            if prune_unknown {
                continue;
            }
            return Err("Extension agent selections must reference saved agents.".into());
        }
        if !unique.insert(id.clone()) {
            return Err("Extension agent selections must be unique.".into());
        }
        normalized.push(id.clone());
    }
    *ids = normalized;
    Ok(())
}

fn bounded_required(value: &mut String, limit: usize, label: &str) -> Result<(), String> {
    *value = value.trim().to_string();
    if value.chars().count() > limit || value.contains('\0') {
        return Err(format!("{label} exceeds the allowed length."));
    }
    Ok(())
}

fn bounded_optional(value: &mut String, limit: usize, label: &str) -> Result<(), String> {
    bounded_required(value, limit, label)
}

fn bounded_list(values: &mut Vec<String>, limit: usize, label: &str) -> Result<(), String> {
    if values.len() > limit {
        return Err(format!("{label} has too many values."));
    }
    for value in values {
        if value.chars().count() > MAX_VALUE || value.contains('\0') {
            return Err(format!("{label} value exceeds the allowed length."));
        }
    }
    Ok(())
}

fn bounded_map(
    values: &mut BTreeMap<String, String>,
    limit: usize,
    label: &str,
) -> Result<(), String> {
    if values.len() > limit {
        return Err(format!("{label} has too many values."));
    }
    for (key, value) in values {
        let key_ok = if label == "MCP environment" {
            valid_environment_name(key)
        } else {
            valid_header_name(key)
        };
        if !key_ok
            || value.chars().count() > MAX_VALUE
            || value.contains('\0')
            || value.contains('\r')
            || value.contains('\n')
        {
            return Err(format!("{label} contains an invalid value."));
        }
    }
    Ok(())
}

fn validate_http_url(value: &str) -> Result<(), String> {
    let valid = match tauri::Url::parse(value) {
        Ok(url) => {
            matches!(url.scheme(), "http" | "https")
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none()
                && url.fragment().is_none()
        }
        Err(_) => false,
    };
    if !valid {
        return Err("MCP HTTP URL must be an absolute http or https URL.".into());
    }
    Ok(())
}

fn valid_environment_name(value: &str) -> bool {
    let mut chars = value.bytes();
    matches!(chars.next(), Some(b'A'..=b'Z' | b'a'..=b'z' | b'_'))
        && chars.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn valid_header_name(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| {
            matches!(byte,
            b'0'..=b'9' | b'A'..=b'Z' | b'a'..=b'z' |
            b'!' | b'#' | b'$' | b'%' | b'&' | b'\'' | b'*' | b'+' |
            b'-' | b'.' | b'^' | b'_' | b'`' | b'|' | b'~')
        })
}

#[cfg(unix)]
fn private_create(path: &Path) -> Result<File, String> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|_| "Cannot create private extension configuration.".to_string())
}
#[cfg(not(unix))]
fn private_create(path: &Path) -> Result<File, String> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| "Cannot create private extension configuration.".to_string())
}
#[cfg(unix)]
fn private_dir(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|_| "Cannot secure private extension storage.".to_string())
}
#[cfg(not(unix))]
fn private_dir(_: &Path) -> Result<(), String> {
    Ok(())
}
#[cfg(unix)]
fn private_file(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|_| "Cannot secure private extension configuration.".to_string())
}
#[cfg(not(unix))]
fn private_file(_: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn dir() -> PathBuf {
        std::env::temp_dir().join(format!("monitter-extensions-{}", uuid::Uuid::new_v4()))
    }
    fn config() -> ExtensionConfig {
        ExtensionConfig {
            revision: None,
            mcp_servers: vec![McpServerConfig {
                id: "11111111-1111-4111-8111-111111111111".into(),
                name: "Private search".into(),
                enabled: true,
                agent_ids: vec!["agent".into()],
                transport: McpTransport::Http,
                command: "".into(),
                args: vec![],
                env: BTreeMap::new(),
                url: "https://example.test/mcp".into(),
                headers: BTreeMap::from([("Authorization".into(), "Bearer not-for-errors".into())]),
            }],
            skills: vec![ManagedSkill {
                id: "22222222-2222-4222-8222-222222222222".into(),
                name: "Writing".into(),
                description: "private".into(),
                enabled: true,
                agent_ids: vec![],
                all_agents: false,
                source_url: None,
                content: "Use concise prose.".into(),
            }],
        }
    }
    fn agents() -> HashSet<String> {
        HashSet::from(["agent".into()])
    }
    #[test]
    fn missing_defaults_and_save_round_trips_privately() {
        let path = dir();
        let store = ExtensionStore::open(&path).unwrap();
        let initial = store.load(&agents()).unwrap();
        assert!(initial.mcp_servers.is_empty() && initial.skills.is_empty());
        let saved = store.save(config(), &agents()).unwrap();
        assert!(store.load(&agents()).unwrap() == saved);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(path.join(FILE_NAME))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn legacy_skill_json_defaults_all_agents_to_false() {
        let skill: ManagedSkill = serde_json::from_value(serde_json::json!({
            "id": "22222222-2222-4222-8222-222222222222",
            "name": "Writing",
            "enabled": true,
            "agentIds": [],
            "content": "Use concise prose."
        }))
        .unwrap();
        assert!(!skill.all_agents);
        assert!(skill.source_url.is_none());
    }
    #[test]
    fn invalid_entries_and_unknown_agents_are_rejected() {
        let path = dir();
        let store = ExtensionStore::open(&path).unwrap();
        let mut invalid = config();
        invalid.mcp_servers[0].name = "Monitter".into();
        assert!(store.save(invalid, &agents()).is_err());
        let mut invalid = config();
        invalid.mcp_servers[0].agent_ids = vec!["missing".into()];
        assert!(store.save(invalid, &agents()).is_err());
        let _ = fs::remove_dir_all(path);
    }
    #[test]
    fn corrupt_file_is_not_overwritten_and_errors_do_not_echo_secrets() {
        let path = dir();
        let store = ExtensionStore::open(&path).unwrap();
        let file = path.join(FILE_NAME);
        fs::write(&file, b"{secret-broken-value").unwrap();
        let before = fs::read(&file).unwrap();
        let error = match store.load(&agents()) {
            Ok(_) => panic!("corrupt private config unexpectedly loaded"),
            Err(error) => error,
        };
        assert_eq!(fs::read(&file).unwrap(), before);
        assert!(!error.contains("secret-broken-value"));
        let _ = fs::remove_dir_all(path);
    }
    #[test]
    fn load_prunes_deleted_agents_but_save_rejects_them() {
        let path = dir();
        let store = ExtensionStore::open(&path).unwrap();
        let mut value = config();
        value.mcp_servers[0].agent_ids = vec!["removed".into()];
        fs::write(path.join(FILE_NAME), serde_json::to_vec(&value).unwrap()).unwrap();
        let loaded = store.load(&agents()).unwrap();
        assert!(loaded.mcp_servers[0].agent_ids.is_empty());
        assert!(store.save(value, &agents()).is_err());
        let _ = fs::remove_dir_all(path);
    }
    #[test]
    fn save_does_not_replace_a_corrupt_existing_file() {
        let path = dir();
        let store = ExtensionStore::open(&path).unwrap();
        let file = path.join(FILE_NAME);
        fs::write(&file, b"{not-valid").unwrap();
        let before = fs::read(&file).unwrap();
        assert!(store.save(config(), &agents()).is_err());
        assert_eq!(fs::read(&file).unwrap(), before);
        let _ = fs::remove_dir_all(path);
    }
    #[test]
    fn nonregular_config_file_is_rejected() {
        let path = dir();
        let store = ExtensionStore::open(&path).unwrap();
        fs::create_dir(path.join(FILE_NAME)).unwrap();
        assert!(store.load(&agents()).is_err());
        let _ = fs::remove_dir_all(path);
    }
    #[test]
    fn changed_mcp_servers_revoke_only_their_explicit_agents() {
        let before = config();
        let mut after = before.clone();
        after.mcp_servers[0].url = "https://replacement.test/mcp".into();
        assert_eq!(
            changed_mcp_agent_ids(&before, &after),
            HashSet::from(["agent".into()])
        );
        after.skills[0].content = "different".into();
        assert_eq!(
            changed_mcp_agent_ids(&before, &after),
            HashSet::from(["agent".into()])
        );
    }
    #[test]
    fn revisions_change_with_content_and_reject_stale_submissions() {
        let path = dir();
        let store = ExtensionStore::open(&path).unwrap();
        let before = store.load(&agents()).unwrap();
        let saved = store.save(config(), &agents()).unwrap();
        assert_ne!(before.revision, saved.revision);
        assert!(!revision_matches(&before, &saved));
        assert!(revision_matches(&saved, &saved));
        assert!(serde_json::to_value(&saved).unwrap()["revision"].is_string());
        let disk: serde_json::Value =
            serde_json::from_slice(&fs::read(path.join(FILE_NAME)).unwrap()).unwrap();
        assert!(disk.get("revision").is_none());
        let _ = fs::remove_dir_all(path);
    }
    #[cfg(unix)]
    #[test]
    fn symlink_is_rejected() {
        use std::os::unix::fs::symlink;
        let path = dir();
        let store = ExtensionStore::open(&path).unwrap();
        let target = path.join("target");
        fs::write(&target, b"{}").unwrap();
        symlink(&target, path.join(FILE_NAME)).unwrap();
        assert!(store.load(&agents()).is_err());
        let _ = fs::remove_dir_all(path);
    }
}
