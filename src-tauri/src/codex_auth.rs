//! Read OpenAI API credentials from a Codex home directory.
//!
//! This module deliberately lives outside `codex_accounts`, which only
//! discovers directory names and explicitly never inspects credentials.
//! `codex_auth` is the only place that opens `auth.json`.
//!
//! Recognised shapes:
//! - `{ "OPENAI_API_KEY": "sk-..." }` → returns `Ok(Some("sk-..."))`
//! - `{ "auth_mode": "chatgpt", "tokens": { ... } }` → returns `Ok(None)`
//!   (ChatGPT subscription tokens cannot call `/v1/models`.)
//! - Anything else → returns `Err(...)`.
//!
//! A missing `auth.json` is reported as `Err(...)` so callers can surface a
//! warning that the picker is falling back to the local Codex CLI catalog.

use serde::Deserialize;
use std::{fs, path::Path};

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum AuthFile {
    ApiKey {
        #[serde(rename = "OPENAI_API_KEY")]
        api_key: String,
    },
    ChatGpt {
        auth_mode: String,
        #[serde(default)]
        tokens: serde_json::Value,
    },
    Other(serde_json::Value),
}

/// Read the OpenAI API key from `auth.json` in the given codex home.
///
/// Returns:
/// - `Ok(Some(key))` if the file declares a non-empty OpenAI API key.
/// - `Ok(None)` if the file declares ChatGPT-OAuth (skip the HTTP fallback).
/// - `Err(...)` if the file is missing, unreadable, or unrecognised.
pub fn read_api_key(codex_home: &str) -> Result<Option<String>, String> {
    let path = Path::new(codex_home).join("auth.json");
    let text = fs::read_to_string(&path)
        .map_err(|error| format!("Could not read {}: {}", path.display(), error))?;
    let parsed: AuthFile = serde_json::from_str(&text)
        .map_err(|error| format!("{} is not a recognised Codex auth file: {}", path.display(), error))?;
    match parsed {
        AuthFile::ApiKey { api_key } if !api_key.trim().is_empty() => Ok(Some(api_key)),
        AuthFile::ApiKey { .. } => Err(format!(
            "{} has an empty OPENAI_API_KEY.",
            path.display()
        )),
        AuthFile::ChatGpt { auth_mode, .. } if auth_mode == "chatgpt" => Ok(None),
        AuthFile::ChatGpt { auth_mode, .. } => Err(format!(
            "{} uses auth_mode '{}' which this build does not recognise.",
            path.display(),
            auth_mode
        )),
        AuthFile::Other(_) => Err(format!(
            "{} is not a recognised Codex auth file (no OPENAI_API_KEY, no auth_mode).",
            path.display()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;

    fn write_auth(contents: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("monitter-auth-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("auth.json");
        let mut file = fs::File::create(&path).unwrap();
        file.write_all(contents.as_bytes()).unwrap();
        dir
    }

    fn empty_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("monitter-auth-empty-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn reads_api_key() {
        let dir = write_auth(r#"{"OPENAI_API_KEY":"sk-test"}"#);
        let key = read_api_key(dir.to_str().unwrap()).unwrap();
        assert_eq!(key.as_deref(), Some("sk-test"));
    }

    #[test]
    fn skips_chatgpt_oauth() {
        let dir = write_auth(
            r#"{"auth_mode":"chatgpt","tokens":{"access_token":"eyJ","refresh_token":"rt","account_id":"acc"}}"#,
        );
        let key = read_api_key(dir.to_str().unwrap()).unwrap();
        assert!(key.is_none());
    }

    #[test]
    fn chatgpt_without_tokens_still_skips() {
        let dir = write_auth(r#"{"auth_mode":"chatgpt"}"#);
        let key = read_api_key(dir.to_str().unwrap()).unwrap();
        assert!(key.is_none());
    }

    #[test]
    fn rejects_empty_api_key() {
        let dir = write_auth(r#"{"OPENAI_API_KEY":"  "}"#);
        assert!(read_api_key(dir.to_str().unwrap()).is_err());
    }

    #[test]
    fn rejects_unknown_shape() {
        let dir = write_auth(r#"{"something":"else"}"#);
        assert!(read_api_key(dir.to_str().unwrap()).is_err());
    }

    #[test]
    fn errors_on_missing_file() {
        let dir = empty_dir();
        assert!(read_api_key(dir.to_str().unwrap()).is_err());
    }
}
