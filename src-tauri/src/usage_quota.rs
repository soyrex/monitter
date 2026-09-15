//! Read-only, short-lived subscription allowance probes.
//!
//! These probes deliberately do not share the resident task transports: they
//! must never start a thread, submit a prompt, or alter provider auth state.

use crate::{
    model::{now, AllowanceBalance, AllowanceWindow, Host, SubscriptionUsageSource},
    runner::{resolve_local, resolve_local_provider},
};
use jiff::{Timestamp, Zoned};
use regex::Regex;
use serde_json::{json, Value};
use std::{
    ffi::{OsStr, OsString},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{mpsc, OnceLock},
    thread,
    time::{Duration, Instant},
};

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
// The verified local command itself is quick, but a cold Claude CLI can take
// longer than the other quota clients to initialise its Node runtime.
const CLAUDE_PROBE_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_OUTPUT_BYTES: usize = 128 * 1024;
const STALE_AFTER_MS: i64 = 60_000;

/// Refresh the subscription allowance that can be safely read on this local
/// host.  The returned source intentionally carries failures rather than
/// returning raw CLI/protocol errors, as those errors can contain account data.
///
/// Supported provider names are `codex`, `claude`, `minimax` (with `mmx`
/// accepted as an alias), and `opencode-go`.
pub(crate) fn refresh_provider_quota(host: &Host, provider: &str) -> SubscriptionUsageSource {
    let attempted_at = now();
    let provider = provider.trim().to_ascii_lowercase();

    if host.kind != "local" {
        return source(
            canonical_provider(&provider),
            host,
            "local subscription allowance probe",
            "not-applicable",
            attempted_at,
            None,
            vec![],
            vec![],
            None,
        );
    }

    match provider.as_str() {
        "codex" => refresh_codex(host, attempted_at, None),
        "claude" => refresh_claude(host, attempted_at),
        "minimax" | "mmx" => refresh_minimax(host, attempted_at),
        "opencode-go" => refresh_opencode_go(host, attempted_at),
        _ => source(
            canonical_provider(&provider),
            host,
            "on-demand allowance refresh",
            "unsupported",
            attempted_at,
            None,
            vec![],
            vec![],
            None,
        ),
    }
}

/// Claude's local `/usage` command is a zero-token CLI command. It is kept as
/// a separate, short-lived argv invocation so it cannot affect a resident
/// Claude chat or its authentication/configuration.
fn refresh_claude(host: &Host, attempted_at: i64) -> SubscriptionUsageSource {
    let executable = match resolve_local_provider("claude", &host.claude_path) {
        Ok(path) => path,
        Err(_) => {
            return probe_error(
                "claude",
                host,
                "claude /usage",
                attempted_at,
                "Claude quota probe is unavailable on this host.",
            )
        }
    };
    let mut command = Command::new(executable);
    command.args(claude_usage_args());
    configure(&mut command);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(_) => {
            return probe_error(
                "claude",
                host,
                "claude /usage",
                attempted_at,
                "Could not start the Claude quota probe.",
            )
        }
    };
    let result = read_all_bounded_with_timeout(&mut child, CLAUDE_PROBE_TIMEOUT)
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).map_err(|_| ()))
        .and_then(|value| normalize_claude_usage(value, attempted_at));
    stop_child(&mut child);
    match result {
        Ok(windows) => success(
            "claude",
            host,
            "claude -p /usage --output-format json",
            attempted_at,
            None,
            windows,
            vec![],
        ),
        Err(_) => probe_error(
            "claude",
            host,
            "claude -p /usage --output-format json",
            attempted_at,
            "Claude quota probe did not return supported account usage.",
        ),
    }
}

fn refresh_opencode_go(host: &Host, attempted_at: i64) -> SubscriptionUsageSource {
    let executable = match resolve_router_control() {
        Ok(path) => path,
        Err(_) => {
            return probe_error(
                "opencode-go",
                host,
                "codex-router provider-quota",
                attempted_at,
                "OpenCode Go quota requires the local Codex Router control command.",
            )
        }
    };
    let mut command = Command::new(executable);
    command.args(opencode_go_args());
    configure(&mut command);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(_) => {
            return probe_error(
                "opencode-go",
                host,
                "codex-router provider-quota",
                attempted_at,
                "Could not start the OpenCode Go quota probe.",
            )
        }
    };
    let result = read_all_bounded(&mut child)
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).map_err(|_| ()))
        .and_then(normalize_opencode_go);
    stop_child(&mut child);
    match result {
        Ok(windows) => success(
            "opencode-go",
            host,
            "codex-router provider-quota opencode-go",
            attempted_at,
            Some("OpenCode Go".into()),
            windows,
            vec![],
        ),
        Err(_) => probe_error(
            "opencode-go",
            host,
            "codex-router provider-quota opencode-go",
            attempted_at,
            "OpenCode Go quota is unavailable. Check the router credential and subscription.",
        ),
    }
}

/// Probe one account without changing the parent process or reading credentials.
/// Keep identity on failures too, so one account cannot replace another's meter.
pub(crate) fn refresh_codex_quota(host: &Host, home: &str, label: &str) -> SubscriptionUsageSource {
    let mut result = if host.kind == "local" {
        refresh_codex(host, now(), Some(home))
    } else {
        refresh_provider_quota(host, "codex")
    };
    result.codex_home = Some(home.into());
    result.account_label = Some(label.into());
    result
}

