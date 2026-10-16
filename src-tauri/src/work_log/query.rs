//! Bounded, parameterized reads of the private Work Log. Caller authorization is
//! performed by the coordinator; `ReadScope` is an additional SQL-level fence.
use super::types::{
    UsageProjection, WorkDay, WorkEntry, WorkPage, WorkQuery, WorkRun, WorkSummary,
};
use crate::model::UsageTokens;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::{DateTime, Duration, TimeZone, Utc};
use chrono_tz::Tz;
use rusqlite::{params_from_iter, types::Value, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    str::FromStr,
    sync::mpsc,
    time::{Duration as StdDuration, Instant},
};

const MAX_PAGE: usize = 64;
const MAX_PAGE_BYTES: usize = 32 * 1024;
const MAX_RUNS: usize = 10_000;
const MAX_RANGE_MS: i64 = 366 * 86_400_000;
const DEADLINE: StdDuration = StdDuration::from_millis(250);

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadScope {
    pub installation_id: Option<String>,
    pub profile_id: Option<String>,
    pub projects: Vec<String>,
    pub tasks: Vec<String>,
    pub lineages: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct Cursor {
    sequence: i64,
    event_id: String,
    fingerprint: String,
}

/// Cancel long SQLite statements without a connection-global progress hook.
/// Dropping the sender wakes and joins the timer before the connection is reused.
struct Deadline {
    cancel: Option<mpsc::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
    started: Instant,
}
impl Deadline {
    fn start(conn: &Connection) -> Self {
        let interrupt = conn.get_interrupt_handle();
        let (tx, rx) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            if matches!(
                rx.recv_timeout(DEADLINE),
                Err(mpsc::RecvTimeoutError::Timeout)
            ) {
                interrupt.interrupt();
            }
        });
        Self {
            cancel: Some(tx),
            thread: Some(thread),
            started: Instant::now(),
        }
    }
    fn check(&self) -> Result<(), String> {
        if self.started.elapsed() >= DEADLINE {
            Err("Work log query timed out.".into())
        } else {
            Ok(())
        }
    }
}
impl Drop for Deadline {
    fn drop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            let _ = cancel.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn validate(query: &WorkQuery, scope: Option<&ReadScope>) -> Result<(Tz, usize), String> {
    let tz = query.timezone.as_deref().unwrap_or("UTC");
    if tz.len() > 80 {
        return Err("Invalid work log timezone.".into());
    }
    let tz = Tz::from_str(tz).map_err(|_| "Invalid work log timezone.")?;
    let limit = query.limit.unwrap_or(32) as usize;
    if !(1..=MAX_PAGE).contains(&limit) {
        return Err("Invalid work log page limit.".into());
    }
    if let (Some(from), Some(through)) = (query.from, query.through) {
        if through <= from
            || through
                .checked_sub(from)
                .is_none_or(|span| span > MAX_RANGE_MS)
        {
            return Err("Invalid work log time range.".into());
        }
    }
    for v in [
        query.project_id.as_deref(),
        query.task_id.as_deref(),
        query.agent_id.as_deref(),
        query.host_id.as_deref(),
        query.harness.as_deref(),
        query.model.as_deref(),
        query.phase.as_deref(),
        query.outcome.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        if v.is_empty() || v.len() > 128 || v.chars().any(char::is_control) {
            return Err("Invalid work log filter.".into());
        }
    }
    if let Some(s) = scope {
        if s.projects.len() + s.tasks.len() + s.lineages.len() > 128
            || s.projects
                .iter()
                .chain(&s.tasks)
                .chain(&s.lineages)
                .any(|v| v.is_empty() || v.len() > 128)
        {
            return Err("Invalid work log read scope.".into());
        }
    }
    Ok((tz, limit))
}

fn fingerprint(query: &WorkQuery, scope: Option<&ReadScope>) -> Result<String, String> {
    let mut q = query.clone();
    q.cursor = None;
    q.limit = None;
    let data = serde_json::to_vec(&(q, scope)).map_err(|_| "Invalid work log query.")?;
    Ok(format!("{:x}", Sha256::digest(data)))
}

fn placeholders(count: usize) -> String {
    std::iter::repeat_n("?", count)
        .collect::<Vec<_>>()
        .join(",")
}

/// A latest-version context subquery is necessary because deletion appends a
/// tombstone. Its immutable lineage root is recorded at capture, not inferred
/// from a mutable title or a single parent edge.
fn lineage_sql() -> &'static str {
    "json_extract((SELECT c.raw_json FROM context_refs c WHERE c.context_ref=r.context_ref ORDER BY c.version DESC LIMIT 1),'$.lineageRootTaskId')"
}

