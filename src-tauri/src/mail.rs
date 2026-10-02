//! Provider-neutral, local-only mailbox access for agent MCP tools.
//!
//! Credentials live only in the macOS Keychain vault below.  The public
//! configuration deliberately contains no tokens and must never be copied into
//! a Snapshot.  Gmail is the first provider; providers are reached exclusively
//! through `call_for_agent`, which checks the account grant before *and after*
//! each read.

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::{Read, Write},
    net::{Ipv4Addr, TcpListener, TcpStream},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
use uuid::Uuid;

#[cfg(all(target_os = "macos", not(test)))]
const KEYCHAIN_SERVICE: &str = "com.monitter.desktop.mail";
#[cfg(all(target_os = "macos", not(test)))]
const KEYCHAIN_ACCOUNT: &str = "default";
const MAX_PAYLOAD_BYTES: usize = 512 * 1024;
const MAX_ACCOUNTS: usize = 16;
const MAX_AGENT_IDS: usize = 128;
const MAX_QUERY_BYTES: usize = 4 * 1024;
const OAUTH_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const CALLBACK_MAX_BYTES: usize = 16 * 1024;
const HTTP_MAX_BYTES: usize = 2 * 1024 * 1024;
const ACCESS_TOKEN_TTL_SKEW: i64 = 30_000;
#[cfg(not(target_os = "macos"))]
const UNSUPPORTED: &str = "Mail connections are supported only on macOS.";

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MailConfig {
    pub(crate) revision: String,
    pub(crate) client_id: String,
    pub(crate) client_secret_configured: bool,
    pub(crate) accounts: Vec<MailAccount>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct MailAccount {
    pub(crate) id: String,
    pub(crate) provider: String,
    pub(crate) email: String,
    pub(crate) label: String,
    pub(crate) agent_ids: Vec<String>,
    pub(crate) connected_at: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MailConnectStart {
    pub(crate) attempt_id: String,
    pub(crate) authorization_url: String,
    pub(crate) expires_at: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MailConnectStatus {
    pub(crate) status: String,
    pub(crate) error: Option<String>,
    pub(crate) account: Option<MailAccount>,
}

#[derive(Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MailVault {
    #[serde(default)]
    gmail_client_id: String,
    #[serde(default)]
    gmail_client_secret: Option<String>,
    #[serde(default)]
    accounts: Vec<VaultAccount>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct VaultAccount {
    #[serde(flatten)]
    public: MailAccount,
    refresh_token: String,
    #[serde(default)]
    modify_scope: bool,
}

#[derive(Clone)]
struct AccessToken {
    value: String,
    expires_at: i64,
}

enum AttemptState {
    Pending,
    Connected(MailAccount),
    Error(String),
    Cancelled,
}

struct ConnectAttempt {
    cancel: std::sync::atomic::AtomicBool,
    state: Mutex<AttemptState>,
}

/// Providers own remote requests and normalize their output; the service owns
/// argument validation, account grants, credentials and result revocation checks.
struct SearchRequest {
    query: Option<String>,
    limit: usize,
    page_token: Option<String>,
    label_ids: Vec<String>,
    include_spam_trash: bool,
}
trait MailProvider: Sync {
    fn search(&self, token: &str, request: SearchRequest) -> Result<Value, String>;
    fn read(&self, token: &str, ids: &[String], max_body: usize) -> Result<Vec<Value>, String>;
    fn thread(
        &self,
        token: &str,
        id: &str,
        max_messages: usize,
        max_body: usize,
    ) -> Result<Value, String>;
}
struct GmailProvider;
impl MailProvider for GmailProvider {
    fn search(&self, token: &str, request: SearchRequest) -> Result<Value, String> {
        let query = gmail_search_query(&request);
        let list: GmailList = gmail_get(
            &format!("https://gmail.googleapis.com/gmail/v1/users/me/messages?{query}"),
            token,
        )?;
        if list.messages.len() > request.limit {
            return Err("Gmail returned too many search results.".into());
        }
        let messages = fetch_metadata_bounded(&list.messages, token)?;
        Ok(json!({"messages":messages,"nextPageToken":list.next_page_token}))
    }
    fn read(&self, token: &str, ids: &[String], max_body: usize) -> Result<Vec<Value>, String> {
        fetch_messages_bounded(ids, token, max_body)
    }
    fn thread(
        &self,
        token: &str,
        id: &str,
        max_messages: usize,
        max_body: usize,
    ) -> Result<Value, String> {
        let thread: GmailThread = gmail_get(
            &format!(
                "https://gmail.googleapis.com/gmail/v1/users/me/threads/{}?format=minimal",
                url_encode(id)
            ),
            token,
        )?;
        let truncated = thread.messages.len() > max_messages;
        let ids = thread
            .messages
            .into_iter()
            .take(max_messages)
            .map(|message| message.id)
            .collect::<Vec<_>>();
        let messages = self.read(token, &ids, max_body)?;
        Ok(json!({"threadId":thread.id,"messages":messages,"threadTruncated":truncated}))
    }
}
fn gmail_search_query(request: &SearchRequest) -> String {
    let mut parts = vec![format!("maxResults={}", request.limit)];
    if let Some(query) = &request.query {
        parts.push(format!("q={}", url_encode(query)));
    }
    if let Some(page) = &request.page_token {
        parts.push(format!("pageToken={}", url_encode(page)));
    }
    for label in &request.label_ids {
        parts.push(format!("labelIds={}", url_encode(label)));
    }
    if request.include_spam_trash {
        parts.push("includeSpamTrash=true".into());
    }
    parts.join("&")
}
fn provider_for(account: &VaultAccount) -> Result<&'static dyn MailProvider, String> {
    match account.public.provider.as_str() {
        "gmail" => Ok(&GmailProvider),
        _ => Err("Mail provider is not supported.".into()),
    }
}

/// This service is intentionally outside the app Snapshot. Parent wiring owns
/// command/MCP exposure and must keep this private to the native desktop.
pub(crate) struct MailService {
    writes: Mutex<()>,
    attempts: Mutex<HashMap<String, Arc<ConnectAttempt>>>,
    access_tokens: Mutex<HashMap<String, AccessToken>>,
    /// An agent's launch path can use this after the first metadata read without
    /// repeatedly opening Keychain or touching any credential value.
    granted_agents: Mutex<Option<HashSet<String>>>,
    #[cfg(test)]
    test_vault: Mutex<MailVault>,
}

impl MailService {
    pub(crate) fn new() -> Self {
        Self {
            writes: Mutex::new(()),
            attempts: Mutex::new(HashMap::new()),
            access_tokens: Mutex::new(HashMap::new()),
            granted_agents: Mutex::new(None),
            #[cfg(test)]
            test_vault: Mutex::new(MailVault::default()),
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test() -> Self {
        Self::new()
    }

    #[cfg(test)]
    pub(crate) fn seed_test_account(
        &self,
        account: MailAccount,
        refresh_token: String,
    ) -> Result<(), String> {
        let mut vault = self.test_vault.lock().map_err(|_| unavailable())?;
        vault.accounts.push(VaultAccount {
            public: account,
            refresh_token,
            modify_scope: false,
        });
        normalize_vault(&mut vault)?;
        self.refresh_granted_agents(&vault)?;
        Ok(())
    }

    pub(crate) fn config(&self) -> Result<MailConfig, String> {
        let _guard = self.writes.lock().map_err(|_| unavailable())?;
        let vault = self.read_vault()?;
        self.refresh_granted_agents(&vault)?;
        Ok(public_config(&vault))
    }

    /// Metadata-only fast path for harness setup. The cache holds agent IDs,
    /// never refresh/access tokens or mailbox content.
    pub(crate) fn has_agent_access(&self, agent_id: &str) -> Result<bool, String> {
        #[cfg(all(not(target_os = "macos"), not(test)))]
        {
            let _ = agent_id;
            return Ok(false);
        }
        validate_agent_id(agent_id)?;
        let _guard = self.writes.lock().map_err(|_| unavailable())?;
        if let Some(grants) = self
            .granted_agents
            .lock()
            .map_err(|_| unavailable())?
            .as_ref()
        {
            return Ok(grants.contains(agent_id));
        }
        let vault = self.read_vault()?;
        self.refresh_granted_agents(&vault)?;
        Ok(vault
            .accounts
            .iter()
            .any(|account| account.public.agent_ids.iter().any(|id| id == agent_id)))
    }

    pub(crate) fn configure_gmail(
        &self,
        revision: String,
        client_id: String,
        client_secret: Option<String>,
    ) -> Result<MailConfig, String> {
        let _guard = self.writes.lock().map_err(|_| unavailable())?;
        let mut vault = self.read_vault()?;
        if revision != revision_for(&vault) {
            return Err(conflict());
        }
        let client_id = validate_client_id(&client_id)?;
        if !vault.accounts.is_empty() && vault.gmail_client_id != client_id {
            return Err(
                "Disconnect existing Gmail accounts before changing the OAuth client ID.".into(),
            );
        }
        vault.gmail_client_id = client_id;
        vault.gmail_client_secret = match client_secret {
            Some(secret) => Some(validate_secret(secret)?),
            None => vault.gmail_client_secret,
        };
        normalize_vault(&mut vault)?;
        self.write_vault(&vault)?;
        self.refresh_granted_agents(&vault)?;
        Ok(public_config(&vault))
    }

    pub(crate) fn start_connect(self: &Arc<Self>) -> Result<MailConnectStart, String> {
        let vault = self.read_vault()?;
        if vault.gmail_client_id.is_empty() {
            return Err("Configure a Gmail OAuth client ID before connecting an account.".into());
        }
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .map_err(|_| "Could not start the local Gmail authorization callback.".to_string())?;
        listener.set_nonblocking(true).map_err(|_| {
            "Could not configure the local Gmail authorization callback.".to_string()
        })?;
        let port = listener
            .local_addr()
            .map_err(|_| "Could not start the local Gmail authorization callback.".to_string())?
            .port();
        let state = random_url_token(32)?;
        let verifier = random_url_token(64)?;
        let challenge = base64url(&Sha256::digest(verifier.as_bytes()));
        let redirect_uri = format!("http://127.0.0.1:{port}");
        let attempt_id = Uuid::new_v4().to_string();
        let now = now();
        let expires_at = now + OAUTH_TIMEOUT.as_millis() as i64;
        let authorization_url = format!(
            "https://accounts.google.com/o/oauth2/v2/auth?client_id={}&redirect_uri={}&response_type=code&scope={}&access_type=offline&prompt=consent&code_challenge={}&code_challenge_method=S256&state={}",
            url_encode(&vault.gmail_client_id), url_encode(&redirect_uri), url_encode("https://www.googleapis.com/auth/gmail.modify"), challenge, state
        );
        let attempt = Arc::new(ConnectAttempt {
            cancel: std::sync::atomic::AtomicBool::new(false),
            state: Mutex::new(AttemptState::Pending),
        });
        let mut attempts = self.attempts.lock().map_err(|_| unavailable())?;
        attempts.retain(|_, attempt| {
            matches!(
                *attempt
                    .state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()),
                AttemptState::Pending
            )
        });
        if attempts.len() >= 8 {
            return Err("Too many Gmail authorization attempts are pending.".into());
        }
        attempts.insert(attempt_id.clone(), attempt.clone());
        let service = Arc::clone(self);
        let initial_client_id = vault.gmail_client_id.clone();
        thread::Builder::new()
            .name("monitter-gmail-oauth".into())
            .spawn(move || {
                service.run_gmail_callback(
                    listener,
                    attempt,
                    state,
                    verifier,
                    redirect_uri,
                    initial_client_id,
                );
            })
            .map_err(|_| "Could not start the Gmail authorization worker.".to_string())?;
        Ok(MailConnectStart {
            attempt_id,
            authorization_url,
            expires_at,
        })
    }

    pub(crate) fn connect_status(&self, attempt_id: &str) -> Result<MailConnectStatus, String> {
        let attempt = self
            .attempts
            .lock()
            .map_err(|_| unavailable())?
            .get(attempt_id)
            .cloned()
            .ok_or("Mail connection attempt was not found.")?;
        let state = attempt.state.lock().map_err(|_| unavailable())?;
        Ok(match &*state {
            AttemptState::Pending => MailConnectStatus {
                status: "pending".into(),
                error: None,
                account: None,
            },
            AttemptState::Connected(account) => MailConnectStatus {
                status: "connected".into(),
                error: None,
                account: Some(account.clone()),
            },
            AttemptState::Error(error) => MailConnectStatus {
                status: "error".into(),
                error: Some(error.clone()),
                account: None,
            },
            AttemptState::Cancelled => MailConnectStatus {
                status: "cancelled".into(),
                error: None,
                account: None,
            },
        })
    }

    pub(crate) fn cancel_connect(&self, attempt_id: &str) -> Result<(), String> {
        let attempt = self
            .attempts
            .lock()
            .map_err(|_| unavailable())?
            .get(attempt_id)
            .cloned()
            .ok_or("Mail connection attempt was not found.")?;
        let mut state = attempt.state.lock().map_err(|_| unavailable())?;
        if matches!(*state, AttemptState::Connected(_)) {
            return Err(
                "This account is already connected. Refresh Mail settings to see it.".into(),
            );
        }
        if matches!(*state, AttemptState::Pending) {
            attempt
                .cancel
                .store(true, std::sync::atomic::Ordering::Release);
            *state = AttemptState::Cancelled;
        }
        Ok(())
    }

    pub(crate) fn cancel_all_connects(&self) {
        if let Ok(attempts) = self.attempts.lock() {
            for attempt in attempts.values() {
                attempt
                    .cancel
                    .store(true, std::sync::atomic::Ordering::Release);
                if let Ok(mut state) = attempt.state.lock() {
                    if matches!(*state, AttemptState::Pending) {
                        *state = AttemptState::Cancelled;
                    }
                }
            }
        }
    }

    pub(crate) fn set_account_agents(
        &self,
        revision: String,
        account_id: String,
        agent_ids: Vec<String>,
    ) -> Result<MailConfig, String> {
        let _guard = self.writes.lock().map_err(|_| unavailable())?;
        let mut vault = self.read_vault()?;
        if revision != revision_for(&vault) {
            return Err(conflict());
        }
        let agent_ids = normalize_agent_ids(agent_ids)?;
        let account = vault
            .accounts
            .iter_mut()
            .find(|a| a.public.id == account_id)
            .ok_or("Mail account was not found. Refresh before saving.")?;
        account.public.agent_ids = agent_ids;
        self.write_vault(&vault)?;
        self.refresh_granted_agents(&vault)?;
        Ok(public_config(&vault))
    }

    pub(crate) fn disconnect(
        &self,
        revision: String,
        account_id: String,
    ) -> Result<MailConfig, String> {
        let _guard = self.writes.lock().map_err(|_| unavailable())?;
        let mut vault = self.read_vault()?;
        if revision != revision_for(&vault) {
            return Err(conflict());
        }
        let before = vault.accounts.len();
        vault.accounts.retain(|a| a.public.id != account_id);
        if before == vault.accounts.len() {
            return Err("Mail account was not found. Refresh before disconnecting.".into());
        }
        self.write_vault(&vault)?;
        self.refresh_granted_agents(&vault)?;
        self.access_tokens
            .lock()
            .map_err(|_| unavailable())?
            .remove(&account_id);
        Ok(public_config(&vault))
    }

    /// The sole provider-neutral agent entry point. Errors are user-safe and
    /// never include OAuth tokens, message bodies, or provider response bodies.
    pub(crate) fn call_for_agent(
        &self,
        agent_id: &str,
        name: &str,
        args: Value,
    ) -> Result<Value, String> {
        validate_agent_id(agent_id)?;
        match name {
            "mail_accounts" => {
                require_empty(&args)?;
                Ok(
                    json!({"accounts": self.accounts_for(agent_id)?.into_iter().map(|account| json!({"id":account.id,"provider":account.provider,"email":account.email,"label":account.label,"connectedAt":account.connected_at,"capabilities":{"search":true,"read":true,"thread":true}})).collect::<Vec<_>>() }),
                )
            }
            "mail_search" => self.mail_search(agent_id, args),
            "mail_read" => {
                let mut result = self.mail_read(agent_id, args)?;
                strip_html_bodies(&mut result);
                Ok(result)
            }
            "mail_thread" => {
                let mut result = self.mail_thread(agent_id, args)?;
                strip_html_bodies(&mut result);
                Ok(result)
            }
            _ => Err("Unknown mail tool.".into()),
        }
    }

    /// Native inbox operation; never exposed through the agent tool dispatcher.
    pub(crate) fn native_search_unread(
        &self,
        agent_id: &str,
        account_id: &str,
    ) -> Result<Value, String> {
        validate_agent_id(agent_id)?;
        let account = self.authorized_account(agent_id, account_id)?;
        let token = self.access_token(&account)?;
        self.recheck_grant(agent_id, account_id)?;
        let mut result = provider_for(&account)?.search(
            &token,
            SearchRequest {
                query: Some("is:unread in:inbox".into()),
                limit: 100,
                page_token: None,
                label_ids: Vec::new(),
                include_spam_trash: false,
            },
        )?;
        self.recheck_grant(agent_id, account_id)?;
        result["accountId"] = json!(account_id);
        Ok(result)
    }

    pub(crate) fn native_thread(
        &self,
        agent_id: &str,
        account_id: &str,
        thread_id: &str,
    ) -> Result<Value, String> {
        validate_agent_id(agent_id)?;
        self.mail_thread(agent_id, json!({"account_id":account_id, "thread_id":thread_id, "max_messages":50, "max_body_chars":50_000}))
    }

    pub(crate) fn native_read(
        &self,
        agent_id: &str,
        account_id: &str,
        message_id: &str,
    ) -> Result<Value, String> {
        self.mail_read(agent_id, json!({"account_id":account_id, "message_ids":[message_id], "max_body_chars":50_000}))
    }

    pub(crate) fn native_mark_read(
        &self,
        agent_id: &str,
        account_id: &str,
        message_id: &str,
    ) -> Result<(), String> {
        validate_agent_id(agent_id)?;
        validate_provider_id(message_id)?;
        let account = self.authorized_account(agent_id, account_id)?;
        if !account.modify_scope {
            return Err("Reconnect this Gmail account to authorize marking messages read.".into());
        }
        let token = self.access_token(&account)?;
        self.recheck_grant(agent_id, account_id)?;
        curl_request_with_headers(
            "POST",
            &format!(
                "https://gmail.googleapis.com/gmail/v1/users/me/messages/{}/modify",
                url_encode(message_id)
            ),
            &[&format!("Authorization: Bearer {token}"), "Content-Type: application/json"],
            Some(br#"{"removeLabelIds":["UNREAD"]}"#),
        )?;
        self.recheck_grant(agent_id, account_id)
    }

    fn accounts_for(&self, agent_id: &str) -> Result<Vec<MailAccount>, String> {
        Ok(self
            .read_vault()?
            .accounts
            .into_iter()
            .filter_map(|a| {
                a.public
                    .agent_ids
                    .contains(&agent_id.to_string())
                    .then_some(a.public)
            })
            .collect())
    }

    fn mail_search(&self, agent_id: &str, args: Value) -> Result<Value, String> {
        let object = object_args(
            args,
            &[
                "account_id",
                "query",
                "limit",
                "page_token",
                "label_ids",
                "include_spam_trash",
            ],
        )?;
        let account_id = required_string(&object, "account_id", 64)?;
        let query = optional_string(&object, "query", MAX_QUERY_BYTES)?;
        let limit = optional_usize(&object, "limit", 20, 1, 50)?;
        let page = optional_string(&object, "page_token", 4096)?;
        let labels = optional_string_array(&object, "label_ids", 0, 20)?;
        let spam = optional_bool(&object, "include_spam_trash")?.unwrap_or(false);
        let account = self.authorized_account(agent_id, &account_id)?;
        let token = self.access_token(&account)?;
        let mut result = provider_for(&account)?.search(
            &token,
            SearchRequest {
                query,
                limit,
                page_token: page,
                label_ids: labels,
                include_spam_trash: spam,
            },
        )?;
        self.recheck_grant(agent_id, &account_id)?;
        result["accountId"] = json!(account_id);
        Ok(result)
    }

    fn mail_read(&self, agent_id: &str, args: Value) -> Result<Value, String> {
        let object = object_args(args, &["account_id", "message_ids", "max_body_chars"])?;
        let account_id = required_string(&object, "account_id", 64)?;
        let ids = required_string_array(&object, "message_ids", 1, 20, 256)?;
        let max_body = optional_usize(&object, "max_body_chars", 20_000, 1, 50_000)?;
        let account = self.authorized_account(agent_id, &account_id)?;
        let token = self.access_token(&account)?;
        let messages = provider_for(&account)?.read(&token, &ids, max_body)?;
        self.recheck_grant(agent_id, &account_id)?;
        Ok(json!({"accountId": account_id, "messages": messages}))
    }

    fn mail_thread(&self, agent_id: &str, args: Value) -> Result<Value, String> {
        let object = object_args(
            args,
            &["account_id", "thread_id", "max_messages", "max_body_chars"],
        )?;
        let account_id = required_string(&object, "account_id", 64)?;
        let thread_id = required_string(&object, "thread_id", 256)?;
        validate_provider_id(&thread_id)?;
        let max_messages = optional_usize(&object, "max_messages", 20, 1, 50)?;
        let max_body = optional_usize(&object, "max_body_chars", 20_000, 1, 50_000)?;
        let account = self.authorized_account(agent_id, &account_id)?;
        let token = self.access_token(&account)?;
        let mut result =
            provider_for(&account)?.thread(&token, &thread_id, max_messages, max_body)?;
        self.recheck_grant(agent_id, &account_id)?;
        result["accountId"] = json!(account_id);
        Ok(result)
    }

    fn authorized_account(&self, agent_id: &str, account_id: &str) -> Result<VaultAccount, String> {
        let account = self
            .read_vault()?
            .accounts
            .into_iter()
            .find(|a| a.public.id == account_id)
            .ok_or("Mail account was not found.")?;
        provider_for(&account)?;
        if !account.public.agent_ids.iter().any(|id| id == agent_id) {
            return Err("This agent is not allowed to access that mail account.".into());
        }
        Ok(account)
    }
    fn recheck_grant(&self, agent_id: &str, account_id: &str) -> Result<(), String> {
        self.authorized_account(agent_id, account_id).map(|_| ())
    }
    fn access_token(&self, account: &VaultAccount) -> Result<String, String> {
        let now = now();
        if let Some(token) = self
            .access_tokens
            .lock()
            .map_err(|_| unavailable())?
            .get(&account.public.id)
        {
            if token.expires_at > now + ACCESS_TOKEN_TTL_SKEW {
                return Ok(token.value.clone());
            }
        }
        // Serialize refreshes, and reload the account while holding the same
        // lock used for vault updates so a disconnect cannot race a refresh.
        let _guard = self.writes.lock().map_err(|_| unavailable())?;
        if let Some(token) = self
            .access_tokens
            .lock()
            .map_err(|_| unavailable())?
            .get(&account.public.id)
        {
            if token.expires_at > now + ACCESS_TOKEN_TTL_SKEW {
                return Ok(token.value.clone());
            }
        }
        let vault = self.read_vault()?;
        let account = vault
            .accounts
            .iter()
            .find(|a| a.public.id == account.public.id)
            .ok_or("Mail account was disconnected.")?;
        let refreshed = gmail_refresh(
            &vault.gmail_client_id,
            vault.gmail_client_secret.as_deref(),
            &account.refresh_token,
        )?;
        self.access_tokens
            .lock()
            .map_err(|_| unavailable())?
            .insert(account.public.id.clone(), refreshed.clone());
        Ok(refreshed.value)
    }

    fn run_gmail_callback(
        &self,
        listener: TcpListener,
        attempt: Arc<ConnectAttempt>,
        expected_state: String,
        verifier: String,
        redirect_uri: String,
        initial_client_id: String,
    ) {
        let deadline = Instant::now() + OAUTH_TIMEOUT;
        loop {
            if attempt.cancel.load(std::sync::atomic::Ordering::Acquire) {
                return;
            }
            if Instant::now() >= deadline {
                set_attempt_error(
                    &attempt,
                    "Gmail authorization timed out. Start again when ready.",
                );
                return;
            }
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let result = callback_parameters(&mut stream).and_then(|params| {
                        if params.get("state") != Some(&expected_state) {
                            return Err("Ignore callback.".into());
                        }
                        if params.contains_key("error") {
                            return Err("Gmail authorization was not completed.".into());
                        }
                        let code = params
                            .get("code")
                            .ok_or("Gmail authorization did not return a code.")?;
                        let account = self.exchange_gmail_code(
                            code,
                            &verifier,
                            &redirect_uri,
                            &expected_state,
                            &initial_client_id,
                            &attempt,
                        )?;
                        write_callback(&mut stream, true);
                        Ok(account)
                    });
                    if matches!(&result, Err(error) if error == "Ignore callback.") {
                        write_callback(&mut stream, false);
                        continue;
                    }
                    if let Err(_) = &result {
                        write_callback(&mut stream, false);
                    }
                    match result {
                        Ok(account) => set_attempt_connected(&attempt, account),
                        Err(error) => set_attempt_error(&attempt, &error),
                    }
                    return;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(25))
                }
                Err(_) => {
                    set_attempt_error(
                        &attempt,
                        "Gmail authorization callback failed. Start again.",
                    );
                    return;
                }
            }
        }
    }

    fn exchange_gmail_code(
        &self,
        code: &str,
        verifier: &str,
        redirect_uri: &str,
        _state: &str,
        initial_client_id: &str,
        attempt: &ConnectAttempt,
    ) -> Result<MailAccount, String> {
        let _guard = self.writes.lock().map_err(|_| unavailable())?;
        if attempt.cancel.load(std::sync::atomic::Ordering::Acquire) {
            return Err("Gmail authorization was cancelled.".into());
        }
        let mut vault = self.read_vault()?;
        if vault.gmail_client_id != initial_client_id {
            return Err(
                "Gmail OAuth settings changed while authorization was pending. Start again.".into(),
            );
        }
        let token = gmail_exchange(
            &vault.gmail_client_id,
            vault.gmail_client_secret.as_deref(),
            code,
            verifier,
            redirect_uri,
        )?;
        if token.access_token.is_empty() {
            return Err("Gmail did not return an access token.".into());
        }
        let access = AccessToken {
            value: token.access_token,
            expires_at: now() + token.expires_in.max(60) * 1000,
        };
        let email = gmail_email(&access.value)?;
        let modify_scope = token.scope.as_deref().map(|scope| {
            scope
                .split_whitespace()
                .any(|value| value == "https://www.googleapis.com/auth/gmail.modify")
        }).unwrap_or(true);
        let existing = vault.accounts.iter().position(|a| {
            a.public.provider == "gmail" && a.public.email.eq_ignore_ascii_case(&email)
        });
        let account = if let Some(index) = existing {
            let existing = &mut vault.accounts[index];
            existing.refresh_token = token
                .refresh_token
                .unwrap_or_else(|| existing.refresh_token.clone());
            existing.modify_scope = modify_scope;
            existing.public.clone()
        } else {
            if vault.accounts.len() >= MAX_ACCOUNTS {
                return Err("Mail has reached its account limit.".into());
            }
            let account = MailAccount {
                id: Uuid::new_v4().to_string(),
                provider: "gmail".into(),
                email: email.clone(),
                label: email,
                agent_ids: Vec::new(),
                connected_at: now(),
            };
            vault.accounts.push(VaultAccount {
                public: account.clone(),
                refresh_token: token.refresh_token.ok_or(
                    "Gmail did not return a refresh token. Start again and grant offline access.",
                )?,
                modify_scope,
            });
            account
        };
        normalize_vault(&mut vault)?;
        let mut attempt_state = attempt.state.lock().map_err(|_| unavailable())?;
        if attempt.cancel.load(std::sync::atomic::Ordering::Acquire)
            || !matches!(*attempt_state, AttemptState::Pending)
        {
            return Err("Gmail authorization was cancelled.".into());
        }
        self.write_vault(&vault)?;
        *attempt_state = AttemptState::Connected(account.clone());
        self.refresh_granted_agents(&vault)?;
        self.access_tokens
            .lock()
            .map_err(|_| unavailable())?
            .insert(account.id.clone(), access);
        Ok(account)
    }

    #[cfg(not(test))]
    fn read_vault(&self) -> Result<MailVault, String> {
        load_vault()
    }
    #[cfg(test)]
    fn read_vault(&self) -> Result<MailVault, String> {
        self.test_vault
            .lock()
            .map(|vault| vault.clone())
            .map_err(|_| unavailable())
    }
    #[cfg(not(test))]
    fn write_vault(&self, vault: &MailVault) -> Result<(), String> {
        save_vault(vault)
    }
    #[cfg(test)]
    fn write_vault(&self, vault: &MailVault) -> Result<(), String> {
        *self.test_vault.lock().map_err(|_| unavailable())? = vault.clone();
        Ok(())
    }
    fn refresh_granted_agents(&self, vault: &MailVault) -> Result<(), String> {
        let agents = vault
            .accounts
            .iter()
            .flat_map(|account| account.public.agent_ids.iter().cloned())
            .collect();
        *self.granted_agents.lock().map_err(|_| unavailable())? = Some(agents);
        Ok(())
    }
}

fn strip_html_bodies(result: &mut Value) {
    if let Some(messages) = result.get_mut("messages").and_then(Value::as_array_mut) {
        for message in messages {
            if let Some(object) = message.as_object_mut() { object.remove("bodyHtml"); }
        }
    }
}

#[derive(Deserialize)]
struct GmailToken {
    access_token: String,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: i64,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GmailProfile {
    email_address: String,
}
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct GmailList {
    #[serde(default)]
    messages: Vec<GmailMessageRef>,
    next_page_token: Option<String>,
}
#[derive(Clone, Deserialize)]
struct GmailMessageRef {
    id: String,
}
#[derive(Deserialize)]
struct GmailThread {
    id: String,
    #[serde(default)]
    messages: Vec<GmailMessage>,
}
#[derive(Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct GmailMessage {
    id: String,
    thread_id: String,
    #[serde(default)]
    label_ids: Vec<String>,
    #[serde(default)]
    snippet: String,
    #[serde(default)]
    internal_date: String,
    payload: Option<GmailPart>,
}
#[derive(Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct GmailPart {
    mime_type: Option<String>,
    #[serde(default)]
    headers: Vec<GmailHeader>,
    body: Option<GmailBody>,
    #[serde(default)]
    parts: Vec<GmailPart>,
    filename: Option<String>,
}
#[derive(Clone, Deserialize)]
struct GmailHeader {
    name: String,
    value: String,
}
#[derive(Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct GmailBody {
    data: Option<String>,
    attachment_id: Option<String>,
    #[serde(default)]
    size: i64,
}