fn refresh_codex(host: &Host, attempted_at: i64, home: Option<&str>) -> SubscriptionUsageSource {
    let executable = match resolve_local(&host.codex_path) {
        Ok(path) => path,
        Err(_) => {
            return probe_error(
                "codex",
                host,
                "codex app-server",
                attempted_at,
                "Codex quota probe is unavailable on this host.",
            )
        }
    };
    let mut command = Command::new(executable);
    command.arg("app-server");
    if crate::codex_accounts::configure_command(&mut command, home).is_err() {
        return probe_error("codex", host, "codex app-server", attempted_at,
            "Codex account home is unavailable. Check the selected account folder.");
    }
    configure(&mut command);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(_) => {
            return probe_error(
                "codex",
                host,
                "codex app-server",
                attempted_at,
                "Could not start the Codex quota probe.",
            )
        }
    };
    let result = read_codex_rate_limits(&mut child).and_then(normalize_codex);
    stop_child(&mut child);
    match result {
        Ok((plan_type, windows, balances)) => success(
            "codex",
            host,
            "codex app-server account/rateLimits/read",
            attempted_at,
            plan_type,
            windows,
            balances,
        ),
        Err(_) => probe_error(
            "codex",
            host,
            "codex app-server account/rateLimits/read",
            attempted_at,
            "Codex quota probe did not return a supported response.",
        ),
    }
}

fn refresh_minimax(host: &Host, attempted_at: i64) -> SubscriptionUsageSource {
    let executable = match resolve_minimax() {
        Ok(path) => path,
        Err(_) => {
            return probe_error(
                "minimax",
                host,
                "mmx quota show",
                attempted_at,
                "MiniMax quota probe is unavailable on this host.",
            )
        }
    };
    let mut command = minimax_command(&executable);
    configure(&mut command);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(_) => {
            return probe_error(
                "minimax",
                host,
                "mmx quota show",
                attempted_at,
                "MiniMax quota probe is unavailable on this host.",
            )
        }
    };
    let result = read_all_bounded(&mut child)
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).map_err(|_| ()))
        .and_then(normalize_minimax);
    stop_child(&mut child);
    match result {
        Ok(windows) => success(
            "minimax",
            host,
            "mmx quota show --output json --non-interactive",
            attempted_at,
            Some("Token Plan".into()),
            windows,
            vec![],
        ),
        Err(_) => probe_error(
            "minimax",
            host,
            "mmx quota show --output json --non-interactive",
            attempted_at,
            "MiniMax quota probe did not return a supported response. Check the local sign-in and CLI.",
        ),
    }
}

/// Resolve fixed, local install locations explicitly: Finder-launched apps do
/// not reliably inherit an interactive shell PATH. `resolve_local` supplies
/// the project's existing `is_file` validation and tilde handling.
fn resolve_minimax() -> Result<PathBuf, ()> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut candidates = vec![
        PathBuf::from("/opt/homebrew/bin/mmx"),
        PathBuf::from("/usr/local/bin/mmx"),
        PathBuf::from("/usr/bin/mmx"),
    ];
    if let Some(home) = home {
        candidates.splice(
            0..0,
            [
                home.join(".local/bin/mmx"),
                home.join(".npm-global/bin/mmx"),
            ],
        );
    }
    candidates
        .into_iter()
        .find_map(|candidate| resolve_local(&candidate.to_string_lossy()).ok())
        .ok_or(())
}

fn resolve_router_control() -> Result<PathBuf, ()> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut candidates = vec![
        PathBuf::from("/opt/homebrew/bin/codex-router-control"),
        PathBuf::from("/usr/local/bin/codex-router-control"),
    ];
    if let Some(home) = home {
        candidates.splice(0..0, [home.join(".local/share/codex-router/bin/control")]);
    }
    candidates
        .into_iter()
        .find_map(|candidate| resolve_local(&candidate.to_string_lossy()).ok())
        .ok_or(())
}

/// This remains an argv invocation: it never involves a shell.
fn minimax_command(executable: &Path) -> Command {
    let mut command = Command::new(executable);
    command.args(minimax_args());
    command
}

fn minimax_args() -> [&'static str; 5] {
    ["quota", "show", "--output", "json", "--non-interactive"]
}

/// Literal Claude CLI argv. `/usage` is a local command, not a model prompt.
fn claude_usage_args() -> [&'static str; 4] {
    ["-p", "/usage", "--output-format", "json"]
}

fn opencode_go_args() -> [&'static str; 3] {
    ["provider-quota", "opencode-go", "--json"]
}

