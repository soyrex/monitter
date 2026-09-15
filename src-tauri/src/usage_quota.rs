//! Read-only, short-lived subscription allowance probes.
//!
//! These probes deliberately do not share the resident task transports: they
//! must never start a thread, submit a prompt, or alter provider auth state.

use crate::{
    model::{now, AllowanceBalance, AllowanceWindow, Host, SubscriptionUsageSource},
    runner::resolve_local,
};
use serde_json::{json, Value};
use std::{
    ffi::{OsStr, OsString},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_OUTPUT_BYTES: usize = 128 * 1024;
const STALE_AFTER_MS: i64 = 60_000;

/// Refresh the subscription allowance that can be safely read on this local
/// host.  The returned source intentionally carries failures rather than
/// returning raw CLI/protocol errors, as those errors can contain account data.
///
/// Supported provider names are `codex`, `minimax` (with `mmx` accepted as an
/// alias), and `opencode-go`. Claude has no safe on-demand allowance read path
/// yet.
pub(crate) fn refresh_provider_quota(host: &Host, provider: &str) -> SubscriptionUsageSource {
    let attempted_at = now();
    let provider = provider.trim().to_ascii_lowercase();

    if provider == "claude" {
        return source(
            "claude",
            host,
            "Claude on-demand allowance refresh",
            "unsupported",
            attempted_at,
            None,
            vec![],
            vec![],
            None,
        );
    }
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
        "codex" => refresh_codex(host, attempted_at),
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

fn refresh_codex(host: &Host, attempted_at: i64) -> SubscriptionUsageSource {
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
    let stdout = child.stdout.take().ok_or(())?;
    let (tx, rx) = mpsc::channel();
    let deadline = Instant::now() + PROBE_TIMEOUT;
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
            "interval",
            "Interval",
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
    fn claude_is_explicitly_unsupported_without_an_error() {
        let host = Host {
            id: "local".into(),
            name: "Local".into(),
            kind: "local".into(),
            address: String::new(),
            user: String::new(),
            port: 22,
            identity_file: String::new(),
            default_cwd: String::new(),
            codex_path: String::new(),
            claude_path: String::new(),
            opencode_path: String::new(),
            hermes_path: String::new(),
        };
        let source = refresh_provider_quota(&host, "claude");
        assert_eq!(source.state, "unsupported");
        assert_eq!(source.error, None);
    }
}
