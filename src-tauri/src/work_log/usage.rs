use super::types::{UsageObservation, UsageProjection};
use crate::model::UsageTokens;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

const USD_SCALE: u32 = 1_000_000;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
struct Scope {
    run_id: String,
    provider: String,
    model: Option<String>,
    turn_id: Option<String>,
    epoch: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CounterState {
    last_sequence: i64,
    last_sample_id: Option<String>,
    cumulative: UsageTokens,
    deltas: UsageTokens,
    frozen_fields: [bool; 6],
    cumulative_cost: Option<i64>,
    delta_cost: i64,
    cost_seen: bool,
    cost_frozen: bool,
}

impl Default for CounterState {
    fn default() -> Self {
        Self {
            last_sequence: i64::MIN,
            last_sample_id: None,
            cumulative: UsageTokens::default(),
            deltas: UsageTokens::default(),
            frozen_fields: [false; 6],
            cumulative_cost: None,
            delta_cost: 0,
            cost_seen: false,
            cost_frozen: false,
        }
    }
}

/// Projects canonical samples independently for each real run and provider turn.
/// Primary outbox sequence, rather than caller ordering, determines precedence.
pub fn project(observations: &[UsageObservation]) -> Vec<UsageProjection> {
    let mut ordered = observations.to_vec();
    ordered.sort_by_key(|o| o.sequence);
    let mut seen = HashSet::new();
    let mut groups: BTreeMap<Scope, (CounterState, UsageProjection)> = BTreeMap::new();
    for observation in ordered {
        let s = &observation.sample;
        let scope = Scope {
            run_id: s.run_id.clone(),
            provider: s.provider.clone(),
            model: s.configured_model.clone(),
            turn_id: s.provider_turn_id.clone(),
            epoch: s.counter_epoch.clone().unwrap_or_else(|| "initial".into()),
        };
        if seen.insert((scope.clone(), s.sample_id.clone())) {
            let (state, projection) = groups
                .entry(scope.clone())
                .or_insert_with(|| (CounterState::default(), empty_projection(&scope)));
            consume(state, projection, observation);
        }
    }
    groups
        .into_iter()
        .map(|(_, (_, projection))| projection)
        .collect()
}

fn empty_projection(scope: &Scope) -> UsageProjection {
    UsageProjection {
        run_id: scope.run_id.clone(),
        provider: scope.provider.clone(),
        configured_model: scope.model.clone(),
        provider_turn_id: scope.turn_id.clone(),
        counter_epoch: scope.epoch.clone(),
        cost_scale: USD_SCALE,
        complete: true,
        ..UsageProjection::default()
    }
}

/// Applies only newly accepted raw usage observations to the compact projection
/// rows. The caller owns the surrounding transaction and invokes this before
/// advancing the source watermark, so projection and cursor commit together.
pub fn apply_committed(conn: &Connection, observations: &[UsageObservation]) -> Result<(), String> {
    if observations.is_empty() {
        return Ok(());
    }
    let mut ordered = observations.to_vec();
    ordered.sort_by_key(|o| o.sequence);
    let mut scopes: BTreeMap<String, (Scope, CounterState, UsageProjection)> = BTreeMap::new();
    let mut seen_samples = HashSet::new();
    for observation in &ordered {
        let sample = &observation.sample;
        let scope = Scope {
            run_id: sample.run_id.clone(),
            provider: sample.provider.clone(),
            model: sample.configured_model.clone(),
            turn_id: sample.provider_turn_id.clone(),
            epoch: sample
                .counter_epoch
                .clone()
                .unwrap_or_else(|| "initial".into()),
        };
        let scope_key = serde_json::to_string(&scope)
            .map_err(|_| "Could not encode usage projection scope.".to_string())?;
        if !seen_samples.insert((scope_key.clone(), sample.sample_id.clone())) {
            continue;
        }
        if !scopes.contains_key(&scope_key) {
            let stored = conn.query_row(
                "SELECT state_json, totals_json FROM usage_projection WHERE scope_key = ?1 AND run_id = ?2",
                params![scope_key, scope.run_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            );
            let (state, projection) = match stored {
                Ok((state_json, totals_json)) => (
                    serde_json::from_str(&state_json)
                        .map_err(|_| "Stored usage projection state is invalid.".to_string())?,
                    serde_json::from_str(&totals_json)
                        .map_err(|_| "Stored usage projection totals are invalid.".to_string())?,
                ),
                Err(rusqlite::Error::QueryReturnedNoRows) => {
                    (CounterState::default(), empty_projection(&scope))
                }
                Err(_) => return Err("Could not load usage projection state.".into()),
            };
            scopes.insert(scope_key.clone(), (scope, state, projection));
        }
        let (_, state, projection) = scopes
            .get_mut(&scope_key)
            .expect("inserted projection scope");
        consume(state, projection, (*observation).clone());
    }
    for (scope_key, (scope, state, projection)) in scopes {
        let state_json = serde_json::to_string(&state)
            .map_err(|_| "Could not encode usage projection state.".to_string())?;
        let totals_json = serde_json::to_string(&projection)
            .map_err(|_| "Could not encode usage projection totals.".to_string())?;
        conn.execute(
            "INSERT INTO usage_projection(scope_key, run_id, state_json, totals_json) VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT(scope_key) DO UPDATE SET run_id=excluded.run_id, state_json=excluded.state_json, totals_json=excluded.totals_json",
            params![scope_key, scope.run_id, state_json, totals_json],
        ).map_err(|_| "Could not save usage projection state.".to_string())?;
    }
    Ok(())
}

fn consume(
    state: &mut CounterState,
    projection: &mut UsageProjection,
    observation: UsageObservation,
) {
    let sample = observation.sample;
    if state.last_sample_id.as_deref() == Some(sample.sample_id.as_str()) {
        return;
    }
    if observation.sequence < state.last_sequence {
        return;
    }
    if observation.sequence == state.last_sequence {
        if state.last_sample_id.as_deref() != Some(sample.sample_id.as_str()) {
            projection.complete = false;
            projection.coverage.push("sequenceCollision".into());
        }
        return;
    }
    state.last_sequence = observation.sequence;
    state.last_sample_id = Some(sample.sample_id.clone());
    projection.observations = projection.observations.saturating_add(1);
    projection.final_seen |= sample.final_sample;
    let values_present = has_values(&sample.tokens)
        || sample.cost_usd.is_some()
        || sample.duration_ms.is_some()
        || sample.api_duration_ms.is_some()
        || sample.provider_turns.is_some()
        || sample.context.is_some();
    if !values_present {
        projection.missing_observations = projection.missing_observations.saturating_add(1);
        return;
    }
    let is_delta = sample.classification.eq_ignore_ascii_case("delta");
    consume_token(
        &mut state.cumulative.input,
        &mut state.deltas.input,
        &mut state.frozen_fields[0],
        sample.tokens.input,
        is_delta,
        projection,
    );
    consume_token(
        &mut state.cumulative.output,
        &mut state.deltas.output,
        &mut state.frozen_fields[1],
        sample.tokens.output,
        is_delta,
        projection,
    );
    consume_token(
        &mut state.cumulative.cache_read,
        &mut state.deltas.cache_read,
        &mut state.frozen_fields[2],
        sample.tokens.cache_read,
        is_delta,
        projection,
    );
    consume_token(
        &mut state.cumulative.cache_write,
        &mut state.deltas.cache_write,
        &mut state.frozen_fields[3],
        sample.tokens.cache_write,
        is_delta,
        projection,
    );
    consume_token(
        &mut state.cumulative.reasoning,
        &mut state.deltas.reasoning,
        &mut state.frozen_fields[4],
        sample.tokens.reasoning,
        is_delta,
        projection,
    );
    consume_token(
        &mut state.cumulative.total,
        &mut state.deltas.total,
        &mut state.frozen_fields[5],
        sample.tokens.total,
        is_delta,
        projection,
    );
    if let Some(cost) = sample.cost_usd {
        if let Some(cost) = to_usd_units(cost) {
            state.cost_seen = true;
            if !state.cost_frozen {
                if is_delta {
                    if let Some(total) = state.delta_cost.checked_add(cost) {
                        state.delta_cost = total;
                    } else {
                        state.cost_frozen = true;
                        projection.complete = false;
                        projection.coverage.push("costOverflow".into());
                    }
                } else if state.cumulative_cost.is_some_and(|old| cost < old) {
                    state.cost_frozen = true;
                    projection.complete = false;
                    projection.coverage.push("costCounterReset".into());
                } else {
                    state.cumulative_cost = Some(cost);
                }
            }
        } else {
            projection.complete = false;
            projection.missing_observations = projection.missing_observations.saturating_add(1);
            projection.coverage.push("invalidCost".into());
        }
    }
    refresh_totals(state, projection);
}

fn consume_token(
    cumulative: &mut Option<i64>,
    delta: &mut Option<i64>,
    frozen: &mut bool,
    incoming: Option<i64>,
    is_delta: bool,
    projection: &mut UsageProjection,
) {
    let Some(value) = incoming else {
        return;
    };
    if value < 0 {
        projection.complete = false;
        projection.coverage.push("invalidTokenCount".into());
        return;
    }
    if *frozen {
        return;
    }
    if is_delta {
        *delta = match *delta {
            Some(old) => old.checked_add(value),
            None => Some(value),
        };
        if delta.is_none() {
            *frozen = true;
            projection.complete = false;
            projection.coverage.push("tokenOverflow".into());
        }
    } else if cumulative.is_some_and(|old| value < old) {
        *frozen = true;
        projection.complete = false;
        projection.coverage.push("counterReset".into());
    } else {
        *cumulative = Some(value);
    }
}

fn refresh_totals(state: &mut CounterState, projection: &mut UsageProjection) {
    fn sum(a: Option<i64>, b: Option<i64>) -> (Option<i64>, bool) {
        match (a, b) {
            (Some(a), Some(b)) => match a.checked_add(b) {
                Some(total) => (Some(total), false),
                None => (None, true),
            },
            (Some(a), None) => (Some(a), false),
            (None, Some(b)) => (Some(b), false),
            _ => (None, false),
        }
    }
    macro_rules! checked_total {
        ($index:expr, $field:ident) => {{
            let (value, overflow) = sum(state.cumulative.$field, state.deltas.$field);
            if overflow {
                state.frozen_fields[$index] = true;
                projection.complete = false;
                projection.coverage.push("tokenOverflow".into());
            }
            value
        }};
    }
    projection.tokens = UsageTokens {
        input: checked_total!(0, input),
        output: checked_total!(1, output),
        cache_read: checked_total!(2, cache_read),
        cache_write: checked_total!(3, cache_write),
        reasoning: checked_total!(4, reasoning),
        total: checked_total!(5, total),
    };
    if state.cost_seen {
        projection.reported_cost_units = state
            .cumulative_cost
            .unwrap_or(0)
            .checked_add(state.delta_cost);
        if projection.reported_cost_units.is_none() {
            projection.complete = false;
            projection.coverage.push("costOverflow".into());
        }
        projection.currency = Some("USD".into());
        projection.cost_source = Some("providerReported".into());
        projection.source_precision = Some("floatingUsdRoundedToMillionths".into());
    }
    projection.coverage.sort();
    projection.coverage.dedup();
}

fn has_values(t: &UsageTokens) -> bool {
    t.input.is_some()
        || t.output.is_some()
        || t.cache_read.is_some()
        || t.cache_write.is_some()
        || t.reasoning.is_some()
        || t.total.is_some()
}
fn to_usd_units(value: f64) -> Option<i64> {
    if !value.is_finite() || value < 0.0 {
        return None;
    }
    let scaled = value * USD_SCALE as f64;
    if scaled > i64::MAX as f64 {
        return None;
    }
    Some(scaled.round() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{RunUsageSample, UsageTokens};
    fn observation(
        seq: i64,
        id: &str,
        run: &str,
        classification: &str,
        final_sample: bool,
        input: Option<i64>,
        cost: Option<f64>,
        turn: Option<&str>,
    ) -> UsageObservation {
        UsageObservation {
            sequence: seq,
            sample: RunUsageSample {
                sample_id: id.into(),
                run_id: run.into(),
                task_id: "t".into(),
                provider: "p".into(),
                configured_model: Some("m".into()),
                started_at: 0,
                observed_at: seq,
                final_sample,
                classification: classification.into(),
                provider_turn_id: turn.map(str::to_string),
                counter_epoch: None,
                tokens: UsageTokens {
                    input,
                    ..Default::default()
                },
                cost_usd: cost,
                duration_ms: None,
                api_duration_ms: None,
                provider_turns: None,
                context: None,
            },
        }
    }
    #[test]
    fn sequence_dedupe_scopes_and_empty_final() {
        let out = project(&[
            observation(3, "f", "r", "cumulative", true, None, None, Some("turn")),
            observation(
                2,
                "b",
                "r",
                "cumulative",
                false,
                Some(8),
                Some(0.0000015),
                Some("turn"),
            ),
            observation(
                1,
                "a",
                "r",
                "delta",
                false,
                Some(2),
                Some(0.000001),
                Some("turn"),
            ),
            observation(4, "a", "retry", "delta", false, Some(5), None, Some("turn")),
            observation(
                5,
                "other",
                "r",
                "delta",
                false,
                Some(100),
                None,
                Some("other"),
            ),
        ]);
        assert_eq!(out.len(), 3);
        let main = out
            .iter()
            .find(|x| x.run_id == "r" && x.provider_turn_id.as_deref() == Some("turn"))
            .unwrap();
        assert_eq!(main.tokens.input, Some(10));
        assert_eq!(main.reported_cost_units, Some(3));
        assert!(main.final_seen);
        assert_eq!(main.observations, 3);
        assert_eq!(
            out.iter()
                .find(|x| x.run_id == "retry")
                .unwrap()
                .tokens
                .input,
            Some(5)
        );
    }
    #[test]
    fn cumulative_decrease_marks_incomplete_and_nullable_fields_stay_null() {
        let out = project(&[
            observation(1, "a", "r", "cumulative", false, Some(9), None, None),
            observation(2, "b", "r", "cumulative", false, Some(2), None, None),
        ]);
        assert!(!out[0].complete);
        assert_eq!(out[0].tokens.input, Some(9));
        assert_eq!(out[0].tokens.output, None);
        assert!(out[0].coverage.contains(&"counterReset".into()));
    }

    #[test]
    fn documented_counter_epochs_are_independent_and_unknown_resets_stay_frozen() {
        let mut first = observation(1, "a", "r", "cumulative", false, Some(50), None, None);
        first.sample.counter_epoch = Some("provider-session-1".into());
        let mut next_epoch = observation(2, "b", "r", "cumulative", false, Some(4), None, None);
        next_epoch.sample.counter_epoch = Some("provider-session-2".into());
        let projections = project(&[first, next_epoch]);
        assert_eq!(projections.len(), 2);
        assert!(projections
            .iter()
            .any(|p| p.counter_epoch == "provider-session-1" && p.tokens.input == Some(50)));
        assert!(projections
            .iter()
            .any(|p| p.counter_epoch == "provider-session-2" && p.tokens.input == Some(4)));
    }

    #[test]
    fn token_total_overflow_is_incomplete_instead_of_silently_nullable() {
        let out = project(&[
            observation(1, "a", "r", "cumulative", false, Some(i64::MAX), None, None),
            observation(2, "b", "r", "delta", false, Some(1), None, None),
        ]);
        assert_eq!(out[0].tokens.input, None);
        assert!(!out[0].complete);
        assert!(out[0].coverage.contains(&"tokenOverflow".into()));
    }

    fn projection_db() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE usage_projection(scope_key TEXT PRIMARY KEY, run_id TEXT NOT NULL, state_json TEXT NOT NULL, totals_json TEXT NOT NULL); CREATE INDEX usage_projection_run_id ON usage_projection(run_id);").unwrap();
        conn
    }

    #[test]
    fn committed_projection_rolls_back_and_exact_replay_is_idempotent() {
        let mut conn = projection_db();
        let batch = [
            observation(
                10,
                "sample-a",
                "run",
                "delta",
                false,
                Some(4),
                Some(0.000001),
                Some("turn"),
            ),
            observation(
                11,
                "sample-a",
                "run",
                "delta",
                false,
                Some(400),
                Some(1.0),
                Some("turn"),
            ),
        ];
        {
            let tx = conn.transaction().unwrap();
            apply_committed(&tx, &batch).unwrap();
            // Dropping without commit simulates a later writer/watermark failure.
        }
        let count: i64 = conn
            .query_row("SELECT count(*) FROM usage_projection", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);

        apply_committed(&conn, &batch).unwrap();
        apply_committed(&conn, &batch).unwrap();
        let (state_json, totals_json): (String, String) = conn
            .query_row(
                "SELECT state_json, totals_json FROM usage_projection",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let state: CounterState = serde_json::from_str(&state_json).unwrap();
        let totals: UsageProjection = serde_json::from_str(&totals_json).unwrap();
        assert_eq!(state.last_sequence, 10);
        assert_eq!(totals.observations, 1);
        assert_eq!(totals.tokens.input, Some(4));
        assert_eq!(totals.reported_cost_units, Some(1));
    }

    #[test]
    fn committed_projection_persists_documented_epochs_separately() {
        let conn = projection_db();
        let mut old = observation(1, "old", "run", "cumulative", false, Some(40), None, None);
        old.sample.counter_epoch = Some("epoch-a".into());
        let mut new = observation(2, "new", "run", "cumulative", false, Some(3), None, None);
        new.sample.counter_epoch = Some("epoch-b".into());
        apply_committed(&conn, &[old, new]).unwrap();
        let rows: Vec<String> = conn
            .prepare("SELECT totals_json FROM usage_projection ORDER BY scope_key")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(rows.len(), 2);
        let projections: Vec<UsageProjection> = rows
            .iter()
            .map(|row| serde_json::from_str(row).unwrap())
            .collect();
        assert!(projections
            .iter()
            .any(|p| p.counter_epoch == "epoch-a" && p.tokens.input == Some(40)));
        assert!(projections
            .iter()
            .any(|p| p.counter_epoch == "epoch-b" && p.tokens.input == Some(3)));
    }

    #[test]
    fn one_scope_update_with_many_unrelated_projections_is_incremental() {
        let mut conn = projection_db();
        let empty = serde_json::to_string(&CounterState::default()).unwrap();
        let mut stmt = conn.prepare("INSERT INTO usage_projection(scope_key, run_id, state_json, totals_json) VALUES (?1, ?2, ?3, ?4)").unwrap();
        for index in 0..5_000 {
            let scope = Scope {
                run_id: format!("unrelated-{index}"),
                provider: "p".into(),
                model: None,
                turn_id: None,
                epoch: "initial".into(),
            };
            stmt.execute(params![
                serde_json::to_string(&scope).unwrap(),
                scope.run_id,
                empty,
                serde_json::to_string(&empty_projection(&scope)).unwrap()
            ])
            .unwrap();
        }
        drop(stmt);
        apply_committed(
            &conn,
            &[observation(
                1,
                "one",
                "target",
                "delta",
                false,
                Some(7),
                None,
                None,
            )],
        )
        .unwrap();
        let count: i64 = conn
            .query_row("SELECT count(*) FROM usage_projection", [], |row| {
                row.get(0)
            })
            .unwrap();
        let target: String = conn
            .query_row(
                "SELECT totals_json FROM usage_projection WHERE run_id='target'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let target: UsageProjection = serde_json::from_str(&target).unwrap();
        assert_eq!(count, 5_001);
        assert_eq!(target.tokens.input, Some(7));
        assert_eq!(target.observations, 1);
    }
}