fn configure(command: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command
        .env("PATH", quota_probe_path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
}

/// Finder-launched applications normally receive only the system search path.
/// The reviewed quota clients are scripts whose shebangs launch `node`, so the
/// child also needs the standard local package-manager directories even though
/// Monitter resolves the quota client itself to an absolute path.
fn quota_probe_path() -> OsString {
    build_quota_probe_path(
        std::env::var_os("HOME").as_deref().map(Path::new),
        std::env::var_os("PATH").as_deref(),
    )
}

fn build_quota_probe_path(home: Option<&Path>, existing: Option<&OsStr>) -> OsString {
    let mut paths = Vec::new();
    if let Some(home) = home {
        paths.push(home.join(".local/bin"));
        paths.push(home.join(".npm-global/bin"));
    }
    paths.extend([
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/usr/bin"),
        PathBuf::from("/bin"),
        PathBuf::from("/usr/sbin"),
        PathBuf::from("/sbin"),
    ]);
    if let Some(existing) = existing {
        paths.extend(std::env::split_paths(existing).filter(|path| path.is_absolute()));
    }
    std::env::join_paths(paths).unwrap_or_else(|_| OsString::from("/usr/bin:/bin"))
}

fn read_codex_rate_limits(child: &mut Child) -> Result<Value, ()> {
    let mut stdin = child.stdin.take().ok_or(())?;
    let stdout = child.stdout.take().ok_or(())?;
    let lines = spawn_json_reader(stdout);
    let deadline = Instant::now() + PROBE_TIMEOUT;

    write_request(
        &mut stdin,
        json!({
            "id": 1,
            "method": "initialize",
            "params": { "clientInfo": { "name": "Monitter", "version": "0.1" } }
        }),
    )?;
    response(&lines, 1, deadline)?;
    write_request(&mut stdin, json!({ "method": "initialized" }))?;
    write_request(
        &mut stdin,
        json!({
            "id": 2,
            "method": "account/rateLimits/read",
            "params": {}
        }),
    )?;
    response(&lines, 2, deadline)
}

fn write_request(stdin: &mut impl Write, value: Value) -> Result<(), ()> {
    serde_json::to_writer(&mut *stdin, &value).map_err(|_| ())?;
    stdin
        .write_all(b"\n")
        .and_then(|_| stdin.flush())
        .map_err(|_| ())
}

fn response(
    lines: &mpsc::Receiver<Result<Value, ()>>,
    id: i64,
    deadline: Instant,
) -> Result<Value, ()> {
    loop {
        let remaining = deadline.checked_duration_since(Instant::now()).ok_or(())?;
        let value = lines.recv_timeout(remaining).map_err(|_| ())??;
        if value.get("id").and_then(Value::as_i64) == Some(id) {
            // Do not pass provider error text through: it can include account
            // identifiers or other locally configured details.
            return if value.get("error").is_some() {
                Err(())
            } else {
                Ok(value)
            };
        }
    }
}

fn spawn_json_reader(stdout: impl Read + Send + 'static) -> mpsc::Receiver<Result<Value, ()>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut reader = stdout;
        let mut chunk = [0_u8; 4096];
        let mut line = Vec::new();
        let mut total = 0_usize;
        loop {
            let count = match reader.read(&mut chunk) {
                Ok(count) => count,
                Err(_) => {
                    let _ = tx.send(Err(()));
                    return;
                }
            };
            if count == 0 {
                return;
            }
            total = match total.checked_add(count) {
                Some(total) if total <= MAX_OUTPUT_BYTES => total,
                _ => {
                    let _ = tx.send(Err(()));
                    return;
                }
            };
            for byte in &chunk[..count] {
                if *byte == b'\n' {
                    let value = serde_json::from_slice::<Value>(&line).map_err(|_| ());
                    line.clear();
                    if tx.send(value).is_err() {
                        return;
                    }
                } else {
                    line.push(*byte);
                    if line.len() > MAX_OUTPUT_BYTES {
                        let _ = tx.send(Err(()));
                        return;
                    }
                }
            }
        }
    });
    rx
}

fn read_all_bounded(child: &mut Child) -> Result<Vec<u8>, ()> {
    read_all_bounded_with_timeout(child, PROBE_TIMEOUT)
}

fn read_all_bounded_with_timeout(child: &mut Child, timeout: Duration) -> Result<Vec<u8>, ()> {
    let stdout = child.stdout.take().ok_or(())?;
    let (tx, rx) = mpsc::channel();
    let deadline = Instant::now() + timeout;
    thread::spawn(move || {
        let mut reader = stdout;
        let mut bytes = Vec::new();
        let mut chunk = [0_u8; 4096];
        loop {
            let count = match reader.read(&mut chunk) {
                Ok(count) => count,
                Err(_) => {
                    let _ = tx.send(Err(()));
                    return;
                }
            };
            if count == 0 {
                let _ = tx.send(Ok(bytes));
                return;
            }
            if bytes
                .len()
                .checked_add(count)
                .is_none_or(|length| length > MAX_OUTPUT_BYTES)
            {
                let _ = tx.send(Err(()));
                return;
            }
            bytes.extend_from_slice(&chunk[..count]);
        }
    });
    let remaining = deadline.checked_duration_since(Instant::now()).ok_or(())?;
    let bytes = rx.recv_timeout(remaining).map_err(|_| ())??;
    loop {
        match child.try_wait().map_err(|_| ())? {
            Some(status) if status.success() => return Ok(bytes),
            Some(_) => return Err(()),
            None if Instant::now() >= deadline => return Err(()),
            None => thread::sleep(Duration::from_millis(10)),
        }
    }
}

static CLAUDE_SESSION_LINE: OnceLock<Regex> = OnceLock::new();
static CLAUDE_WEEK_LINE: OnceLock<Regex> = OnceLock::new();