fn add_filter(terms: &mut Vec<String>, values: &mut Vec<Value>, term: &str, value: Option<&str>) {
    if let Some(value) = value {
        terms.push(format!("{term}=?"));
        values.push(Value::Text(value.into()));
    }
}
fn filters(query: &WorkQuery, scope: Option<&ReadScope>, event: bool) -> (String, Vec<Value>) {
    let mut terms = vec!["1=1".to_string()];
    let mut values = Vec::new();
    add_filter(
        &mut terms,
        &mut values,
        "r.project_id",
        query.project_id.as_deref(),
    );
    add_filter(
        &mut terms,
        &mut values,
        "r.task_id",
        query.task_id.as_deref(),
    );
    add_filter(
        &mut terms,
        &mut values,
        "r.agent_id",
        query.agent_id.as_deref(),
    );
    add_filter(
        &mut terms,
        &mut values,
        "r.host_id",
        query.host_id.as_deref(),
    );
    add_filter(
        &mut terms,
        &mut values,
        "r.harness",
        query.harness.as_deref(),
    );
    if let Some(model) = query.model.as_deref() {
        terms.push("(r.configured_model=? OR r.observed_model=?)".into());
        values.extend([Value::Text(model.into()), Value::Text(model.into())]);
    }
    add_filter(
        &mut terms,
        &mut values,
        "r.outcome",
        query.outcome.as_deref(),
    );
    if let Some(from) = query.from {
        terms.push("COALESCE(r.ended_at,r.observed_at)>?".into());
        values.push(Value::Integer(from));
    }
    if let Some(through) = query.through {
        terms.push("r.started_at<?".into());
        values.push(Value::Integer(through));
    }
    if event {
        if let Some(from) = query.from {
            terms.push("e.occurred_at>=?".into());
            values.push(Value::Integer(from));
        }
        if let Some(through) = query.through {
            terms.push("e.occurred_at<?".into());
            values.push(Value::Integer(through));
        }
    }
    if let Some(phase) = query.phase.as_deref() {
        if event {
            terms.push("e.phase=?".into());
        } else {
            terms.push(
                "EXISTS(SELECT 1 FROM events pe WHERE pe.run_id=r.run_id AND pe.phase=?)".into(),
            );
        }
        values.push(Value::Text(phase.into()));
    }
    if let Some(s) = scope {
        for (key, id) in [
            ("installationId", s.installation_id.as_ref()),
            ("profileId", s.profile_id.as_ref()),
        ] {
            if let Some(id) = id {
                terms.push(format!("json_extract((SELECT c.raw_json FROM context_refs c WHERE c.context_ref=r.context_ref ORDER BY c.version DESC LIMIT 1),'$.{key}')=?"));
                values.push(Value::Text(id.clone()));
            }
        }
        let mut grants = Vec::new();
        for (column, ids) in [
            ("r.project_id", &s.projects),
            ("r.task_id", &s.tasks),
            (lineage_sql(), &s.lineages),
        ] {
            if !ids.is_empty() {
                grants.push(format!("{column} IN ({})", placeholders(ids.len())));
                values.extend(ids.iter().cloned().map(Value::Text));
            }
        }
        terms.push(if grants.is_empty() {
            "0=1".into()
        } else {
            format!("({})", grants.join(" OR "))
        });
    }
    (terms.join(" AND "), values)
}

fn sql_error(_: rusqlite::Error) -> String {
    "Work log query unavailable.".into()
}

pub fn page(
    conn: &Connection,
    query: &WorkQuery,
    scope: Option<&ReadScope>,
) -> Result<WorkPage, String> {
    let (_, limit) = validate(query, scope)?;
    let fp = fingerprint(query, scope)?;
    let cursor = query
        .cursor
        .as_deref()
        .map(|s| -> Result<Cursor, String> {
            if s.len() > 512 {
                return Err("Invalid work log cursor.".into());
            }
            let bytes = URL_SAFE_NO_PAD
                .decode(s)
                .map_err(|_| "Invalid work log cursor.")?;
            let cursor: Cursor =
                serde_json::from_slice(&bytes).map_err(|_| "Invalid work log cursor.")?;
            if cursor.fingerprint != fp || cursor.sequence < 0 || cursor.event_id.len() > 128 {
                return Err("Invalid work log cursor.".into());
            }
            Ok(cursor)
        })
        .transpose()?;
    let deadline = Deadline::start(conn);
    let (where_sql, mut values) = filters(query, scope, true);
    let mut sql = format!("SELECT e.sequence,e.source_key,e.event_id,e.raw_json,r.raw_json FROM events e JOIN runs r ON r.run_id=e.run_id WHERE {where_sql}");
    if let Some(c) = &cursor {
        sql.push_str(" AND (e.sequence>? OR (e.sequence=? AND e.event_id>?))");
        values.extend([
            Value::Integer(c.sequence),
            Value::Integer(c.sequence),
            Value::Text(c.event_id.clone()),
        ]);
    }
    sql.push_str(" ORDER BY e.sequence,e.event_id LIMIT ?");
    values.push(Value::Integer((limit + 1) as i64));
    let mut stmt = conn.prepare(&sql).map_err(sql_error)?;
    let mut rows = stmt.query(params_from_iter(values)).map_err(sql_error)?;
    let mut entries = Vec::new();
    let mut has_more = false;
    while let Some(row) = rows.next().map_err(sql_error)? {
        deadline.check()?;
        if entries.len() == limit {
            has_more = true;
            break;
        }
        let sequence: i64 = row.get(0).map_err(sql_error)?;
        let source_key: String = row.get(1).map_err(sql_error)?;
        let event_raw: String = row.get(3).map_err(sql_error)?;
        let run_raw: String = row.get(4).map_err(sql_error)?;
        let event = serde_json::from_str(&event_raw).map_err(|_| "Work log query unavailable.")?;
        let mut run_value: serde_json::Value =
            serde_json::from_str(&run_raw).map_err(|_| "Work log query unavailable.")?;
        if scope.is_some() {
            // A granted agent can follow the opaque context ref but does not
            // receive local filesystem locations or native backend identity.
            if let Some(object) = run_value.as_object_mut() {
                object.remove("cwd");
                object.remove("repositoryRoot");
                object.insert("backendSessionId".into(), serde_json::json!(""));
            }
        }
        let run = serde_json::from_value(run_value).map_err(|_| "Work log query unavailable.")?;
        let candidate = WorkEntry {
            sequence,
            source_key,
            run,
            event,
        };
        let mut test = entries.clone();
        test.push(candidate.clone());
        if serde_json::to_vec(&test)
            .map_err(|_| "Work log query unavailable.")?
            .len()
            > MAX_PAGE_BYTES - 2048
        {
            if entries.is_empty() {
                return Err("Work log entry exceeds page limit.".into());
            }
            has_more = true;
            break;
        }
        entries.push(candidate);
    }
    let next_cursor = if has_more {
        entries.last().map(|e| {
            URL_SAFE_NO_PAD.encode(
                serde_json::to_vec(&Cursor {
                    sequence: e.sequence,
                    event_id: e.event.event_id.clone(),
                    fingerprint: fp.clone(),
                })
                .unwrap_or_default(),
            )
        })
    } else {
        None
    };
    Ok(WorkPage {
        entries,
        next_cursor,
        coverage: vec!["eventTimeline".into()],
    })
}