fn gmail_exchange(
    client_id: &str,
    secret: Option<&str>,
    code: &str,
    verifier: &str,
    redirect: &str,
) -> Result<GmailToken, String> {
    let mut form = vec![
        ("client_id", client_id),
        ("code", code),
        ("code_verifier", verifier),
        ("grant_type", "authorization_code"),
        ("redirect_uri", redirect),
    ];
    if let Some(secret) = secret {
        form.push(("client_secret", secret));
    }
    gmail_post_form("https://oauth2.googleapis.com/token", &form)
}
fn gmail_refresh(
    client_id: &str,
    secret: Option<&str>,
    refresh: &str,
) -> Result<AccessToken, String> {
    let mut form = vec![
        ("client_id", client_id),
        ("refresh_token", refresh),
        ("grant_type", "refresh_token"),
    ];
    if let Some(secret) = secret {
        form.push(("client_secret", secret));
    }
    let token: GmailToken = gmail_post_form("https://oauth2.googleapis.com/token", &form)?;
    if token.access_token.is_empty() {
        return Err("Gmail did not return an access token.".into());
    }
    Ok(AccessToken {
        value: token.access_token,
        expires_at: now() + token.expires_in.max(60) * 1000,
    })
}
fn gmail_email(token: &str) -> Result<String, String> {
    let profile: GmailProfile = gmail_get(
        "https://gmail.googleapis.com/gmail/v1/users/me/profile",
        token,
    )?;
    validate_email(&profile.email_address)
}
fn gmail_get<T: for<'de> Deserialize<'de>>(url: &str, token: &str) -> Result<T, String> {
    let bytes = curl_request(
        "GET",
        url,
        Some(&format!("Authorization: Bearer {token}")),
        None,
    )?;
    serde_json::from_slice(&bytes).map_err(|_| "Gmail returned an invalid response.".into())
}
fn gmail_post_form<T: for<'de> Deserialize<'de>>(
    url: &str,
    form: &[(&str, &str)],
) -> Result<T, String> {
    let body = form
        .iter()
        .map(|(k, v)| format!("{}={}", url_encode(k), url_encode(v)))
        .collect::<Vec<_>>()
        .join("&");
    let bytes = curl_request(
        "POST",
        url,
        Some("Content-Type: application/x-www-form-urlencoded"),
        Some(body.as_bytes()),
    )?;
    serde_json::from_slice(&bytes).map_err(|_| "Gmail returned an invalid response.".into())
}