/// Decode Claude's one-object JSON response and retain only the two
/// account-level allowance lines. Raw output is never returned or logged.
fn normalize_claude_usage(value: Value, attempted_at: i64) -> Result<Vec<AllowanceWindow>, ()> {
    let object = value.as_object().ok_or(())?;
    if object.get("local_command").and_then(Value::as_str) != Some("usage")
        || object.get("is_error").and_then(Value::as_bool) != Some(false)
    {
        return Err(());
    }
    let result = object
        .get("result")
        .and_then(Value::as_str)
        .filter(|result| !result.trim().is_empty())
        .ok_or(())?;
    let session = claude_line_regex(&CLAUDE_SESSION_LINE, "Current\\s+session");
    let week = claude_line_regex(
        &CLAUDE_WEEK_LINE,
        "Current\\s+week\\s*\\(\\s*all\\s+models\\s*\\)",
    );
    Ok(vec![
        claude_window("session", "Current session", session, result, attempted_at)?,
        claude_window(
            "week_all_models",
            "Current week (all models)",
            week,
            result,
            attempted_at,
        )?,
    ])
}

/// Exactly two compiled, multiline expressions track the provider's two
/// labelled allowance lines while tolerating markdown, spacing and separators.
fn claude_line_regex(slot: &'static OnceLock<Regex>, heading: &str) -> &'static Regex {
    slot.get_or_init(|| {
        Regex::new(&format!(
            r"(?im)^\s*(?:[-*#>]+\s*)?{heading}\s*(?:[:*_-]+\s*)*(?<percent>\d{{1,3}}(?:\.\d+)?)\s*%\s*used\b.*?\bresets?\s+(?<month>[A-Za-z]{{3,9}})\s+(?<day>\d{{1,2}})(?:\s*,?\s*(?<year>\d{{4}}))?\s+at\s+(?<hour>\d{{1,2}})(?::(?<minute>\d{{2}}))?\s*(?<meridiem>am|pm)\s*\(\s*(?<zone>[A-Za-z0-9_+\-/]+)\s*\)\s*$"
        ))
        .expect("constant Claude usage regex is valid")
    })
}

fn claude_window(
    key: &str,
    label: &str,
    regex: &Regex,
    result: &str,
    attempted_at: i64,
) -> Result<AllowanceWindow, ()> {
    let captures = regex.captures(result).ok_or(())?;
    let used_percent = captures
        .name("percent")
        .ok_or(())?
        .as_str()
        .parse::<f64>()
        .map_err(|_| ())?;
    if !(0.0..=100.0).contains(&used_percent) {
        return Err(());
    }
    let zone = captures.name("zone").ok_or(())?.as_str();
    let initial_year = match captures.name("year") {
        Some(year) => year.as_str().parse::<i16>().map_err(|_| ())?,
        None => Timestamp::from_millisecond(attempted_at)
            .and_then(|timestamp| timestamp.in_tz(zone))
            .map(|zoned| zoned.year())
            .map_err(|_| ())?,
    };
    let timestamp = claude_reset_timestamp(&captures, initial_year)?;
    let resets_at = if captures.name("year").is_none() && timestamp < attempted_at {
        claude_reset_timestamp(&captures, initial_year.checked_add(1).ok_or(())?)?
    } else {
        timestamp
    };
    Ok(AllowanceWindow {
        key: key.into(),
        label: label.into(),
        metric: "combined".into(),
        used_percent: Some(used_percent),
        used: None,
        limit: None,
        unit: "unknown".into(),
        resets_at: Some(resets_at),
    })
}

fn claude_reset_timestamp(captures: &regex::Captures<'_>, year: i16) -> Result<i64, ()> {
    let month = captures.name("month").ok_or(())?.as_str();
    let day = captures.name("day").ok_or(())?.as_str();
    let hour = captures.name("hour").ok_or(())?.as_str();
    let minute = captures
        .name("minute")
        .map_or("00", |minute| minute.as_str());
    let meridiem = captures.name("meridiem").ok_or(())?.as_str();
    let zone = captures.name("zone").ok_or(())?.as_str();
    Zoned::strptime(
        "%Y %b %e at %I:%M%p %Q",
        format!("{year} {month} {day} at {hour}:{minute}{meridiem} {zone}"),
    )
    .map(|zoned| zoned.timestamp().as_millisecond())
    .map_err(|_| ())
}

fn normalize_codex(
    value: Value,
) -> Result<(Option<String>, Vec<AllowanceWindow>, Vec<AllowanceBalance>), ()> {
    let limits = value
        .pointer("/result/rateLimits")
        .and_then(Value::as_object)
        .ok_or(())?;
    let plan_type = limits
        .get("planType")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let mut windows = Vec::new();
    append_codex_windows(&mut windows, limits, None, None);
    if let Some(additional) = limits.get("additionalRateLimits").and_then(Value::as_array) {
        for (index, item) in additional.iter().enumerate() {
            let Some(item) = item.as_object() else {
                continue;
            };
            // Never use a limit ID as a display key. It can be account-scoped.
            // A provider-supplied model/limit name keeps distinct buckets apart.
            let label = item
                .get("limitName")
                .or_else(|| item.get("normalModelSlug"))
                .and_then(Value::as_str)
                .map(safe_display)
                .filter(|label| !label.is_empty())
                .unwrap_or_else(|| format!("Additional allowance {}", index + 1));
            append_codex_windows(&mut windows, item, Some(index + 1), Some(&label));
        }
    }
    let mut balances = Vec::new();
    if let Some(credits) = limits.get("credits").and_then(Value::as_object) {
        if credits.get("unlimited").and_then(Value::as_bool) == Some(true) {
            balances.push(AllowanceBalance {
                key: "credits".into(),
                label: "Credits (unlimited)".into(),
                unit: "unknown".into(),
                remaining: None,
                limit: None,
                resets_at: None,
            });
        } else if credits.get("hasCredits").and_then(Value::as_bool) == Some(true) {
            balances.push(AllowanceBalance {
                key: "credits".into(),
                label: "Credits".into(),
                unit: "credits".into(),
                remaining: credits.get("balance").and_then(number),
                limit: None,
                resets_at: None,
            });
        }
    }
    Ok((plan_type, windows, balances))
}

fn append_codex_windows(
    output: &mut Vec<AllowanceWindow>,
    limits: &serde_json::Map<String, Value>,
    additional_index: Option<usize>,
    label_prefix: Option<&str>,
) {
    for (fallback_key, fallback_label) in [("primary", "Primary"), ("secondary", "Secondary")] {
        let Some(window) = limits.get(fallback_key).and_then(Value::as_object) else {
            continue;
        };
        let used_percent = percentage(window.get("usedPercent"));
        let resets_at = seconds_to_millis(window.get("resetsAt"));
        let duration = window.get("windowDurationMins").and_then(Value::as_i64);
        if used_percent.is_none() && resets_at.is_none() && duration.is_none() {
            continue;
        }
        let (base_key, base_label) = codex_window_identity(duration, fallback_key, fallback_label);
        let (key, label) = match (additional_index, label_prefix) {
            (Some(index), Some(prefix)) => (
                format!("additional-{index}-{base_key}"),
                format!("{prefix} {base_label}"),
            ),
            _ => (base_key, base_label),
        };
        output.push(AllowanceWindow {
            key,
            label,
            metric: "combined".into(),
            used_percent,
            used: None,
            limit: None,
            unit: "unknown".into(),
            resets_at,
        });
    }
}

/// Codex does not promise that `primary` is a five-hour bucket or that
/// `secondary` is weekly. Its duration is the stable semantic identifier.
fn codex_window_identity(
    duration_minutes: Option<i64>,
    fallback_key: &str,
    fallback_label: &str,
) -> (String, String) {
    match duration_minutes {
        Some(300) => ("five_hour".into(), "5-hour".into()),
        Some(10_080) => ("weekly".into(), "Week".into()),
        Some(minutes) if minutes > 0 => (
            format!("window-{minutes}m"),
            format!("Window ({minutes} min)"),
        ),
        _ => (fallback_key.into(), fallback_label.into()),
    }
}

fn normalize_minimax(value: Value) -> Result<Vec<AllowanceWindow>, ()> {
    if value
        .pointer("/base_resp/status_code")
        .and_then(Value::as_i64)
        .is_some_and(|status| status != 0)
    {
        return Err(());
    }
    let models = value
        .get("model_remains")
        .and_then(Value::as_array)
        .ok_or(())?;
    let mut windows = Vec::new();
    for model in models {
        let model = model.as_object().ok_or(())?;
        let name = model
            .get("model_name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
            .ok_or(())?;
        let key = stable_key(name);
        append_minimax_window(
            &mut windows,
            &key,
            name,
            "five_hour",
            "5-hour",
            model.get("current_interval_status"),
            model.get("current_interval_remaining_percent"),
            model.get("end_time"),
        );
        append_minimax_window(
            &mut windows,
            &key,
            name,
            "weekly",
            "Weekly",
            model.get("current_weekly_status"),
            model.get("current_weekly_remaining_percent"),
            model.get("weekly_end_time"),
        );
    }
    Ok(windows)
}

fn normalize_opencode_go(value: Value) -> Result<Vec<AllowanceWindow>, ()> {
    if value.get("provider").and_then(Value::as_str) != Some("opencode-go") {
        return Err(());
    }
    let usage = value.get("usage").and_then(Value::as_object).ok_or(())?;
    [
        ("rolling", "5-hour"),
        ("weekly", "Week"),
        ("monthly", "Month"),
    ]
    .into_iter()
    .map(|(key, label)| {
        let window = usage.get(key).and_then(Value::as_object).ok_or(())?;
        let status = window.get("status").and_then(Value::as_str).ok_or(())?;
        if !matches!(status, "ok" | "rate-limited") {
            return Err(());
        }
        Ok(AllowanceWindow {
            key: key.into(),
            label: label.into(),
            metric: "spend".into(),
            used_percent: percentage(window.get("percent")),
            used: None,
            limit: None,
            unit: "usd".into(),
            resets_at: window.get("resetsAtMs").and_then(Value::as_i64),
        })
    })
    .collect()
}

fn append_minimax_window(
    windows: &mut Vec<AllowanceWindow>,
    key: &str,
    model: &str,
    period_key: &str,
    period_label: &str,
    status: Option<&Value>,
    remaining_percent: Option<&Value>,
    resets_at: Option<&Value>,
) {
    // MiniMax status 3 means the allowance is unlimited. Do not represent it
    // as 0% used: that would falsely imply a finite allowance.
    let unlimited = status.and_then(Value::as_i64) == Some(3);
    let used_percent = if unlimited {
        None
    } else {
        remaining_percent
            .and_then(|value| percentage(Some(value)))
            .map(|remaining| 100.0 - remaining)
    };
    let resets_at = minimax_millis(resets_at);
    if !unlimited && used_percent.is_none() && resets_at.is_none() {
        return;
    }
    windows.push(AllowanceWindow {
        key: format!("{key}-{period_key}"),
        label: if unlimited {
            format!("{model} {period_label} (unlimited)")
        } else {
            format!("{model} {period_label}")
        },
        metric: "combined".into(),
        used_percent,
        used: None,
        limit: None,
        unit: "unknown".into(),
        resets_at,
    });
}

fn percentage(value: Option<&Value>) -> Option<f64> {
    value
        .and_then(number)
        .filter(|percent| (0.0..=100.0).contains(percent))
}

fn number(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|value| value.parse::<f64>().ok()))
        .filter(|value| value.is_finite())
}