fn add_token(dst: &mut Option<i64>, src: Option<i64>) -> Result<(), String> {
    if let Some(src) = src {
        *dst = Some(
            dst.unwrap_or(0)
                .checked_add(src)
                .ok_or("Work log aggregate overflow.")?,
        );
    }
    Ok(())
}
fn add_tokens(dst: &mut UsageTokens, src: &UsageTokens) -> Result<(), String> {
    add_token(&mut dst.input, src.input)?;
    add_token(&mut dst.output, src.output)?;
    add_token(&mut dst.cache_read, src.cache_read)?;
    add_token(&mut dst.cache_write, src.cache_write)?;
    add_token(&mut dst.reasoning, src.reasoning)?;
    add_token(&mut dst.total, src.total)
}
fn to_utc(ms: i64) -> Result<DateTime<Utc>, String> {
    Utc.timestamp_millis_opt(ms)
        .single()
        .ok_or("Invalid work log time range.".into())
}

fn add_span(
    days: &mut BTreeMap<String, (u64, HashMap<String, Vec<(i64, i64)>>)>,
    tz: Tz,
    start: i64,
    end: i64,
    project: Option<&str>,
) -> Result<(), String> {
    let mut cursor = start;
    while cursor < end {
        let day = to_utc(cursor)?.with_timezone(&tz).date_naive();
        let next = day.succ_opt().ok_or("Invalid work log time range.")?;
        // `earliest` handles fall-back ambiguity. A nonexistent midnight is
        // resolved by moving forward minute by minute, bounded to one day.
        let mut boundary = None;
        for minute in 0..1440 {
            let local = next
                .and_hms_opt(0, 0, 0)
                .ok_or("Invalid work log time range.")?
                + Duration::minutes(minute);
            if let Some(dt) = tz.from_local_datetime(&local).earliest() {
                boundary = Some(dt.timestamp_millis());
                break;
            }
        }
        let stop = end.min(boundary.ok_or("Invalid work log timezone.")?);
        if stop <= cursor {
            return Err("Invalid work log timezone.".into());
        }
        let entry = days.entry(day.format("%Y-%m-%d").to_string()).or_default();
        entry.0 = entry.0.saturating_add((stop - cursor) as u64);
        if let Some(project) = project {
            entry
                .1
                .entry(project.to_owned())
                .or_default()
                .push((cursor, stop));
        }
        cursor = stop;
    }
    Ok(())
}
fn union_ms(mut spans: Vec<(i64, i64)>) -> u64 {
    spans.sort_unstable();
    let (mut total, mut current): (u64, Option<(i64, i64)>) = (0, None);
    for (a, b) in spans {
        if let Some((s, e)) = current {
            if a <= e {
                current = Some((s, e.max(b)));
            } else {
                total = total.saturating_add((e - s) as u64);
                current = Some((a, b));
            }
        } else {
            current = Some((a, b));
        }
    }
    if let Some((s, e)) = current {
        total = total.saturating_add((e - s) as u64);
    }
    total
}