fn fetch_metadata_bounded(refs: &[GmailMessageRef], token: &str) -> Result<Vec<Value>, String> {
    let mut results: Vec<Option<Value>> = vec![None; refs.len()];
    let mut next = 0;
    while next < refs.len() {
        let end = (next + 4).min(refs.len());
        thread::scope(|scope| {
            let mut joins = Vec::new();
            for (index, item) in refs[next..end].iter().enumerate() {
                let id = item.id.clone();
                joins.push((
                    next + index,
                    scope.spawn(move || read_message(&id, token, 0)),
                ));
            }
            for (index, join) in joins {
                results[index] = Some(
                    join.join()
                        .map_err(|_| "Gmail message retrieval failed.")??,
                );
            }
            Ok::<(), String>(())
        })?;
        next = end;
    }
    Ok(results.into_iter().flatten().collect())
}
fn fetch_messages_bounded(
    ids: &[String],
    token: &str,
    max_body: usize,
) -> Result<Vec<Value>, String> {
    let mut results: Vec<Option<Value>> = vec![None; ids.len()];
    let mut next = 0;
    while next < ids.len() {
        let end = (next + 4).min(ids.len());
        thread::scope(|scope| {
            let mut joins = Vec::new();
            for (index, id) in ids[next..end].iter().enumerate() {
                joins.push((
                    next + index,
                    scope.spawn(move || read_message(id, token, max_body)),
                ));
            }
            for (index, join) in joins {
                results[index] = Some(
                    join.join()
                        .map_err(|_| "Gmail message retrieval failed.")??,
                );
            }
            Ok::<(), String>(())
        })?;
        next = end;
    }
    Ok(results.into_iter().flatten().collect())
}
fn read_message(id: &str, token: &str, max_body: usize) -> Result<Value, String> {
    validate_provider_id(id)?;
    let format = if max_body == 0 {
        "metadata&metadataHeaders=From&metadataHeaders=To&metadataHeaders=Cc&metadataHeaders=Subject&metadataHeaders=Date"
    } else {
        "full"
    };
    let message: GmailMessage = gmail_get(
        &format!(
            "https://gmail.googleapis.com/gmail/v1/users/me/messages/{}?format={format}",
            url_encode(id)
        ),
        token,
    )?;
    Ok(normalize_message(message, max_body))
}
fn normalize_message(message: GmailMessage, max_body: usize) -> Value {
    let mut headers = HashMap::new();
    let mut texts = Vec::new();
    let mut html = Vec::new();
    let mut attachments = Vec::new();
    if let Some(payload) = &message.payload {
        collect_part(
            payload,
            &mut headers,
            &mut texts,
            &mut html,
            &mut attachments,
        );
    }
    let body_unavailable = max_body == 0 || message.payload.as_ref().is_some_and(has_external_body);
    // Raw HTML is untrusted and must be rendered only in an isolated sandbox.
    let body_html =
        (max_body > 0 && !html.is_empty()).then(|| truncate_chars(html.join("\n"), max_body));
    let (body, body_truncated) = if max_body == 0 {
        (None, false)
    } else {
        let text = if texts.is_empty() {
            html.into_iter()
                .map(|x| html_to_text(&x))
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            texts.join("\n")
        };
        let truncated = text.chars().count() > max_body;
        (Some(truncate_chars(text, max_body)), truncated)
    };
    json!({"id": message.id, "threadId": message.thread_id, "from": headers.remove("from"), "to": headers.remove("to"), "cc": headers.remove("cc"), "subject": headers.remove("subject"), "date": headers.remove("date"), "receivedAt": message.internal_date.parse::<i64>().ok(), "snippet": truncate_chars(message.snippet, 1024), "labelIds": message.label_ids, "isRead": !message.label_ids.iter().any(|x| x == "UNREAD"), "body": body, "bodyHtml": body_html, "bodyTruncated": body_truncated, "bodyUnavailable": body_unavailable, "attachments": attachments})
}
fn has_external_body(part: &GmailPart) -> bool {
    let is_body = part.filename.as_deref().unwrap_or("").is_empty()
        && matches!(part.mime_type.as_deref(), Some("text/plain" | "text/html"));
    (is_body
        && part
            .body
            .as_ref()
            .is_some_and(|body| body.attachment_id.is_some() && body.data.is_none()))
        || part.parts.iter().any(has_external_body)
}
fn collect_part(
    part: &GmailPart,
    headers: &mut HashMap<String, String>,
    texts: &mut Vec<String>,
    html: &mut Vec<String>,
    attachments: &mut Vec<Value>,
) {
    for header in &part.headers {
        let name = header.name.to_ascii_lowercase();
        if ["from", "to", "cc", "subject", "date"].contains(&name.as_str()) {
            headers
                .entry(name)
                .or_insert_with(|| truncate_chars(header.value.clone(), 4096));
        }
    }
    let mime = part.mime_type.as_deref().unwrap_or("");
    if let Some(body) = &part.body {
        if body.attachment_id.is_some() || !part.filename.as_deref().unwrap_or("").is_empty() {
            if attachments.len() < 100 {
                attachments.push(json!({"filename": part.filename.as_ref().map(|v| truncate_chars(v.clone(),512)), "mimeType": truncate_chars(mime.to_string(),256), "size": body.size, "attachmentId": body.attachment_id.as_ref().map(|v| truncate_chars(v.clone(),256))}));
            }
        } else if let Some(data) = &body.data {
            if let Ok(decoded) = base64url_decode(data) {
                let value = String::from_utf8_lossy(&decoded).to_string();
                if mime.eq_ignore_ascii_case("text/plain") {
                    texts.push(value);
                } else if mime.eq_ignore_ascii_case("text/html") {
                    html.push(value);
                }
            }
        }
    }
    for child in &part.parts {
        collect_part(child, headers, texts, html, attachments);
    }
}