fn seconds_to_millis(value: Option<&Value>) -> Option<i64> {
    value
        .and_then(Value::as_i64)
        .and_then(|seconds| seconds.checked_mul(1_000))
}

// MiniMax already emits Unix milliseconds. Keeping this separate from Codex's
// seconds conversion prevents a subtle 1,000x reset-time error.
fn minimax_millis(value: Option<&Value>) -> Option<i64> {
    value.and_then(Value::as_i64)
}

fn stable_key(value: &str) -> String {
    let key: String = value
        .to_ascii_lowercase()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '-'
            }
        })
        .collect();
    let key = key.trim_matches('-');
    if key.is_empty() {
        "model".into()
    } else {
        key[..key.len().min(80)].into()
    }
}

fn safe_display(value: &str) -> String {
    value
        .chars()
        .filter(|character| {
            character.is_ascii_alphanumeric() || matches!(character, ' ' | '-' | '_' | '.' | '/')
        })
        .take(80)
        .collect::<String>()
        .trim()
        .into()
}

fn success(
    provider: &str,
    host: &Host,
    source_name: &str,
    attempted_at: i64,
    plan_type: Option<String>,
    windows: Vec<AllowanceWindow>,
    balances: Vec<AllowanceBalance>,
) -> SubscriptionUsageSource {
    source(
        provider,
        host,
        source_name,
        "available",
        attempted_at,
        Some(attempted_at),
        windows,
        balances,
        plan_type,
    )
}