pub fn summary(
    conn: &Connection,
    query: &WorkQuery,
    scope: Option<&ReadScope>,
) -> Result<WorkSummary, String> {
    let (tz, _) = validate(query, scope)?;
    if query.phase.is_some() {
        return Err("Phase filters cannot allocate run-level metrics.".into());
    }
    // Prevent an accidental lifetime-wide scan. The caller may select one
    // task/project without dates; even then the run count is capped below.
    if query.from.is_none()
        && query.through.is_none()
        && query.project_id.is_none()
        && query.task_id.is_none()
        && scope.is_none()
    {
        return Err("Work log summary requires a range or source filter.".into());
    }
    let deadline = Deadline::start(conn);
    let (where_sql, mut values) = filters(query, scope, false);
    let sql = format!(
        "SELECT r.raw_json FROM runs r WHERE {where_sql} ORDER BY r.started_at,r.run_id LIMIT ?"
    );
    values.push(Value::Integer((MAX_RUNS + 1) as i64));
    let mut stmt = conn.prepare(&sql).map_err(sql_error)?;
    let raw = stmt
        .query_map(params_from_iter(values), |r| r.get::<_, String>(0))
        .map_err(sql_error)?;
    let mut runs = Vec::new();
    for row in raw {
        deadline.check()?;
        runs.push(
            serde_json::from_str::<WorkRun>(&row.map_err(sql_error)?)
                .map_err(|_| "Work log query unavailable.")?,
        );
    }
    if runs.len() > MAX_RUNS {
        return Err("Work log summary range is too broad.".into());
    }
    let mut out: WorkSummary = serde_json::from_value(serde_json::json!({
        "days":[], "runs":runs.len(), "agentElapsedMs":0,
        "projectCoveredMs":0, "knownWaitMs":null, "runnableMs":null,
        "cpuMs":null, "rssByteMs":null, "coveredResourceMs":0,
        "sampledPeakRssBytes":null, "tokens":UsageTokens::default(),
        "missingUsageRuns":0, "reportedCostUnits":null,
        "costScale":1_000_000, "currency":null, "coverage":[],
        "resolution":"rawOrBucket"
    }))
    .map_err(|_| "Work log query unavailable.")?;
    let mut coverage = BTreeSet::new();
    let mut days: BTreeMap<String, (u64, HashMap<String, Vec<(i64, i64)>>)> = BTreeMap::new();
    let mut projects: HashMap<String, Vec<(i64, i64)>> = HashMap::new();
    let mut currencies = BTreeSet::new();
    let mut wait_total = 0u64;
    let mut runnable_total = 0u64;
    let mut wait_seen = false;
    let mut runnable_seen = false;
    for run in &runs {
        deadline.check()?;
        let end = run.ended_at.unwrap_or(run.observed_at);
        let start = run.started_at.max(query.from.unwrap_or(i64::MIN));
        let stop = end.min(query.through.unwrap_or(i64::MAX));
        let whole = start == run.started_at && stop == end && run.ended_at.is_some();
        if run.coverage != "observed" {
            coverage.insert("partialRunCoverage");
        }
        if !whole {
            coverage.insert("partialRunBoundaryOrActive");
            coverage.insert("monotonicTotalsLowerBound");
        }
        if start < stop && whole {
            if let Some(elapsed) = run.elapsed_ms {
                out.agent_elapsed_ms = out
                    .agent_elapsed_ms
                    .checked_add(elapsed)
                    .ok_or("Work log aggregate overflow.")?;
            } else {
                coverage.insert("missingElapsed");
            }
        }
        if start < stop {
            gap_coverage(conn, &run.run_id, query, &mut coverage, &deadline)?;
            interval_totals(
                conn,
                run,
                query,
                tz,
                &mut days,
                &mut projects,
                &mut wait_total,
                &mut runnable_total,
                &mut wait_seen,
                &mut runnable_seen,
                &mut coverage,
                &deadline,
            )?;
        }
        let mut projections = conn
            .prepare("SELECT totals_json FROM usage_projection WHERE run_id=?1 LIMIT 513")
            .map_err(sql_error)?;
        let rows = projections
            .query_map([&run.run_id], |r| r.get::<_, String>(0))
            .map_err(sql_error)?;
        let mut any = false;
        let mut complete_run = true;
        let mut projection_count = 0usize;
        for row in rows {
            deadline.check()?;
            projection_count += 1;
            if projection_count > 512 {
                return Err("Work log summary range is too broad.".into());
            }
            any = true;
            let p: UsageProjection = serde_json::from_str(&row.map_err(sql_error)?)
                .map_err(|_| "Work log query unavailable.")?;
            if !p.complete {
                coverage.insert("incompleteUsage");
                complete_run = false;
            }
            if whole {
                add_tokens(&mut out.tokens, &p.tokens)?;
                if let Some(cost) = p.reported_cost_units {
                    if let Some(currency) = p.currency {
                        currencies.insert(currency);
                        out.reported_cost_units = Some(
                            out.reported_cost_units
                                .unwrap_or(0)
                                .checked_add(cost)
                                .ok_or("Work log aggregate overflow.")?,
                        );
                    } else {
                        coverage.insert("missingCurrency");
                    }
                } else {
                    coverage.insert("missingCost");
                }
            }
        }
        if !any || !whole || !complete_run {
            out.missing_usage_runs += 1;
        }
        if start < stop {
            resource_totals(conn, &run.run_id, query, &mut out, &mut coverage, &deadline)?;
        }
    }
    out.project_covered_ms = projects
        .into_values()
        .map(union_ms)
        .fold(0u64, u64::saturating_add);
    out.days = days
        .into_iter()
        .map(|(date, (agent, projects))| WorkDay {
            date,
            timezone: tz.to_string(),
            agent_elapsed_ms: agent,
            project_covered_ms: projects
                .into_values()
                .map(union_ms)
                .fold(0u64, u64::saturating_add),
        })
        .collect();
    if currencies.len() == 1 {
        out.currency = currencies.into_iter().next();
    } else if currencies.len() > 1 {
        out.reported_cost_units = None;
        coverage.insert("mixedCurrencies");
    }
    if out.missing_usage_runs > 0 {
        coverage.insert("missingUsageRuns");
    }
    if !out.days.is_empty() {
        coverage.insert("dailyWallClockGrouping");
    }
    out.coverage = coverage.into_iter().map(str::to_owned).collect();
    // These fields were added by the coordinator after the frozen C0 branch.
    // JSON construction keeps this isolated module source-compatible with C0.
    let mut value = serde_json::to_value(&out).map_err(|_| "Work log query unavailable.")?;
    value["knownWaitMs"] = if wait_seen {
        serde_json::json!(wait_total)
    } else {
        serde_json::Value::Null
    };
    value["runnableMs"] = if runnable_seen {
        serde_json::json!(runnable_total)
    } else {
        serde_json::Value::Null
    };
    serde_json::from_value(value).map_err(|_| "Work log query unavailable.".into())
}