struct CurlChild(std::process::Child);
impl std::ops::Deref for CurlChild {
    type Target = std::process::Child;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for CurlChild {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
impl Drop for CurlChild {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}
fn curl_request(
    method: &str,
    url: &str,
    header: Option<&str>,
    body: Option<&[u8]>,
) -> Result<Vec<u8>, String> {
    curl_request_with_headers(method, url, &header.into_iter().collect::<Vec<_>>(), body)
}
fn curl_request_with_headers(
    method: &str,
    url: &str,
    headers: &[&str],
    body: Option<&[u8]>,
) -> Result<Vec<u8>, String> {
    // Secrets only travel through curl's stdin config. Do not put OAuth forms,
    // bearer tokens, or provider responses in argv, environment, or temp files.
    let result = (|| {
        let mut child = CurlChild(
            Command::new("/usr/bin/curl")
                .args(["--disable", "--config", "-"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|_| "Could not start Gmail HTTPS client.".to_string())?,
        );
        let mut config = format!("url = {}\nrequest = {}\nsilent\nshow-error\nproto = \"=https\"\nconnect-timeout = 5\nmax-time = 20\nmax-redirs = 0\nmax-filesize = {}\nwrite-out = \"\\n%{{http_code}}\"\n", curl_value(url), curl_value(method), HTTP_MAX_BYTES);
        for header in headers {
            config.push_str(&format!("header = {}\n", curl_value(header)));
        }
        if let Some(body) = body {
            let body = std::str::from_utf8(body).map_err(|_| "Could not prepare Gmail request.")?;
            if body.starts_with('{') {
                config.push_str("header = \"Content-Type: application/json\"\n");
            }
            config.push_str(&format!("data = {}\n", curl_value(body)));
        }
        child
            .stdin
            .as_mut()
            .ok_or("Could not start Gmail HTTPS client.")?
            .write_all(config.as_bytes())
            .map_err(|_| "Could not start Gmail HTTPS client.")?;
        drop(child.stdin.take());
        let mut output = Vec::with_capacity(4096);
        let mut stdout = child.stdout.take().ok_or("Gmail HTTPS request failed.")?;
        let copied = std::io::Read::by_ref(&mut stdout)
            .take((HTTP_MAX_BYTES + 16) as u64)
            .read_to_end(&mut output)
            .map_err(|_| "Gmail HTTPS request failed.")?;
        if copied > HTTP_MAX_BYTES {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Gmail response exceeded the allowed size.".into());
        }
        let result = child.wait().map_err(|_| "Gmail HTTPS request failed.")?;
        let marker = output
            .iter()
            .rposition(|byte| *byte == b'\n')
            .ok_or("Gmail HTTPS request failed.")?;
        let status = std::str::from_utf8(&output[marker + 1..])
            .unwrap_or("")
            .trim()
            .to_owned();
        output.truncate(marker);
        if !result.success() || !matches!(status.as_str(), "200" | "201") {
            return Err(if status == "401" {
                "Gmail access expired or was revoked. Reconnect the account.".into()
            } else if status == "403" {
                "Gmail denied access. Check that Gmail API is enabled and the account granted read-only mail access.".into()
            } else if status == "429" {
                "Gmail is rate limiting requests. Try again shortly.".into()
            } else {
                "Gmail request failed. Check your connection and Google app configuration.".into()
            });
        }
        Ok(output)
    })();
    result
}

fn callback_parameters(stream: &mut TcpStream) -> Result<HashMap<String, String>, String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(1)))
        .map_err(|_| "Gmail authorization callback failed.")?;
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 512];
    let deadline = Instant::now() + Duration::from_secs(3);
    while bytes.len() < CALLBACK_MAX_BYTES {
        if Instant::now() >= deadline {
            return Err("Ignore callback.".into());
        }
        let n = stream
            .read(&mut chunk)
            .map_err(|_| "Ignore callback.")?;
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..n]);
        if bytes.windows(4).any(|item| item == b"\r\n\r\n") {
            break;
        }
    }
    if bytes.len() >= CALLBACK_MAX_BYTES {
        return Err("Ignore callback.".into());
    }
    let line = String::from_utf8_lossy(&bytes)
        .lines()
        .next()
        .unwrap_or_default()
        .to_string();
    let mut parts = line.split_whitespace();
    if parts.next() != Some("GET") {
        return Err("Ignore callback.".into());
    }
    let path = parts.next().ok_or("Ignore callback.")?;
    let (route, query) = path.split_once('?').unwrap_or((path, ""));
    if route != "/" {
        return Err("Ignore callback.".into());
    }
    parse_query(query)
}
fn write_callback(stream: &mut TcpStream, success: bool) {
    let page = if success {
        "Gmail connected. You can return to Monitter."
    } else {
        "Gmail connection could not be completed. You can return to Monitter."
    };
    let _ = stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", page.len(), page).as_bytes());
}
fn parse_query(query: &str) -> Result<HashMap<String, String>, String> {
    let mut out = HashMap::new();
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').ok_or("Ignore callback.")?;
        let key = url_decode(key).ok_or("Ignore callback.")?;
        let value = url_decode(value).ok_or("Ignore callback.")?;
        if out.insert(key, value).is_some() {
            return Err("Ignore callback.".into());
        }
    }
    Ok(out)
}