fn probe_error(
    provider: &str,
    host: &Host,
    source_name: &str,
    attempted_at: i64,
    error: &str,
) -> SubscriptionUsageSource {
    source(
        provider,
        host,
        source_name,
        "error",
        attempted_at,
        None,
        vec![],
        vec![],
        None,
    )
    .with_error(error)
}

fn source(
    provider: &str,
    host: &Host,
    source_name: &str,
    state: &str,
    attempted_at: i64,
    fetched_at: Option<i64>,
    windows: Vec<AllowanceWindow>,
    balances: Vec<AllowanceBalance>,
    plan_type: Option<String>,
) -> SubscriptionUsageSource {
    SubscriptionUsageSource {
        provider: provider.into(),
        host_id: host.id.clone(),
        codex_home: None,
        account_label: None,
        source: source_name.into(),
        state: state.into(),
        plan_type,
        fetched_at,
        stale_after: fetched_at.map(|at| at.saturating_add(STALE_AFTER_MS)),
        last_attempt_at: attempted_at,
        windows,
        balances,
        error: None,
    }
}

trait WithError {
    fn with_error(self, error: &str) -> Self;
}
impl WithError for SubscriptionUsageSource {
    fn with_error(mut self, error: &str) -> Self {
        self.error = Some(error.into());
        self
    }
}

fn canonical_provider(provider: &str) -> &str {
    if matches!(provider, "minimax" | "mmx") {
        "minimax"
    } else {
        provider
    }
}