fn interval_totals(
    conn: &Connection,
    run: &WorkRun,
    query: &WorkQuery,
    tz: Tz,
    days: &mut BTreeMap<String, (u64, HashMap<String, Vec<(i64, i64)>>)>,
    projects: &mut HashMap<String, Vec<(i64, i64)>>,
    wait_total: &mut u64,
    runnable_total: &mut u64,
    wait_seen: &mut bool,
    runnable_seen: &mut bool,
    coverage: &mut BTreeSet<&'static str>,
    deadline: &Deadline,
) -> Result<(), String> {
    let from = query.from.unwrap_or(i64::MIN);
    let through = query.through.unwrap_or(i64::MAX);
    let mut stmt = conn.prepare("SELECT kind,started_at,ended_at,duration_ms FROM work_intervals WHERE run_id=?1 AND ended_at>?2 AND started_at<?3 ORDER BY started_at LIMIT 10001").map_err(sql_error)?;
    let rows = stmt
        .query_map(rusqlite::params![&run.run_id, from, through], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })
        .map_err(sql_error)?;
    let project = run
        .project_id
        .clone()
        .unwrap_or_else(|| format!("task:{}", run.task_id));
    let mut count = 0usize;
    for row in rows {
        deadline.check()?;
        count += 1;
        if count > 10_000 {
            return Err("Work log summary range is too broad.".into());
        }
        let (kind, start, end, duration) = row.map_err(sql_error)?;
        if end < start || duration < 0 {
            return Err("Work log query unavailable.".into());
        }
        let clipped_start = start.max(from);
        let clipped_end = end.min(through);
        if clipped_end <= clipped_start {
            continue;
        }
        let whole = start >= from && end <= through;
        match kind.as_str() {
            "runnable" => {
                if !whole {
                    coverage.insert("partialMonotonicUnavailable");
                    coverage.insert("monotonicTotalsLowerBound");
                }
                if whole {
                    *runnable_seen = true;
                    *runnable_total = runnable_total
                        .checked_add(duration as u64)
                        .ok_or("Work log aggregate overflow.")?;
                }
                projects
                    .entry(project.clone())
                    .or_default()
                    .push((clipped_start, clipped_end));
                add_span(days, tz, clipped_start, clipped_end, Some(&project))?;
            }
            "approvalWait" | "inputWait" => {
                if !whole {
                    coverage.insert("partialMonotonicUnavailable");
                    coverage.insert("monotonicTotalsLowerBound");
                }
                if whole {
                    *wait_seen = true;
                    *wait_total = wait_total
                        .checked_add(duration as u64)
                        .ok_or("Work log aggregate overflow.")?;
                }
                projects
                    .entry(project.clone())
                    .or_default()
                    .push((clipped_start, clipped_end));
                add_span(days, tz, clipped_start, clipped_end, Some(&project))?;
            }
            _ => {
                coverage.insert("unknownInterval");
            }
        }
    }
    if count == 0 {
        coverage.insert("missingIntervals");
    }
    Ok(())
}

fn gap_coverage(
    conn: &Connection,
    run_id: &str,
    query: &WorkQuery,
    coverage: &mut BTreeSet<&'static str>,
    deadline: &Deadline,
) -> Result<(), String> {
    deadline.check()?;
    let gap: Option<i64> = conn.query_row("SELECT 1 FROM collector_gaps WHERE (run_id=?1 OR run_id IS NULL) AND started_at<?2 AND COALESCE(ended_at,9223372036854775807)>?3 LIMIT 1",
        rusqlite::params![run_id,query.through.unwrap_or(i64::MAX),query.from.unwrap_or(i64::MIN)],|r|r.get(0)).optional().map_err(sql_error)?;
    if gap.is_some() {
        coverage.insert("collectorGap");
    }
    Ok(())
}