fn public_config(vault: &MailVault) -> MailConfig {
    MailConfig {
        revision: revision_for(vault),
        client_id: vault.gmail_client_id.clone(),
        client_secret_configured: vault
            .gmail_client_secret
            .as_ref()
            .is_some_and(|s| !s.is_empty()),
        accounts: vault.accounts.iter().map(|a| a.public.clone()).collect(),
    }
}
fn revision_for(vault: &MailVault) -> String {
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(vault).unwrap_or_default())
    )
}
fn normalize_vault(vault: &mut MailVault) -> Result<(), String> {
    vault.gmail_client_id = if vault.gmail_client_id.is_empty() {
        String::new()
    } else {
        validate_client_id(&vault.gmail_client_id)?
    };
    if let Some(secret) = vault.gmail_client_secret.take() {
        vault.gmail_client_secret = Some(validate_secret(secret)?);
    }
    if vault.accounts.len() > MAX_ACCOUNTS {
        return Err("Stored Mail data is invalid. It was not changed.".into());
    }
    let mut ids = HashSet::new();
    for account in &mut vault.accounts {
        normalize_account(&mut account.public)?;
        if account.refresh_token.is_empty()
            || account.refresh_token.len() > 8192
            || !ids.insert(account.public.id.clone())
        {
            return Err("Stored Mail data is invalid. It was not changed.".into());
        }
    }
    let bytes = serde_json::to_vec(vault)
        .map_err(|_| "Stored Mail data is invalid. It was not changed.".to_string())?;
    if bytes.len() > MAX_PAYLOAD_BYTES {
        return Err("Stored Mail data is invalid. It was not changed.".into());
    }
    Ok(())
}
fn normalize_account(account: &mut MailAccount) -> Result<(), String> {
    if Uuid::parse_str(&account.id).is_err()
        || account.provider != "gmail"
        || account.connected_at < 0
    {
        return Err("Stored Mail data is invalid. It was not changed.".into());
    }
    account.email = validate_email(&account.email)?;
    account.label = truncate_chars(account.label.trim().to_owned(), 256);
    if account.label.is_empty() {
        account.label = account.email.clone();
    }
    account.agent_ids = normalize_agent_ids(std::mem::take(&mut account.agent_ids))?;
    Ok(())
}
fn validate_client_id(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() || value.len() > 512 || value.chars().any(char::is_control) {
        Err("Gmail OAuth client ID is invalid.".into())
    } else {
        Ok(value.into())
    }
}
fn validate_secret(value: String) -> Result<String, String> {
    if value.is_empty() || value.len() > 4096 || value.chars().any(char::is_control) {
        Err("Gmail OAuth client secret is invalid.".into())
    } else {
        Ok(value)
    }
}
fn validate_email(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.len() > 320 || !value.contains('@') || value.chars().any(char::is_control) {
        Err("Gmail returned an invalid account email.".into())
    } else {
        Ok(value.into())
    }
}
fn validate_provider_id(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 256
        || value == "."
        || value == ".."
        || value.contains('/')
        || value.contains('\\')
        || value.chars().any(char::is_control)
    {
        Err("Provider message ID is invalid.".into())
    } else {
        Ok(())
    }
}
fn normalize_agent_ids(ids: Vec<String>) -> Result<Vec<String>, String> {
    if ids.len() > MAX_AGENT_IDS {
        return Err("Too many agent grants.".into());
    }
    let mut seen = HashSet::new();
    let mut clean = Vec::new();
    for id in ids {
        validate_agent_id(&id)?;
        if seen.insert(id.clone()) {
            clean.push(id);
        }
    }
    clean.sort();
    Ok(clean)
}
fn validate_agent_id(value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
        Err("Agent ID is invalid.".into())
    } else {
        Ok(())
    }
}
fn require_empty(args: &Value) -> Result<(), String> {
    if args.as_object().is_some_and(Map::is_empty) {
        Ok(())
    } else {
        Err("mail_accounts does not accept arguments.".into())
    }
}
fn object_args(args: Value, allowed: &[&str]) -> Result<Map<String, Value>, String> {
    let object = args
        .as_object()
        .cloned()
        .ok_or("Mail tool arguments must be an object.")?;
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err("Mail tool arguments contain an unsupported field.".into());
    }
    Ok(object)
}
fn required_string(object: &Map<String, Value>, key: &str, max: usize) -> Result<String, String> {
    object
        .get(key)
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty() && v.len() <= max && !v.chars().any(char::is_control))
        .map(str::to_owned)
        .ok_or_else(|| format!("{key} is required and invalid."))
}
fn optional_string(
    object: &Map<String, Value>,
    key: &str,
    max: usize,
) -> Result<Option<String>, String> {
    match object.get(key) {
        None => Ok(None),
        Some(Value::String(v)) if v.len() <= max && !v.chars().any(char::is_control) => {
            Ok(Some(v.clone()))
        }
        _ => Err(format!("{key} is invalid.")),
    }
}
fn optional_usize(
    object: &Map<String, Value>,
    key: &str,
    default: usize,
    min: usize,
    max: usize,
) -> Result<usize, String> {
    match object.get(key) {
        None => Ok(default),
        Some(v) => v
            .as_u64()
            .and_then(|v| usize::try_from(v).ok())
            .filter(|v| *v >= min && *v <= max)
            .ok_or_else(|| format!("{key} is out of bounds.")),
    }
}
fn optional_bool(object: &Map<String, Value>, key: &str) -> Result<Option<bool>, String> {
    match object.get(key) {
        None => Ok(None),
        Some(Value::Bool(v)) => Ok(Some(*v)),
        _ => Err(format!("{key} is invalid.")),
    }
}
fn optional_string_array(
    object: &Map<String, Value>,
    key: &str,
    min: usize,
    max_len: usize,
) -> Result<Vec<String>, String> {
    match object.get(key) {
        None => Ok(Vec::new()),
        Some(v) => parse_string_array(v, min, max_len, 128, key),
    }
}
fn required_string_array(
    object: &Map<String, Value>,
    key: &str,
    min: usize,
    max_len: usize,
    item_max: usize,
) -> Result<Vec<String>, String> {
    object
        .get(key)
        .ok_or_else(|| format!("{key} is required."))
        .and_then(|v| parse_string_array(v, min, max_len, item_max, key))
}
fn parse_string_array(
    value: &Value,
    min: usize,
    max_len: usize,
    item_max: usize,
    key: &str,
) -> Result<Vec<String>, String> {
    let values = value
        .as_array()
        .ok_or_else(|| format!("{key} is invalid."))?;
    if values.len() < min || values.len() > max_len {
        return Err(format!("{key} is out of bounds."));
    }
    let mut seen = HashSet::new();
    let mut output = Vec::new();
    for value in values {
        let value = value
            .as_str()
            .filter(|v| !v.is_empty() && v.len() <= item_max && !v.chars().any(char::is_control))
            .ok_or_else(|| format!("{key} is invalid."))?;
        if seen.insert(value.to_owned()) {
            output.push(value.to_owned());
        }
    }
    Ok(output)
}
fn unavailable() -> String {
    "Mail service is temporarily unavailable.".into()
}
fn conflict() -> String {
    "Mail settings changed in another Settings pane. Refresh before saving; your draft was not overwritten.".into()
}
fn now() -> i64 {
    crate::model::now()
}
fn set_attempt_error(attempt: &ConnectAttempt, error: &str) {
    if let Ok(mut state) = attempt.state.lock() {
        if matches!(*state, AttemptState::Pending) {
            *state = AttemptState::Error(error.into());
        }
    }
}
fn set_attempt_connected(attempt: &ConnectAttempt, account: MailAccount) {
    if let Ok(mut state) = attempt.state.lock() {
        if matches!(*state, AttemptState::Pending) {
            *state = AttemptState::Connected(account);
        }
    }
}
fn curl_value(value: &str) -> String {
    format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
    )
}
fn random_url_token(bytes: usize) -> Result<String, String> {
    let mut value = vec![0; bytes];
    File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut value))
        .map_err(|_| "Could not securely start Gmail authorization.".to_string())?;
    Ok(base64url(&value))
}
fn base64url(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in input.chunks(3) {
        let a = chunk[0];
        let b = *chunk.get(1).unwrap_or(&0);
        let c = *chunk.get(2).unwrap_or(&0);
        out.push(TABLE[(a >> 2) as usize] as char);
        out.push(TABLE[(((a & 3) << 4) | (b >> 4)) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[(((b & 15) << 2) | (c >> 6)) as usize] as char);
        }
        if chunk.len() > 2 {
            out.push(TABLE[(c & 63) as usize] as char);
        }
    }
    out
}
fn base64url_decode(input: &str) -> Result<Vec<u8>, ()> {
    fn v(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'-' => Some(62),
            b'_' => Some(63),
            _ => None,
        }
    }
    let data = input.trim_end_matches('=');
    let mut out = Vec::new();
    let bytes = data.as_bytes();
    if bytes.len() % 4 == 1 {
        return Err(());
    }
    for chunk in bytes.chunks(4) {
        let a = v(chunk[0]).ok_or(())?;
        let b = v(*chunk.get(1).ok_or(())?).ok_or(())?;
        out.push((a << 2) | (b >> 4));
        if let Some(&c) = chunk.get(2) {
            let c = v(c).ok_or(())?;
            out.push((b << 4) | (c >> 2));
            if let Some(&d) = chunk.get(3) {
                out.push((c << 6) | v(d).ok_or(())?);
            }
        }
    }
    Ok(out)
}
fn url_encode(value: &str) -> String {
    value
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}
fn url_decode(value: &str) -> Option<String> {
    let mut out = Vec::new();
    let bytes = value.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3
        } else if bytes[i] == b'+' {
            out.push(b' ');
            i += 1
        } else {
            out.push(bytes[i]);
            i += 1
        }
    }
    String::from_utf8(out).ok()
}
fn truncate_chars(value: String, max: usize) -> String {
    value.chars().take(max).collect()
}
fn html_to_text(value: &str) -> String {
    static HIDDEN: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let value = HIDDEN
        .get_or_init(|| {
            regex::Regex::new(r"(?is)<(?:script|style|head)\b[^>]*>.*?</(?:script|style|head)\s*>")
                .unwrap()
        })
        .replace_all(value, " ");
    let mut out = String::new();
    let mut tag = false;
    for c in value.chars() {
        match c {
            '<' => tag = true,
            '>' => {
                tag = false;
                out.push(' ')
            }
            _ if !tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(all(target_os = "macos", not(test)))]
fn load_vault() -> Result<MailVault, String> {
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT)
        .map_err(|_| "Cannot access the macOS Keychain for Mail.".to_string())?;
    let raw = match entry.get_password() {
        Ok(raw) => raw,
        Err(keyring::Error::NoEntry) => return Ok(MailVault::default()),
        Err(_) => return Err("Cannot access the macOS Keychain for Mail.".into()),
    };
    if raw.len() > MAX_PAYLOAD_BYTES {
        return Err("Stored Mail data is invalid. It was not changed.".into());
    }
    let mut vault = serde_json::from_str::<MailVault>(&raw)
        .map_err(|_| "Stored Mail data is invalid. It was not changed.".to_string())?;
    normalize_vault(&mut vault)?;
    Ok(vault)
}
#[cfg(all(target_os = "macos", not(test)))]
fn save_vault(vault: &MailVault) -> Result<(), String> {
    let raw = serde_json::to_string(vault)
        .map_err(|_| "Cannot prepare Mail settings for storage.".to_string())?;
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT)
        .map_err(|_| "Cannot access the macOS Keychain for Mail.".to_string())?;
    entry
        .set_password(&raw)
        .map_err(|_| "Cannot save Mail settings to the macOS Keychain.".to_string())
}
#[cfg(not(target_os = "macos"))]
fn load_vault() -> Result<MailVault, String> {
    Err(UNSUPPORTED.into())
}
#[cfg(not(target_os = "macos"))]
fn save_vault(_: &MailVault) -> Result<(), String> {
    Err(UNSUPPORTED.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_unknown_and_invalid_tool_arguments() {
        assert!(object_args(json!({"account_id":"a","surprise":true}), &["account_id"]).is_err());
        assert!(parse_string_array(&json!([]), 1, 20, 20, "message_ids").is_err());
        assert!(optional_usize(
            &serde_json::from_value(json!({"limit":51})).unwrap(),
            "limit",
            20,
            1,
            50
        )
        .is_err());
    }
    #[test]
    fn parser_prefers_plain_text_and_keeps_attachment_metadata() {
        let message:GmailMessage=serde_json::from_value(json!({"id":"m","threadId":"t","labelIds":["INBOX"],"snippet":"s","internalDate":"1","payload":{"headers":[{"name":"From","value":"a@example.com"}],"parts":[{"mimeType":"text/plain","body":{"data":"aGVsbG8"}},{"mimeType":"application/pdf","filename":"x.pdf","body":{"attachmentId":"a","size":2}}]}})).unwrap();
        let normalized = normalize_message(message, 100);
        assert_eq!(normalized["body"], "hello");
        assert_eq!(normalized["attachments"][0]["filename"], "x.pdf");
    }
    #[test]
    fn base64url_and_query_helpers_are_bounded() {
        assert_eq!(base64url_decode("aGVsbG8").unwrap(), b"hello");
        assert_eq!(
            parse_query("state=x%20y&code=z").unwrap().get("state"),
            Some(&"x y".to_string())
        );
        assert_eq!(html_to_text("<p>hello <b>world</b></p>"), "hello world");
    }
    #[test]
    fn vault_roundtrip_and_oauth_snake_case_are_supported() {
        let vault = MailVault {
            gmail_client_id: "client".into(),
            gmail_client_secret: Some("secret".into()),
            accounts: vec![VaultAccount {
                public: MailAccount {
                    id: Uuid::new_v4().to_string(),
                    provider: "gmail".into(),
                    email: "reader@example.com".into(),
                    label: "Reader".into(),
                    agent_ids: vec!["agent".into()],
                    connected_at: 1,
                },
                refresh_token: "refresh".into(),
                modify_scope: false,
            }],
        };
        let restored: MailVault =
            serde_json::from_slice(&serde_json::to_vec(&vault).unwrap()).unwrap();
        assert_eq!(restored.accounts[0].refresh_token, "refresh");
        let token: GmailToken = serde_json::from_value(
            json!({"access_token":"access","refresh_token":"refresh","expires_in":3600}),
        )
        .unwrap();
        assert_eq!(token.access_token, "access");
    }
    #[test]
    fn gmail_query_preserves_operators_and_separate_label_ids() {
        let request = SearchRequest {
            query: Some("in:inbox from:(a@example.com) newer_than:7d".into()),
            limit: 20,
            page_token: Some("opaque+/=".into()),
            label_ids: vec!["INBOX".into(), "Label_1".into()],
            include_spam_trash: true,
        };
        let query = gmail_search_query(&request);
        assert!(query.contains("q=in%3Ainbox%20from%3A%28a%40example.com%29%20newer_than%3A7d"));
        assert!(query.contains("pageToken=opaque%2B%2F%3D"));
        assert!(query.contains("labelIds=INBOX&labelIds=Label_1"));
        assert_eq!(
            optional_string_array(
                &json!({"label_ids":["INBOX"]}).as_object().unwrap(),
                "label_ids",
                0,
                20
            )
            .unwrap(),
            vec!["INBOX"]
        );
        assert!(optional_string_array(
            &json!({"label_ids":vec!["x";21]}).as_object().unwrap(),
            "label_ids",
            0,
            20
        )
        .is_err());
    }
    #[test]
    fn account_access_is_explicit_and_revocation_updates_live_checks() {
        let service = MailService::for_test();
        let id = Uuid::new_v4().to_string();
        service
            .seed_test_account(
                MailAccount {
                    id: id.clone(),
                    provider: "gmail".into(),
                    email: "r@example.com".into(),
                    label: "R".into(),
                    agent_ids: vec!["agent".into()],
                    connected_at: 1,
                },
                "private-token".into(),
            )
            .unwrap();
        let listed = service
            .call_for_agent("agent", "mail_accounts", json!({}))
            .unwrap()
            .to_string();
        assert!(listed.contains("r@example.com"));
        assert!(!listed.contains("private-token"));
        assert!(!listed.contains("agentIds"));
        assert!(service.authorized_account("other", &id).is_err());
        assert!(service.has_agent_access("agent").unwrap());
        let config = service.config().unwrap();
        assert!(!serde_json::to_string(&config)
            .unwrap()
            .contains("private-token"));
        service
            .set_account_agents(config.revision.clone(), id.clone(), vec![])
            .unwrap();
        assert!(!service.has_agent_access("agent").unwrap());
        assert!(service.recheck_grant("agent", &id).is_err());
        assert!(service
            .set_account_agents(config.revision, id.clone(), vec!["agent".into()])
            .is_err());
        let config = service.config().unwrap();
        service.disconnect(config.revision, id.clone()).unwrap();
        assert!(service.config().unwrap().accounts.is_empty());
    }
    #[test]
    fn oauth_rejects_ambiguous_callbacks_and_cancels_without_credentials() {
        assert!(parse_query("state=a&state=b&code=c").is_err());
        let service = Arc::new(MailService::for_test());
        let config = service.config().unwrap();
        service
            .configure_gmail(
                config.revision,
                "test.apps.googleusercontent.com".into(),
                None,
            )
            .unwrap();
        let connect = service.start_connect().unwrap();
        let params = parse_query(connect.authorization_url.split_once('?').unwrap().1).unwrap();
        assert_eq!(
            params["scope"],
            "https://www.googleapis.com/auth/gmail.modify"
        );
        assert_eq!(params["code_challenge_method"], "S256");
        let address = params["redirect_uri"].trim_start_matches("http://");
        let mut socket = TcpStream::connect(address).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        socket
            .write_all(b"GET /?state=wrong&code=not-used HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .unwrap();
        let mut response = String::new();
        socket.read_to_string(&mut response).unwrap();
        assert_eq!(
            service.connect_status(&connect.attempt_id).unwrap().status,
            "pending"
        );
        service.cancel_connect(&connect.attempt_id).unwrap();
        assert_eq!(
            service.connect_status(&connect.attempt_id).unwrap().status,
            "cancelled"
        );
        assert!(service.config().unwrap().accounts.is_empty());
    }
    #[test]
    fn unavailable_external_bodies_and_html_are_explicit() {
        let message: GmailMessage=serde_json::from_value(json!({"id":"m","threadId":"t","payload":{"mimeType":"text/plain","body":{"attachmentId":"external","size":100}}})).unwrap();
        assert_eq!(normalize_message(message, 20)["bodyUnavailable"], true);
        assert_eq!(
            html_to_text("<style>.hidden{}</style><p>A &amp; B</p><script>doBadThings()</script>"),
            "A & B"
        );
        let message: GmailMessage=serde_json::from_value(json!({"id":"m","threadId":"t","payload":{"mimeType":"text/plain","body":{"data":"YWJjZA"}}})).unwrap();
        let result = normalize_message(message, 3);
        assert_eq!(result["body"], "abc");
        assert_eq!(result["bodyTruncated"], true);
    }
    #[test]
    fn exact_body_truncation_and_named_attachment_exclusion() {
        let message: GmailMessage = serde_json::from_value(json!({"id":"m","threadId":"t","payload":{"parts":[{"mimeType":"text/plain","body":{"data":"YWJj"}},{"mimeType":"text/plain","filename":"hidden.txt","body":{"data":"c2VjcmV0","size":6}}]}})).unwrap();
        let normalized = normalize_message(message, 3);
        assert_eq!(normalized["body"], "abc");
        assert_eq!(normalized["bodyTruncated"], false);
        assert_eq!(normalized["attachments"][0]["filename"], "hidden.txt");
    }
}