fn stop_child(child: &mut Child) {
    if child.try_wait().ok().flatten().is_some() {
        return;
    }
    #[cfg(unix)]
    unsafe {
        let _ = libc::kill(-(child.id() as i32), libc::SIGTERM);
    }
    let deadline = Instant::now() + Duration::from_millis(250);
    while Instant::now() < deadline {
        if child.try_wait().ok().flatten().is_some() {
            return;
        }
        thread::sleep(Duration::from_millis(10));
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

    #[cfg(unix)]
    #[test]
    fn codex_quota_routes_each_home_and_preserves_identity_on_failure() {
        use std::{fs, os::unix::fs::PermissionsExt};
        let root = std::env::temp_dir().join(format!("monitter-quota-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("personal")).unwrap();
        fs::create_dir_all(root.join("work")).unwrap();
        let executable = root.join("fake-codex");
        // Only the initialize handshake and account read are accepted. A model
        // prompt or any authentication mutation makes the probe fail.
        fs::write(&executable, r#"#!/bin/sh
IFS= read -r request
case "$request" in *'"method":"initialize"'*) ;; *) exit 11;; esac
printf '%s\n' '{"id":1,"result":{}}'
IFS= read -r request
case "$request" in *'"method":"initialized"'*) ;; *) exit 12;; esac
IFS= read -r request
case "$request" in *'"method":"account/rateLimits/read"'*) ;; *) exit 13;; esac
case "$CODEX_HOME" in */personal) used=12;; */work) used=67;; *) exit 14;; esac
printf '{"id":2,"result":{"rateLimits":{"primary":{"usedPercent":%s,"windowDurationMins":300}}}}\n' "$used"
"#).unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        let host: Host = serde_json::from_value(json!({
            "id":"local", "name":"Local", "kind":"local", "address":"", "user":"",
            "port":22, "identityFile":"", "defaultCwd":"", "codexPath":executable,
            "claudePath":"", "opencodePath":"", "hermesPath":""
        })).unwrap();
        let inherited = std::env::var_os("CODEX_HOME");
        for (folder, label, percent) in [("personal", "Personal", 12.0), ("work", "Work", 67.0)] {
            let home = root.join(folder).to_string_lossy().into_owned();
            let result = refresh_codex_quota(&host, &home, label);
            assert_eq!(result.state, "available");
            assert_eq!(result.codex_home.as_deref(), Some(home.as_str()));
            assert_eq!(result.account_label.as_deref(), Some(label));
            assert_eq!(result.windows[0].used_percent, Some(percent));
        }
        let missing = root.join("missing").to_string_lossy().into_owned();
        let failed = refresh_codex_quota(&host, &missing, "Missing");
        assert_eq!(failed.state, "error");
        assert_eq!(failed.codex_home.as_deref(), Some(missing.as_str()));
        assert_eq!(failed.account_label.as_deref(), Some("Missing"));
        assert_eq!(std::env::var_os("CODEX_HOME"), inherited);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn codex_windows_convert_seconds_and_keep_provider_percentage() {
        let (_, windows, balances) = normalize_codex(json!({"result":{"rateLimits":{
            "planType":"pro", "primary":{"usedPercent":86,"windowDurationMins":10080,"resetsAt":1789807205},
            "credits":{"hasCredits":true,"unlimited":false,"balance":"4.5"}
        }}})).unwrap();
        assert_eq!(windows[0].used_percent, Some(86.0));
        assert_eq!(windows[0].resets_at, Some(1_789_807_205_000));
        assert_eq!(balances[0].remaining, Some(4.5));
    }

    #[test]
    fn codex_window_names_come_from_duration_not_primary_or_secondary_position() {
        let (_, windows, _) = normalize_codex(json!({"result":{"rateLimits":{
            "primary":{"usedPercent":10,"windowDurationMins":10080,"resetsAt":1789807205},
            "secondary":{"usedPercent":20,"windowDurationMins":300,"resetsAt":1789807205}
        }}}))
        .unwrap();
        assert_eq!(windows[0].key, "weekly");
        assert_eq!(windows[0].label, "Week");
        assert_eq!(windows[1].key, "five_hour");
        assert_eq!(windows[1].label, "5-hour");
        assert!(windows
            .iter()
            .all(|window| window.metric == "combined" && window.unit == "unknown"));
    }

    #[test]
    fn codex_additional_limits_keep_model_buckets_separate() {
        let (_, windows, _) = normalize_codex(json!({"result":{"rateLimits":{
            "primary":{"usedPercent":10,"windowDurationMins":300,"resetsAt":1789807205},
            "additionalRateLimits":[{
                "normalModelSlug":"gpt-5.3-codex", "primary":{"usedPercent":60,"windowDurationMins":300,"resetsAt":1789807205}
            }]
        }}})).unwrap();
        assert_eq!(windows[0].key, "five_hour");
        assert_eq!(windows[1].key, "additional-1-five_hour");
        assert_eq!(windows[1].label, "gpt-5.3-codex 5-hour");
    }

    #[test]
    fn minimax_maps_remaining_percentage_to_used_without_scaling_millis() {
        let windows = normalize_minimax(json!({"model_remains":[{
            "model_name":"general", "current_interval_status":1, "current_interval_remaining_percent":88,
            "end_time":1789430400000i64, "current_weekly_status":1, "current_weekly_remaining_percent":63,
            "weekly_end_time":1789948800000i64
        }]})).unwrap();
        assert_eq!(windows[0].used_percent, Some(12.0));
        assert_eq!(windows[0].key, "general-five_hour");
        assert_eq!(windows[0].label, "general 5-hour");
        assert_eq!(windows[0].resets_at, Some(1_789_430_400_000));
        assert_eq!(windows[1].used_percent, Some(37.0));
    }

    #[test]
    fn minimax_status_three_is_explicitly_unlimited() {
        let windows = normalize_minimax(json!({"base_resp":{"status_code":0},"model_remains":[{
            "model_name":"video", "current_interval_status":3, "current_interval_remaining_percent":100,
            "end_time":1789430400000i64
        }]})).unwrap();
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].used_percent, None);
        assert_eq!(windows[0].unit, "unknown");
        assert!(windows[0].label.contains("unlimited"));
    }

    #[test]
    fn minimax_uses_the_required_literal_argv() {
        assert_eq!(
            minimax_args(),
            ["quota", "show", "--output", "json", "--non-interactive"]
        );
    }

    #[test]
    fn opencode_go_maps_authoritative_allowance_windows() {
        let windows = normalize_opencode_go(json!({
            "provider":"opencode-go",
            "usage":{
                "rolling":{"status":"ok","percent":12,"resetsAtMs":1789430400000i64},
                "weekly":{"status":"ok","percent":34,"resetsAtMs":1789948800000i64},
                "monthly":{"status":"rate-limited","percent":100,"resetsAtMs":1790812800000i64}
            }
        }))
        .unwrap();
        assert_eq!(windows.len(), 3);
        assert_eq!(windows[0].key, "rolling");
        assert_eq!(windows[0].used_percent, Some(12.0));
        assert_eq!(windows[1].label, "Week");
        assert_eq!(windows[2].used_percent, Some(100.0));
        assert!(windows
            .iter()
            .all(|window| window.metric == "spend" && window.unit == "usd"));
    }

    #[test]
    fn opencode_go_uses_the_router_credential_broker_command() {
        assert_eq!(
            opencode_go_args(),
            ["provider-quota", "opencode-go", "--json"]
        );
    }

    #[test]
    fn quota_probe_path_supplies_node_locations_for_finder_launches() {
        let path = build_quota_probe_path(
            Some(Path::new("/Users/tester")),
            Some(OsStr::new("/usr/bin:/bin:relative-bin")),
        );
        let entries = std::env::split_paths(&path).collect::<Vec<_>>();
        assert!(entries.contains(&PathBuf::from("/Users/tester/.local/bin")));
        assert!(entries.contains(&PathBuf::from("/Users/tester/.npm-global/bin")));
        assert!(entries.contains(&PathBuf::from("/opt/homebrew/bin")));
        assert!(entries.contains(&PathBuf::from("/usr/local/bin")));
        assert!(!entries.contains(&PathBuf::from("relative-bin")));
    }

    #[test]
    fn claude_uses_the_verified_literal_argv() {
        assert_eq!(
            claude_usage_args(),
            ["-p", "/usage", "--output-format", "json"]
        );
    }

    #[test]
    fn claude_parses_the_verified_account_windows_without_retaining_raw_output() {
        let attempted_at = 1_789_473_600_000;
        let windows = normalize_claude_usage(json!({
            "local_command": "usage", "is_error": false,
            "result": "Current session: 0% used · resets Sep 15 at 7pm (Europe/Madrid)\nCurrent week (all models): 6% used · resets Sep 19 at 1am (Europe/Madrid)"
        }), attempted_at).unwrap();
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].key, "session");
        assert_eq!(windows[0].used_percent, Some(0.0));
        assert_eq!(windows[0].resets_at, Some(1_789_491_600_000));
        assert_eq!(windows[1].key, "week_all_models");
        assert_eq!(windows[1].used_percent, Some(6.0));
        assert_eq!(windows[1].resets_at, Some(1_789_772_400_000));
        assert!(windows
            .iter()
            .all(|window| window.metric == "combined" && window.unit == "unknown"));
    }

    #[test]
    fn claude_parser_tolerates_heading_spacing_decimal_percent_and_named_zone() {
        let windows = normalize_claude_usage(json!({
            "local_command":"usage", "is_error":false,
            "result":"** Current session ** : 24.5 % used - resets Sep 15, 2026 at 7:05 PM (America/New_York)\n> current week ( all models ) : 76 % used - resets Sep 19, 2026 at 1 AM (America/New_York)"
        }), 0).unwrap();
        assert_eq!(windows[0].used_percent, Some(24.5));
        assert_eq!(windows[1].used_percent, Some(76.0));
        assert!(windows.iter().all(|window| window.resets_at.is_some()));
    }

    #[test]
    fn claude_omitted_year_advances_to_the_next_plausible_reset() {
        let windows = normalize_claude_usage(json!({
            "local_command":"usage", "is_error":false,
            "result":"Current session: 1% used; resets Sep 15 at 7pm (Europe/Madrid)\nCurrent week (all models): 2% used; resets Sep 19 at 1am (Europe/Madrid)"
        }), 1_789_500_000_000).unwrap();
        let year = Timestamp::from_millisecond(windows[0].resets_at.unwrap())
            .unwrap()
            .in_tz("Europe/Madrid")
            .unwrap()
            .year();
        assert_eq!(year, 2027);
    }

    #[test]
    fn claude_parser_rejects_malformed_or_error_envelopes_and_incomplete_usage() {
        let valid_result = "Current session: 1% used; resets in 1h\nCurrent week (all models): 2% used; resets in 1d";
        assert!(normalize_claude_usage(json!(null), 0).is_err());
        assert!(normalize_claude_usage(
            json!({"local_command":"other", "is_error":false, "result":valid_result}),
            0
        )
        .is_err());
        assert!(normalize_claude_usage(
            json!({"local_command":"usage", "is_error":true, "result":valid_result}),
            0
        )
        .is_err());
        assert!(normalize_claude_usage(
            json!({"local_command":"usage", "is_error":false, "result":"Current session: 1% used"}),
            0
        )
        .is_err());
        assert!(normalize_claude_usage(
            json!({"local_command":"usage", "is_error":false, "result":42}),
            0
        )
        .is_err());
    }

    #[test]
    fn claude_failure_remote_and_freshness_states_are_explicit() {
        let mut host = Host {
            id: "local".into(),
            name: "Local".into(),
            kind: "local".into(),
            address: String::new(),
            user: String::new(),
            port: 22,
            identity_file: String::new(),
            default_cwd: String::new(),
            codex_path: String::new(),
            claude_path: "/definitely/not/a/claude-binary".into(),
            opencode_path: String::new(),
            hermes_path: String::new(),
        };
        let failed = refresh_provider_quota(&host, "claude");
        assert_eq!(failed.state, "error");
        assert_eq!(failed.fetched_at, None);
        assert_eq!(failed.stale_after, None);
        assert_eq!(
            failed.error.as_deref(),
            Some("Claude quota probe is unavailable on this host.")
        );

        host.kind = "ssh".into();
        let remote = refresh_provider_quota(&host, "claude");
        assert_eq!(remote.state, "not-applicable");
        assert_eq!(remote.error, None);

        host.kind = "local".into();
        let fetched_at = 1_789_473_600_000;
        let available = success(
            "claude",
            &host,
            "claude -p /usage --output-format json",
            fetched_at,
            None,
            vec![],
            vec![],
        );
        assert_eq!(available.state, "available");
        assert_eq!(available.fetched_at, Some(fetched_at));
        assert_eq!(available.stale_after, Some(fetched_at + STALE_AFTER_MS));
    }
}