fn resource_totals(
    conn: &Connection,
    run_id: &str,
    query: &WorkQuery,
    out: &mut WorkSummary,
    coverage: &mut BTreeSet<&'static str>,
    deadline: &Deadline,
) -> Result<(), String> {
    // Compaction replaces raw samples in the same transaction. The anti-join
    // also prevents double counting a malformed overlapping old database.
    let from = query.from.unwrap_or(i64::MIN);
    let through = query.through.unwrap_or(i64::MAX);
    let mut stmt = conn.prepare("SELECT interval_start,sampled_at,covered_ms,cpu_delta_ms,rss_byte_ms,sampled_peak_rss_bytes,quality FROM resource_samples s WHERE s.run_id=?1 AND s.sampled_at>?2 AND s.interval_start<?3 AND NOT EXISTS(SELECT 1 FROM resource_buckets b WHERE b.run_id=s.run_id AND s.interval_start<b.through_at AND s.sampled_at>b.from_at) ORDER BY s.sampled_at LIMIT 100001").map_err(sql_error)?;
    let rows = stmt
        .query_map(rusqlite::params![run_id, from, through], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, Option<i64>>(3)?,
                r.get::<_, Option<i64>>(4)?,
                r.get::<_, Option<i64>>(5)?,
                r.get::<_, String>(6)?,
            ))
        })
        .map_err(sql_error)?;
    let mut count = 0usize;
    for row in rows {
        deadline.check()?;
        count += 1;
        if count > 100_000 {
            return Err("Work log summary range is too broad.".into());
        }
        let (start, end, covered, cpu, rss, peak, quality) = row.map_err(sql_error)?;
        if start < from || end > through {
            coverage.insert("partialResourceBoundary");
            continue;
        }
        if quality != "observed" && quality != "measured" {
            coverage.insert("resourceQualityLimited");
        }
        add_resource(out, covered, cpu, rss, peak, coverage)?;
    }
    let mut stmt = conn.prepare("SELECT from_at,through_at,covered_ms,cpu_ms,rss_byte_ms,peak_rss_bytes,missing_cpu,missing_rss,quality_json FROM resource_buckets WHERE run_id=?1 AND through_at>?2 AND from_at<?3 ORDER BY bucket_start LIMIT 100001").map_err(sql_error)?;
    let rows = stmt
        .query_map(rusqlite::params![run_id, from, through], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, Option<i64>>(3)?,
                r.get::<_, Option<i64>>(4)?,
                r.get::<_, Option<i64>>(5)?,
                r.get::<_, i64>(6)?,
                r.get::<_, i64>(7)?,
                r.get::<_, String>(8)?,
            ))
        })
        .map_err(sql_error)?;
    count = 0;
    for row in rows {
        deadline.check()?;
        count += 1;
        if count > 100_000 {
            return Err("Work log summary range is too broad.".into());
        }
        let (start, end, covered, cpu, rss, peak, missing_cpu, missing_rss, quality) =
            row.map_err(sql_error)?;
        if start < from || end > through {
            coverage.insert("partialResourceBoundary");
            continue;
        }
        if missing_cpu > 0 {
            coverage.insert("missingCpu");
        }
        if missing_rss > 0 {
            coverage.insert("missingRss");
        }
        if quality != "[]" && quality != "{}" && quality != "null" {
            coverage.insert("resourceQualityLimited");
        }
        add_resource(out, covered, cpu, rss, peak, coverage)?;
    }
    Ok(())
}
fn add_resource(
    out: &mut WorkSummary,
    covered: i64,
    cpu: Option<i64>,
    rss: Option<i64>,
    peak: Option<i64>,
    coverage: &mut BTreeSet<&'static str>,
) -> Result<(), String> {
    if covered < 0 {
        return Err("Work log query unavailable.".into());
    }
    out.covered_resource_ms = out
        .covered_resource_ms
        .checked_add(covered as u64)
        .ok_or("Work log aggregate overflow.")?;
    for (dst, src, label) in [
        (&mut out.cpu_ms, cpu, "missingCpu"),
        (&mut out.rss_byte_ms, rss, "missingRss"),
    ] {
        if let Some(src) = src {
            if src < 0 {
                return Err("Work log query unavailable.".into());
            }
            *dst = Some(
                dst.unwrap_or(0)
                    .checked_add(src as u64)
                    .ok_or("Work log aggregate overflow.")?,
            );
        } else {
            coverage.insert(label);
        }
    }
    if let Some(peak) = peak {
        if peak < 0 {
            return Err("Work log query unavailable.".into());
        }
        out.sampled_peak_rss_bytes = Some(out.sampled_peak_rss_bytes.unwrap_or(0).max(peak as u64));
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::work_log::types::{fixtures, WorkEvent};
    use rusqlite::params;

    pub(crate) fn db() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE runs(run_id TEXT PRIMARY KEY,task_id TEXT,project_id TEXT,agent_id TEXT,host_id TEXT,harness TEXT,configured_model TEXT,observed_model TEXT,outcome TEXT,started_at INTEGER,ended_at INTEGER,observed_at INTEGER,context_ref TEXT,raw_json TEXT);
            CREATE TABLE events(event_id TEXT PRIMARY KEY,run_id TEXT,phase TEXT,occurred_at INTEGER,sequence INTEGER,source_key TEXT,raw_json TEXT);
            CREATE TABLE context_refs(context_ref TEXT,version INTEGER,raw_json TEXT);
            CREATE TABLE usage_projection(run_id TEXT,totals_json TEXT);
            CREATE TABLE work_intervals(run_id TEXT,kind TEXT,started_at INTEGER,ended_at INTEGER,duration_ms INTEGER);
            CREATE TABLE resource_samples(run_id TEXT,interval_start INTEGER,sampled_at INTEGER,covered_ms INTEGER,cpu_delta_ms INTEGER,rss_byte_ms INTEGER,sampled_peak_rss_bytes INTEGER,quality TEXT);
            CREATE TABLE resource_buckets(run_id TEXT,bucket_start INTEGER,width_ms INTEGER,from_at INTEGER,through_at INTEGER,covered_ms INTEGER,cpu_ms INTEGER,rss_byte_ms INTEGER,peak_rss_bytes INTEGER,missing_cpu INTEGER,missing_rss INTEGER,quality_json TEXT);
            CREATE TABLE collector_gaps(run_id TEXT,started_at INTEGER,ended_at INTEGER,reason TEXT);")
            .unwrap();
        c
    }
    pub(crate) fn insert(
        c: &Connection,
        id: &str,
        task: &str,
        project: &str,
        root: &str,
        start: i64,
        end: i64,
        seq: i64,
    ) {
        let mut run = fixtures::run(id);
        run.task_id = task.into();
        run.project_id = Some(project.into());
        run.started_at = start;
        run.observed_at = end;
        run.ended_at = Some(end);
        run.elapsed_ms = Some((end - start) as u64);
        let event = WorkEvent {
            event_id: format!("event-{id}"),
            run_id: id.into(),
            context_ref: run.context_ref.clone(),
            kind: "workPlan".into(),
            occurred_at: start,
            observed_at: start,
            phase: Some("plan".into()),
            summary: Some("Planned".into()),
            plan_id: None,
            plan_item_id: None,
            labels: vec![],
            source: "backend".into(),
            coverage: "observed".into(),
        };
        c.execute(
            "INSERT INTO runs VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
            params![
                id,
                task,
                project,
                run.agent_id,
                run.host_id,
                run.harness,
                run.configured_model,
                run.observed_model,
                run.outcome,
                start,
                end,
                end,
                run.context_ref,
                serde_json::to_string(&run).unwrap()
            ],
        )
        .unwrap();
        c.execute(
            "INSERT INTO events VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![
                event.event_id,
                id,
                event.phase,
                event.occurred_at,
                seq,
                format!("source-{id}"),
                serde_json::to_string(&event).unwrap()
            ],
        )
        .unwrap();
        c.execute(
            "INSERT INTO context_refs VALUES (?1,1,?2)",
            params![
                run.context_ref,
                serde_json::json!({"lineageRootTaskId":root}).to_string()
            ],
        )
        .unwrap();
    }
    #[test]
    fn scope_is_applied_before_page_and_cursor_is_bound_to_scope() {
        let c = db();
        insert(&c, "a", "task-a", "p", "root", 0, 1000, 1);
        insert(&c, "b", "task-b", "q", "other", 0, 1000, 2);
        insert(&c, "c", "task-c", "q", "root", 0, 1000, 3);
        let scope = ReadScope {
            lineages: vec!["root".into()],
            ..Default::default()
        };
        c.execute("UPDATE runs SET raw_json=json_set(raw_json,'$.cwd','/private/path','$.repositoryRoot','/private/repo') WHERE run_id='a'",[]).unwrap();
        let mut q = WorkQuery {
            from: Some(0),
            through: Some(2000),
            limit: Some(1),
            ..Default::default()
        };
        let first = page(&c, &q, Some(&scope)).unwrap();
        assert_eq!(first.entries[0].run.run_id, "a");
        assert!(!serde_json::to_string(&first.entries[0].run)
            .unwrap()
            .contains("/private/"));
        q.cursor = first.next_cursor;
        let second = page(&c, &q, Some(&scope)).unwrap();
        assert_eq!(second.entries[0].run.run_id, "c");
        assert!(page(&c, &q, Some(&ReadScope::default())).is_err());
        assert!(page(
            &c,
            &WorkQuery { cursor: None, ..q },
            Some(&ReadScope::default())
        )
        .unwrap()
        .entries
        .is_empty());
    }
    #[test]
    fn event_time_is_filtered_inside_overlapping_run() {
        let c = db();
        insert(&c, "a", "task", "p", "task", 0, 1000, 1);
        let q = WorkQuery {
            from: Some(100),
            through: Some(900),
            ..Default::default()
        };
        assert!(page(&c, &q, None).unwrap().entries.is_empty());
        c.execute("UPDATE events SET occurred_at=500,raw_json=json_set(raw_json,'$.occurredAt',500) WHERE event_id='event-a'",[]).unwrap();
        assert_eq!(page(&c, &q, None).unwrap().entries.len(), 1);
    }
    #[test]
    fn dst_and_parallel_projects_preserve_project_coverage() {
        let c = db();
        let start = 1_774_740_600_000i64; // 2026-03-28 23:30 UTC
        let end = start + 23 * 3_600_000;
        insert(&c, "a", "task-a", "p", "a", start, end, 1);
        insert(&c, "b", "task-b", "q", "b", start, end, 2);
        for id in ["a", "b"] {
            c.execute(
                "INSERT INTO work_intervals VALUES (?1,'runnable',?2,?3,?4)",
                params![id, start, end, end - start],
            )
            .unwrap();
        }
        let q = WorkQuery {
            timezone: Some("Europe/Madrid".into()),
            from: Some(start),
            through: Some(end + 1),
            ..Default::default()
        };
        let s = summary(&c, &q, None).unwrap();
        assert_eq!(s.runs, 2);
        assert_eq!(s.project_covered_ms, 46 * 3_600_000);
        assert_eq!(
            s.days.iter().map(|d| d.project_covered_ms).sum::<u64>(),
            46 * 3_600_000
        );
        assert!(s.days.iter().any(|d| d.date == "2026-03-29"));
    }
    #[test]
    fn middle_day_clips_coalesced_forty_eight_hour_interval() {
        let c = db();
        let day = 86_400_000i64;
        insert(&c, "a", "task", "p", "task", 0, 2 * day, 1);
        c.execute(
            "INSERT INTO work_intervals VALUES ('a','runnable',0,?1,?1)",
            [2 * day],
        )
        .unwrap();
        let s = summary(
            &c,
            &WorkQuery {
                from: Some(day),
                through: Some(2 * day),
                timezone: Some("UTC".into()),
                ..Default::default()
            },
            None,
        )
        .unwrap();
        assert_eq!(s.project_covered_ms, day as u64);
        assert_eq!(
            s.days.iter().map(|d| d.project_covered_ms).sum::<u64>(),
            day as u64
        );
        assert_eq!(s.agent_elapsed_ms, 0);
        assert!(s.coverage.contains(&"partialMonotonicUnavailable".into()));
    }
    #[test]
    fn parallel_runnable_and_known_wait_union_project_wall_once() {
        let c = db();
        insert(&c, "a", "task-a", "p", "a", 0, 1000, 1);
        insert(&c, "b", "task-b", "p", "b", 0, 1000, 2);
        c.execute_batch(
            "INSERT INTO work_intervals VALUES ('a','runnable',0,1000,1000);
            INSERT INTO work_intervals VALUES ('b','approvalWait',200,800,600);",
        )
        .unwrap();
        let s = summary(
            &c,
            &WorkQuery {
                from: Some(0),
                through: Some(1001),
                ..Default::default()
            },
            None,
        )
        .unwrap();
        assert_eq!(s.project_covered_ms, 1000);
        assert_eq!(
            s.days.iter().map(|d| d.project_covered_ms).sum::<u64>(),
            1000
        );
    }
    #[test]
    fn intervals_without_window_intersection_add_no_coverage() {
        let c = db();
        insert(&c, "a", "task", "p", "task", 0, 1000, 1);
        c.execute_batch("INSERT INTO work_intervals VALUES ('a','runnable',0,100,100);")
            .unwrap();
        let s = summary(
            &c,
            &WorkQuery {
                from: Some(500),
                through: Some(900),
                ..Default::default()
            },
            None,
        )
        .unwrap();
        assert_eq!(s.project_covered_ms, 0);
        assert!(s.days.is_empty());
    }
    #[test]
    fn bucket_replaces_overlapping_raw_and_null_usage_stays_null() {
        let c = db();
        insert(&c, "a", "task-a", "p", "a", 0, 1000, 1);
        c.execute_batch(
            "INSERT INTO resource_samples VALUES ('a',0,1000,1000,25,4000,8,'measured');
            INSERT INTO resource_buckets VALUES ('a',0,60000,0,1000,1000,25,4000,8,0,0,'[]');",
        )
        .unwrap();
        let q = WorkQuery {
            from: Some(0),
            through: Some(2000),
            ..Default::default()
        };
        let s = summary(&c, &q, None).unwrap();
        assert_eq!(s.cpu_ms, Some(25));
        assert_eq!(s.covered_resource_ms, 1000);
        assert_eq!(s.tokens.total, None);
        assert_eq!(s.reported_cost_units, None);
        assert_eq!(s.missing_usage_runs, 1);
    }
    #[test]
    fn partial_resource_boundaries_and_gaps_are_disclosed() {
        let c = db();
        insert(&c, "a", "task", "p", "task", 0, 1000, 1);
        c.execute_batch(
            "INSERT INTO resource_samples VALUES ('a',0,500,500,10,1000,4,'measured');
            INSERT INTO resource_samples VALUES ('a',500,900,400,NULL,NULL,NULL,'missing');
            INSERT INTO collector_gaps VALUES ('a',400,600,'sleep');",
        )
        .unwrap();
        let s = summary(
            &c,
            &WorkQuery {
                from: Some(200),
                through: Some(800),
                ..Default::default()
            },
            None,
        )
        .unwrap();
        assert_eq!(s.covered_resource_ms, 0);
        assert!(s.coverage.contains(&"partialResourceBoundary".into()));
        assert!(s.coverage.contains(&"collectorGap".into()));
    }
    #[test]
    fn bucket_missing_counts_and_quality_are_disclosed() {
        let c = db();
        insert(&c, "a", "task", "p", "task", 0, 1000, 1);
        c.execute("INSERT INTO resource_buckets VALUES ('a',0,60000,0,1000,1000,25,4000,8,2,1,'[\"counterReset\"]')",[]).unwrap();
        let s = summary(
            &c,
            &WorkQuery {
                from: Some(0),
                through: Some(2000),
                ..Default::default()
            },
            None,
        )
        .unwrap();
        assert_eq!(s.cpu_ms, Some(25));
        for label in ["missingCpu", "missingRss", "resourceQualityLimited"] {
            assert!(s.coverage.contains(&label.into()));
        }
    }
    #[test]
    fn mixed_currencies_do_not_form_one_cost() {
        let c = db();
        insert(&c, "a", "task-a", "p", "a", 0, 1000, 1);
        insert(&c, "b", "task-b", "p", "b", 0, 1000, 2);
        for (id, currency) in [("a", "USD"), ("b", "EUR")] {
            let p = UsageProjection {
                run_id: id.into(),
                currency: Some(currency.into()),
                reported_cost_units: Some(10),
                complete: true,
                ..Default::default()
            };
            c.execute(
                "INSERT INTO usage_projection VALUES (?1,?2)",
                params![id, serde_json::to_string(&p).unwrap()],
            )
            .unwrap();
        }
        let s = summary(
            &c,
            &WorkQuery {
                from: Some(0),
                through: Some(2000),
                ..Default::default()
            },
            None,
        )
        .unwrap();
        assert_eq!(s.reported_cost_units, None);
        assert_eq!(s.currency, None);
        assert!(s.coverage.contains(&"mixedCurrencies".into()));
    }
    #[test]
    fn phase_summary_rejects_run_level_allocation() {
        let c = db();
        let q = WorkQuery {
            from: Some(0),
            through: Some(2000),
            phase: Some("plan".into()),
            ..Default::default()
        };
        assert!(summary(&c, &q, None).is_err());
    }
    #[test]
    fn deadline_interrupts_expensive_sql() {
        let c = Connection::open_in_memory().unwrap();
        let _deadline = Deadline::start(&c);
        let result: rusqlite::Result<i64> = c.query_row(
            "WITH RECURSIVE n(x) AS (VALUES(0) UNION ALL SELECT x+1 FROM n WHERE x<1000000000) SELECT max(x) FROM n",
            [], |r| r.get(0));
        assert!(result.is_err());
    }
    #[test]
    fn rejects_unbounded_or_oversized_pages() {
        let c = db();
        assert!(summary(&c, &WorkQuery::default(), None).is_err());
        assert!(page(
            &c,
            &WorkQuery {
                limit: Some(65),
                ..Default::default()
            },
            None
        )
        .is_err());
    }
}
