//! Offline, provenance-labelled routing replay and observed outcome summaries.
//!
//! This module never opens a classifier/provider. A replay stores the typed
//! classifier decision and version labels, then invokes the production native
//! eligibility function. Outcome metrics are computed only from supplied
//! observations; absent evidence remains absent.

use super::{
    replay_auto_route_policy, JevAutoRouteBasis, RoutingDecision, ROUTING_POLICY_VERSION,
    ROUTING_QUESTION_SCHEMA_VERSION,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::Read,
    path::Path,
};

pub const EVALUATION_SCHEMA_VERSION: &str = "monitter-routing-evaluation-v2";
pub const MAX_CORPUS_BYTES: u64 = 2 * 1024 * 1024;
pub const MAX_OUTCOMES_BYTES: u64 = 8 * 1024 * 1024;
const FIXED_BASELINE_POLICY_VERSION: &str = "fixed-strong-baseline-v1";
const NOT_APPLICABLE: &str = "not_applicable";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Provenance {
    Synthetic,
    Measured,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LabeledCase {
    pub id: String,
    pub label: String,
    pub prompt: String,
    pub expected_minimum_tier: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LabeledCorpus {
    pub schema_version: String,
    pub provenance: Provenance,
    pub question_schema_version: String,
    pub cases: Vec<LabeledCase>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TestEvidence {
    pub executed: u32,
    pub passed: u32,
    pub failed: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RunOutcome {
    pub schema_version: String,
    pub provenance: Provenance,
    pub run_id: String,
    pub case_id: String,
    pub repeat_id: String,
    /// Stable comparison group, such as `fixed_strong` or `jev_route`.
    pub route_id: String,
    pub routing_policy_version: String,
    pub question_schema_version: String,
    /// Opaque caller-supplied IDs, never transcript contents.
    pub evidence_ids: Vec<String>,
    /// Present for classifier-routed records; replayed with native policy.
    pub classifier_decision: Option<RoutingDecision>,
    pub classifier_model: Option<String>,
    pub classifier_model_version: Option<String>,
    pub provider: String,
    pub provider_version: Option<String>,
    pub model: String,
    pub model_version: Option<String>,
    /// Optional observed quality fields; null means not measured.
    pub task_completed: Option<bool>,
    pub tests: Option<TestEvidence>,
    pub cost_usd: Option<f64>,
    pub latency_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MetricSummary {
    pub route_id: String,
    pub outcome_count: usize,
    pub completion_observation_count: usize,
    pub successful_completion_count: Option<usize>,
    pub completion_rate: Option<f64>,
    pub test_observation_count: usize,
    pub tests_passed: Option<u64>,
    pub tests_executed: Option<u64>,
    pub test_pass_rate: Option<f64>,
    pub cost_observation_count: usize,
    pub mean_cost_usd: Option<f64>,
    pub latency_observation_count: usize,
    pub mean_latency_ms: Option<f64>,
    pub provider_models: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RouteReplay {
    pub run_id: String,
    pub case_id: String,
    pub repeat_id: String,
    pub route_id: String,
    pub routing_policy_version: String,
    pub question_schema_version: String,
    pub classifier_model: String,
    pub classifier_model_version: Option<String>,
    pub classifier_decision: RoutingDecision,
    pub expected_minimum_tier: String,
    pub confidence: f32,
    pub threshold: f32,
    pub basis: JevAutoRouteBasis,
    pub auto_route_eligible: bool,
    /// None means policy abstained/recommended without selecting a model tier.
    pub applied_route: Option<String>,
    pub tier_coverage: TierCoverage,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TierCoverage {
    Abstained,
    Underallocated,
    MeetsMinimum,
    AboveMinimum,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TierCoverageSummary {
    pub decision_count: usize,
    pub eligible_count: usize,
    pub abstained_count: usize,
    pub underallocated_count: usize,
    pub meets_minimum_count: usize,
    pub above_minimum_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PairedComparison {
    pub left_route_id: String,
    pub right_route_id: String,
    pub matched_pair_count: usize,
    pub left_unmatched_count: usize,
    pub right_unmatched_count: usize,
    pub completion_observation_pair_count: usize,
    /// Mean of (left - right), using only matched pairs with both values.
    pub mean_completion_delta: Option<f64>,
    pub test_observation_pair_count: usize,
    pub mean_test_pass_rate_delta: Option<f64>,
    pub cost_observation_pair_count: usize,
    pub mean_cost_delta_usd: Option<f64>,
    pub latency_observation_pair_count: usize,
    pub mean_latency_delta_ms: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EvaluationReport {
    pub schema_version: String,
    pub corpus_provenance: Provenance,
    pub outcome_provenance: Provenance,
    pub case_count: usize,
    pub outcome_count: usize,
    pub metrics: Vec<MetricSummary>,
    pub route_replays: Vec<RouteReplay>,
    pub tier_coverage: TierCoverageSummary,
    pub paired_comparisons: Vec<PairedComparison>,
    pub paired_comparison_scope: String,
}

pub fn bundled_corpus() -> Result<LabeledCorpus, String> {
    let corpus: LabeledCorpus =
        serde_json::from_str(include_str!("../../evaluation/jev-routing-corpus-v2.json"))
            .map_err(|error| format!("Could not parse bundled evaluation corpus: {error}"))?;
    validate_corpus(&corpus)?;
    Ok(corpus)
}

pub fn bundled_outcomes(corpus: &LabeledCorpus) -> Result<Vec<RunOutcome>, String> {
    let outcomes: Vec<RunOutcome> =
        serde_json::from_str(include_str!("../../evaluation/jev-routing-replay-v2.json"))
            .map_err(|error| format!("Could not parse bundled evaluation replay: {error}"))?;
    validate_outcomes(&outcomes, corpus)?;
    Ok(outcomes)
}

pub fn load_corpus(path: &Path) -> Result<LabeledCorpus, String> {
    let bytes = read_bounded(path, MAX_CORPUS_BYTES, "evaluation corpus")?;
    let corpus: LabeledCorpus = serde_json::from_slice(&bytes)
        .map_err(|error| format!("Could not parse evaluation corpus: {error}"))?;
    validate_corpus(&corpus)?;
    Ok(corpus)
}

pub fn load_outcomes(path: &Path, corpus: &LabeledCorpus) -> Result<Vec<RunOutcome>, String> {
    let bytes = read_bounded(path, MAX_OUTCOMES_BYTES, "run outcomes")?;
    let outcomes: Vec<RunOutcome> = serde_json::from_slice(&bytes)
        .map_err(|error| format!("Could not parse run outcomes: {error}"))?;
    validate_outcomes(&outcomes, corpus)?;
    Ok(outcomes)
}

fn read_bounded(path: &Path, limit: u64, label: &str) -> Result<Vec<u8>, String> {
    let metadata =
        std::fs::metadata(path).map_err(|error| format!("Could not inspect {label}: {error}"))?;
    if metadata.len() > limit {
        return Err(format!("{label} exceeds the {limit}-byte import limit."));
    }
    let file = File::open(path).map_err(|error| format!("Could not open {label}: {error}"))?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Could not read {label}: {error}"))?;
    if bytes.len() as u64 > limit {
        return Err(format!("{label} exceeds the {limit}-byte import limit."));
    }
    Ok(bytes)
}

pub fn report(corpus: &LabeledCorpus, outcomes: &[RunOutcome]) -> Result<EvaluationReport, String> {
    validate_corpus(corpus)?;
    validate_outcomes(outcomes, corpus)?;
    let outcome_provenance = outcomes
        .first()
        .map(|item| item.provenance)
        .unwrap_or(corpus.provenance);
    if outcomes
        .iter()
        .any(|item| item.provenance != outcome_provenance)
    {
        return Err("An evaluation input cannot mix measured and synthetic run outcomes.".into());
    }

    let cases: BTreeMap<_, _> = corpus
        .cases
        .iter()
        .map(|case| (case.id.as_str(), case))
        .collect();
    let mut grouped: BTreeMap<&str, Vec<&RunOutcome>> = BTreeMap::new();
    for outcome in outcomes {
        grouped.entry(&outcome.route_id).or_default().push(outcome);
    }
    let metrics = grouped
        .iter()
        .map(|(route_id, records)| summarize(route_id, records))
        .collect();
    let route_replays = outcomes
        .iter()
        .filter_map(|outcome| {
            outcome
                .classifier_decision
                .as_ref()
                .map(|decision| replay_outcome(outcome, decision, cases[outcome.case_id.as_str()]))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let tier_coverage = summarize_tier_coverage(&route_replays);
    let paired_comparisons = paired_comparisons(&grouped);

    Ok(EvaluationReport {
        schema_version: EVALUATION_SCHEMA_VERSION.into(),
        corpus_provenance: corpus.provenance,
        outcome_provenance,
        case_count: corpus.cases.len(),
        outcome_count: outcomes.len(),
        metrics,
        route_replays,
        tier_coverage,
        paired_comparisons,
        paired_comparison_scope: "Pairs match only on caseId + repeatId; unmatched observations are shown separately and excluded from every paired mean. Deltas are left route minus right route.".into(),
    })
}

pub fn validate_corpus(corpus: &LabeledCorpus) -> Result<(), String> {
    if corpus.schema_version != EVALUATION_SCHEMA_VERSION {
        return Err("Unsupported evaluation corpus schemaVersion.".into());
    }
    if corpus.question_schema_version != ROUTING_QUESTION_SCHEMA_VERSION {
        return Err("Unsupported corpus questionSchemaVersion.".into());
    }
    if corpus.cases.is_empty() || corpus.cases.len() > 500 {
        return Err("Evaluation corpus must contain between 1 and 500 cases.".into());
    }
    let mut ids = BTreeSet::new();
    for case in &corpus.cases {
        bounded_text(&case.id, 96, "case id")?;
        bounded_text(&case.label, 128, "case label")?;
        bounded_text(&case.prompt, 4096, "case prompt")?;
        bounded_text(&case.expected_minimum_tier, 32, "expected minimum tier")?;
        if tier_rank(&case.expected_minimum_tier).is_none() {
            return Err(format!(
                "Evaluation case '{}' has an unknown expected minimum tier.",
                case.id
            ));
        }
        if !ids.insert(case.id.as_str()) {
            return Err(format!("Duplicate evaluation case id '{}'.", case.id));
        }
    }
    Ok(())
}

pub fn validate_outcomes(outcomes: &[RunOutcome], corpus: &LabeledCorpus) -> Result<(), String> {
    if outcomes.len() > 10_000 {
        return Err("At most 10,000 run outcomes may be imported at once.".into());
    }
    let case_ids: BTreeSet<_> = corpus.cases.iter().map(|case| case.id.as_str()).collect();
    let mut run_ids = BTreeSet::new();
    let mut comparison_keys = BTreeSet::new();
    for outcome in outcomes {
        if outcome.schema_version != EVALUATION_SCHEMA_VERSION {
            return Err("Unsupported outcome schemaVersion.".into());
        }
        if !case_ids.contains(outcome.case_id.as_str()) {
            return Err(format!(
                "Outcome '{}' references unknown case '{}'.",
                outcome.run_id, outcome.case_id
            ));
        }
        for (value, limit, name) in [
            (&outcome.run_id, 128, "run id"),
            (&outcome.case_id, 96, "case id"),
            (&outcome.repeat_id, 96, "repeat id"),
            (&outcome.route_id, 96, "route id"),
            (
                &outcome.routing_policy_version,
                128,
                "routing policy version",
            ),
            (
                &outcome.question_schema_version,
                128,
                "question schema version",
            ),
            (&outcome.provider, 128, "provider"),
            (&outcome.model, 160, "model"),
        ] {
            bounded_text(value, limit, name)?;
        }
        if outcome.evidence_ids.is_empty() || outcome.evidence_ids.len() > 16 {
            return Err(format!(
                "Outcome '{}' must include 1 to 16 evidence IDs.",
                outcome.run_id
            ));
        }
        for evidence_id in &outcome.evidence_ids {
            bounded_text(evidence_id, 160, "evidence id")?;
        }
        for (value, limit, name) in [
            (&outcome.provider_version, 160, "provider version"),
            (&outcome.model_version, 160, "model version"),
            (&outcome.classifier_model, 160, "classifier model"),
            (
                &outcome.classifier_model_version,
                160,
                "classifier model version",
            ),
        ] {
            if let Some(value) = value {
                bounded_text(value, limit, name)?;
            }
        }
        if !run_ids.insert(outcome.run_id.as_str()) {
            return Err(format!("Duplicate run id '{}'.", outcome.run_id));
        }
        if !comparison_keys.insert((
            outcome.route_id.as_str(),
            outcome.case_id.as_str(),
            outcome.repeat_id.as_str(),
        )) {
            return Err(format!(
                "Duplicate route/case/repeat outcome for '{}'.",
                outcome.run_id
            ));
        }
        match outcome.classifier_decision.as_ref() {
            Some(decision) => {
                if outcome.routing_policy_version != ROUTING_POLICY_VERSION {
                    return Err(format!(
                        "Outcome '{}' uses an unsupported routingPolicyVersion.",
                        outcome.run_id
                    ));
                }
                if outcome.question_schema_version != ROUTING_QUESTION_SCHEMA_VERSION {
                    return Err(format!(
                        "Outcome '{}' uses an unsupported questionSchemaVersion.",
                        outcome.run_id
                    ));
                }
                if outcome.route_id != "jev_route" {
                    return Err(format!(
                        "Outcome '{}' includes a classifier decision outside jev_route.",
                        outcome.run_id
                    ));
                }
                if outcome.classifier_model.is_none() || outcome.classifier_model_version.is_none()
                {
                    return Err(format!(
                        "Outcome '{}' must identify classifier model and version.",
                        outcome.run_id
                    ));
                }
                validate_decision(decision)
                    .map_err(|message| format!("Outcome '{}': {message}", outcome.run_id))?;
            }
            None => {
                if outcome.routing_policy_version != FIXED_BASELINE_POLICY_VERSION
                    || outcome.question_schema_version != NOT_APPLICABLE
                {
                    return Err(format!("Outcome '{}' must identify a supported non-classifier route policy and question version.", outcome.run_id));
                }
                if outcome.route_id == "jev_route" {
                    return Err(format!(
                        "Outcome '{}' is missing its captured classifier decision.",
                        outcome.run_id
                    ));
                }
            }
        }
        if outcome
            .cost_usd
            .is_some_and(|value| !value.is_finite() || !(0.0..=10_000.0).contains(&value))
        {
            return Err(format!(
                "Outcome '{}' has an invalid costUsd.",
                outcome.run_id
            ));
        }
        if outcome.latency_ms.is_some_and(|value| value > 172_800_000) {
            return Err(format!(
                "Outcome '{}' latencyMs exceeds 48 hours.",
                outcome.run_id
            ));
        }
        if let Some(tests) = &outcome.tests {
            if tests.executed > 100_000
                || tests.passed > tests.executed
                || tests.failed > tests.executed
                || tests.passed > tests.executed - tests.failed
            {
                return Err(format!(
                    "Outcome '{}' has inconsistent test evidence.",
                    outcome.run_id
                ));
            }
        }
    }
    Ok(())
}

fn validate_decision(decision: &RoutingDecision) -> Result<(), String> {
    let valid_probability = |value: f32| value.is_finite() && (0.0..=1.0).contains(&value);
    if !valid_probability(decision.confidence) {
        return Err("classifier confidence must be in 0..=1.".into());
    }
    let probabilities = decision
        .question_probabilities
        .as_ref()
        .ok_or("classifier decision is missing question probability distributions.")?;
    let confidences = decision
        .question_confidences
        .as_ref()
        .ok_or("classifier decision is missing question confidences.")?;
    let questions = [
        (
            "task_kind",
            &probabilities.task_kind,
            &confidences.task_kind,
            serde_json::to_value(decision.task_kind).unwrap(),
            &[
                "answer",
                "investigate",
                "localized_edit",
                "bug_fix",
                "refactor",
                "architecture",
                "production_sensitive",
            ][..],
        ),
        (
            "model_tier",
            &probabilities.model_tier,
            &confidences.model_tier,
            serde_json::to_value(decision.model_tier).unwrap(),
            &["fast", "balanced", "strong", "frontier"][..],
        ),
        (
            "reasoning_level",
            &probabilities.reasoning_level,
            &confidences.reasoning_level,
            serde_json::to_value(decision.reasoning_level).unwrap(),
            &["low", "medium", "high", "xhigh"][..],
        ),
        (
            "execution_mode",
            &probabilities.execution_mode,
            &confidences.execution_mode,
            serde_json::to_value(decision.execution_mode).unwrap(),
            &["answer", "inspect", "edit"][..],
        ),
        (
            "permission_tier",
            &probabilities.permission_tier,
            &confidences.permission_tier,
            serde_json::to_value(decision.permission_tier).unwrap(),
            &[
                "read_only",
                "workspace_write",
                "shell_and_tests",
                "human_review_required",
            ][..],
        ),
    ];
    let mut lowest_confidence = 1.0_f32;
    for (name, distribution, confidence, selected, expected_options) in questions {
        lowest_confidence = lowest_confidence.min(*confidence);
        if !valid_probability(*confidence) {
            return Err(format!("{name} confidence must be in 0..=1."));
        }
        if distribution.len() != expected_options.len()
            || expected_options
                .iter()
                .any(|key| !distribution.contains_key(*key))
        {
            return Err(format!(
                "{name} probability distribution must include every known option."
            ));
        }
        let selected = selected
            .as_str()
            .ok_or_else(|| format!("{name} choice could not be serialized."))?;
        let selected_probability = distribution
            .get(selected)
            .ok_or_else(|| format!("{name} distribution is missing selected choice."))?;
        if distribution.is_empty()
            || distribution.len() > 16
            || !valid_probability(*selected_probability)
        {
            return Err(format!("{name} distribution has invalid entries."));
        }
        let mut sum = 0.0_f32;
        for probability in distribution.values() {
            if !valid_probability(*probability) {
                return Err(format!("{name} probability is outside 0..=1."));
            }
            sum += probability;
        }
        if (sum - 1.0).abs() > 0.01 || (selected_probability - confidence).abs() > 0.01 {
            return Err(format!(
                "{name} probabilities do not match the selected confidence or sum to one."
            ));
        }
    }
    if (lowest_confidence - decision.confidence).abs() > 0.01 {
        return Err("classifier confidence does not match the lowest question confidence.".into());
    }
    Ok(())
}

fn replay_outcome(
    outcome: &RunOutcome,
    decision: &RoutingDecision,
    case: &LabeledCase,
) -> Result<RouteReplay, String> {
    let policy = replay_auto_route_policy(decision).ok_or_else(|| {
        format!(
            "Outcome '{}' decision has no native auto-route policy inputs.",
            outcome.run_id
        )
    })?;
    let (applied_route, tier_coverage) = if !policy.eligible {
        (None, TierCoverage::Abstained)
    } else {
        let tier = serde_json::to_value(decision.model_tier)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or_else(|| format!("{:?}", decision.model_tier).to_lowercase());
        let applied_rank = tier_rank(&tier).expect("serialized ModelTier is known");
        let expected_rank = tier_rank(&case.expected_minimum_tier).expect("corpus validated");
        let coverage = match applied_rank.cmp(&expected_rank) {
            std::cmp::Ordering::Less => TierCoverage::Underallocated,
            std::cmp::Ordering::Equal => TierCoverage::MeetsMinimum,
            std::cmp::Ordering::Greater => TierCoverage::AboveMinimum,
        };
        (Some(tier), coverage)
    };
    Ok(RouteReplay {
        run_id: outcome.run_id.clone(),
        case_id: outcome.case_id.clone(),
        repeat_id: outcome.repeat_id.clone(),
        route_id: outcome.route_id.clone(),
        routing_policy_version: outcome.routing_policy_version.clone(),
        question_schema_version: outcome.question_schema_version.clone(),
        classifier_model: outcome.classifier_model.clone().unwrap_or_default(),
        classifier_model_version: outcome.classifier_model_version.clone(),
        classifier_decision: decision.clone(),
        expected_minimum_tier: case.expected_minimum_tier.clone(),
        confidence: policy.confidence,
        threshold: policy.threshold,
        basis: policy.basis,
        auto_route_eligible: policy.eligible,
        applied_route,
        tier_coverage,
    })
}

fn summarize_tier_coverage(replays: &[RouteReplay]) -> TierCoverageSummary {
    let count = |coverage| {
        replays
            .iter()
            .filter(|entry| entry.tier_coverage == coverage)
            .count()
    };
    TierCoverageSummary {
        decision_count: replays.len(),
        eligible_count: replays
            .iter()
            .filter(|entry| entry.auto_route_eligible)
            .count(),
        abstained_count: count(TierCoverage::Abstained),
        underallocated_count: count(TierCoverage::Underallocated),
        meets_minimum_count: count(TierCoverage::MeetsMinimum),
        above_minimum_count: count(TierCoverage::AboveMinimum),
    }
}

type PairKey<'a> = (&'a str, &'a str);

fn paired_comparisons<'a>(
    grouped: &BTreeMap<&'a str, Vec<&'a RunOutcome>>,
) -> Vec<PairedComparison> {
    let routes: Vec<_> = grouped.keys().copied().collect();
    let mut comparisons = Vec::new();
    for left_index in 0..routes.len() {
        for right_index in (left_index + 1)..routes.len() {
            let left_route_id = routes[left_index];
            let right_route_id = routes[right_index];
            let left = grouped[left_route_id]
                .iter()
                .map(|item| ((item.case_id.as_str(), item.repeat_id.as_str()), *item))
                .collect::<BTreeMap<PairKey<'_>, _>>();
            let right = grouped[right_route_id]
                .iter()
                .map(|item| ((item.case_id.as_str(), item.repeat_id.as_str()), *item))
                .collect::<BTreeMap<PairKey<'_>, _>>();
            let keys: BTreeSet<_> = left.keys().chain(right.keys()).copied().collect();
            let mut pairs = Vec::new();
            let (mut left_unmatched_count, mut right_unmatched_count) = (0, 0);
            for key in keys {
                match (left.get(&key), right.get(&key)) {
                    (Some(l), Some(r)) => pairs.push((*l, *r)),
                    (Some(_), None) => left_unmatched_count += 1,
                    (None, Some(_)) => right_unmatched_count += 1,
                    _ => unreachable!(),
                }
            }
            comparisons.push(summarize_pairs(
                left_route_id,
                right_route_id,
                pairs,
                left_unmatched_count,
                right_unmatched_count,
            ));
        }
    }
    comparisons
}

fn summarize_pairs(
    left_route_id: &str,
    right_route_id: &str,
    pairs: Vec<(&RunOutcome, &RunOutcome)>,
    left_unmatched_count: usize,
    right_unmatched_count: usize,
) -> PairedComparison {
    let completion_deltas: Vec<_> = pairs
        .iter()
        .filter_map(|(l, r)| {
            Some(f64::from(l.task_completed? as u8) - f64::from(r.task_completed? as u8))
        })
        .collect();
    let test_deltas: Vec<_> = pairs
        .iter()
        .filter_map(|(l, r)| {
            Some(test_pass_rate(l.tests.as_ref()?)? - test_pass_rate(r.tests.as_ref()?)?)
        })
        .collect();
    let cost_deltas: Vec<_> = pairs
        .iter()
        .filter_map(|(l, r)| Some(l.cost_usd? - r.cost_usd?))
        .collect();
    let latency_deltas: Vec<_> = pairs
        .iter()
        .filter_map(|(l, r)| Some(l.latency_ms? as f64 - r.latency_ms? as f64))
        .collect();
    PairedComparison {
        left_route_id: left_route_id.into(),
        right_route_id: right_route_id.into(),
        matched_pair_count: pairs.len(),
        left_unmatched_count,
        right_unmatched_count,
        completion_observation_pair_count: completion_deltas.len(),
        mean_completion_delta: mean(&completion_deltas),
        test_observation_pair_count: test_deltas.len(),
        mean_test_pass_rate_delta: mean(&test_deltas),
        cost_observation_pair_count: cost_deltas.len(),
        mean_cost_delta_usd: mean(&cost_deltas),
        latency_observation_pair_count: latency_deltas.len(),
        mean_latency_delta_ms: mean(&latency_deltas),
    }
}

fn test_pass_rate(tests: &TestEvidence) -> Option<f64> {
    (tests.executed > 0).then(|| tests.passed as f64 / tests.executed as f64)
}

fn summarize(route_id: &str, records: &[&RunOutcome]) -> MetricSummary {
    let completed: Vec<_> = records
        .iter()
        .filter_map(|record| record.task_completed)
        .collect();
    let test_records: Vec<_> = records
        .iter()
        .filter_map(|record| record.tests.as_ref())
        .collect();
    let tests_passed: u64 = test_records
        .iter()
        .map(|tests| u64::from(tests.passed))
        .sum();
    let tests_executed: u64 = test_records
        .iter()
        .map(|tests| u64::from(tests.executed))
        .sum();
    let costs: Vec<_> = records
        .iter()
        .filter_map(|record| record.cost_usd)
        .collect();
    let latencies: Vec<_> = records
        .iter()
        .filter_map(|record| record.latency_ms)
        .collect();
    let provider_models: BTreeSet<_> = records
        .iter()
        .map(|record| {
            format!(
                "{}@{}/{}@{}",
                record.provider,
                record
                    .provider_version
                    .as_deref()
                    .unwrap_or("version-unspecified"),
                record.model,
                record
                    .model_version
                    .as_deref()
                    .unwrap_or("version-unspecified")
            )
        })
        .collect();
    let successful = completed.iter().filter(|value| **value).count();
    MetricSummary {
        route_id: route_id.into(),
        outcome_count: records.len(),
        completion_observation_count: completed.len(),
        successful_completion_count: (!completed.is_empty()).then_some(successful),
        completion_rate: (!completed.is_empty())
            .then(|| successful as f64 / completed.len() as f64),
        test_observation_count: test_records.len(),
        tests_passed: (!test_records.is_empty()).then_some(tests_passed),
        tests_executed: (!test_records.is_empty()).then_some(tests_executed),
        test_pass_rate: (tests_executed > 0).then(|| tests_passed as f64 / tests_executed as f64),
        cost_observation_count: costs.len(),
        mean_cost_usd: mean(&costs),
        latency_observation_count: latencies.len(),
        mean_latency_ms: mean(
            &latencies
                .iter()
                .map(|value| *value as f64)
                .collect::<Vec<_>>(),
        ),
        provider_models: provider_models.into_iter().collect(),
    }
}

fn mean(values: &[f64]) -> Option<f64> {
    (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
}

fn tier_rank(tier: &str) -> Option<u8> {
    match tier {
        "fast" => Some(0),
        "balanced" => Some(1),
        "strong" => Some(2),
        "frontier" => Some(3),
        _ => None,
    }
}

fn bounded_text(value: &str, max_bytes: usize, name: &str) -> Result<(), String> {
    if value.trim().is_empty() || value.len() > max_bytes {
        return Err(format!(
            "Evaluation {name} must be non-empty and at most {max_bytes} bytes."
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corpus() -> LabeledCorpus {
        LabeledCorpus {
            schema_version: EVALUATION_SCHEMA_VERSION.into(),
            provenance: Provenance::Synthetic,
            question_schema_version: ROUTING_QUESTION_SCHEMA_VERSION.into(),
            cases: vec![LabeledCase {
                id: "case-1".into(),
                label: "explanation".into(),
                prompt: "Explain a small behavior.".into(),
                expected_minimum_tier: "balanced".into(),
            }],
        }
    }

    fn outcome(run_id: &str, route: &str, repeat: &str) -> RunOutcome {
        let jev = route == "jev_route";
        RunOutcome {
            schema_version: EVALUATION_SCHEMA_VERSION.into(),
            provenance: Provenance::Synthetic,
            run_id: run_id.into(),
            case_id: "case-1".into(),
            repeat_id: repeat.into(),
            route_id: route.into(),
            routing_policy_version: if jev {
                ROUTING_POLICY_VERSION
            } else {
                FIXED_BASELINE_POLICY_VERSION
            }
            .into(),
            question_schema_version: if jev {
                ROUTING_QUESTION_SCHEMA_VERSION
            } else {
                NOT_APPLICABLE
            }
            .into(),
            evidence_ids: vec![format!("fixture:{run_id}")],
            classifier_decision: jev.then(fixture_decision),
            classifier_model: jev.then(|| "synthetic-classifier".into()),
            classifier_model_version: jev.then(|| "fixture-v1".into()),
            provider: "synthetic_fixture".into(),
            provider_version: Some("fixture-v1".into()),
            model: "strong".into(),
            model_version: Some("fixture-model-v1".into()),
            task_completed: None,
            tests: None,
            cost_usd: None,
            latency_ms: None,
        }
    }

    fn fixture_decision() -> RoutingDecision {
        use super::super::{
            ExecutionMode, ModelTier, PermissionTier, ReasoningLevel, RoutingQuestionConfidences,
            RoutingQuestionProbabilities, TaskKind,
        };
        let probs = |choices: &[(&str, f32)]| {
            choices
                .iter()
                .map(|(key, value)| ((*key).to_owned(), *value))
                .collect()
        };
        RoutingDecision {
            task_kind: TaskKind::Answer,
            model_tier: ModelTier::Balanced,
            reasoning_level: ReasoningLevel::Medium,
            execution_mode: ExecutionMode::Answer,
            permission_tier: PermissionTier::ReadOnly,
            confidence: 0.80,
            question_confidences: Some(RoutingQuestionConfidences {
                task_kind: 0.80,
                model_tier: 0.80,
                reasoning_level: 0.80,
                execution_mode: 0.80,
                permission_tier: 0.80,
            }),
            question_probabilities: Some(RoutingQuestionProbabilities {
                task_kind: probs(&[
                    ("answer", 0.80),
                    ("investigate", 0.05),
                    ("localized_edit", 0.03),
                    ("bug_fix", 0.03),
                    ("refactor", 0.03),
                    ("architecture", 0.03),
                    ("production_sensitive", 0.03),
                ]),
                model_tier: probs(&[
                    ("fast", 0.10),
                    ("balanced", 0.80),
                    ("strong", 0.05),
                    ("frontier", 0.05),
                ]),
                reasoning_level: probs(&[
                    ("low", 0.10),
                    ("medium", 0.80),
                    ("high", 0.05),
                    ("xhigh", 0.05),
                ]),
                execution_mode: probs(&[("answer", 0.80), ("inspect", 0.10), ("edit", 0.10)]),
                permission_tier: probs(&[
                    ("read_only", 0.80),
                    ("workspace_write", 0.10),
                    ("shell_and_tests", 0.05),
                    ("human_review_required", 0.05),
                ]),
            }),
            rationale: "synthetic test decision".into(),
            escalation_conditions: vec![],
        }
    }

    #[test]
    fn evaluation_replays_production_policy_and_uses_expected_minimum_tier() {
        let corpus = corpus();
        let report = report(&corpus, &[outcome("run-1", "jev_route", "repeat-1")]).unwrap();
        assert_eq!(report.route_replays.len(), 1);
        let replay = &report.route_replays[0];
        assert_eq!(replay.applied_route.as_deref(), Some("balanced"));
        assert_eq!(replay.tier_coverage, TierCoverage::MeetsMinimum);
        assert_eq!(replay.threshold, 0.30);
        assert_eq!(report.tier_coverage.meets_minimum_count, 1);
    }

    #[test]
    fn missing_quality_cost_and_latency_remain_missing_and_pairs_need_same_repeat() {
        let corpus = corpus();
        let report = report(
            &corpus,
            &[
                outcome("jev-1", "jev_route", "r1"),
                outcome("base-1", "fixed_strong", "r1"),
                outcome("base-2", "fixed_strong", "r2"),
            ],
        )
        .unwrap();
        let jev = report
            .metrics
            .iter()
            .find(|row| row.route_id == "jev_route")
            .unwrap();
        assert_eq!(jev.completion_observation_count, 0);
        assert_eq!(jev.successful_completion_count, None);
        assert_eq!(jev.completion_rate, None);
        assert_eq!(jev.mean_cost_usd, None);
        assert_eq!(jev.mean_latency_ms, None);
        let paired = report
            .paired_comparisons
            .iter()
            .find(|row| row.left_route_id == "fixed_strong")
            .unwrap();
        assert_eq!(paired.matched_pair_count, 1);
        assert_eq!(paired.left_unmatched_count, 1);
        assert_eq!(paired.right_unmatched_count, 0);
        assert_eq!(paired.mean_cost_delta_usd, None);
    }

    #[test]
    fn invalid_policy_question_case_and_duplicate_pair_are_rejected() {
        let corpus = corpus();
        let mut run = outcome("run-1", "jev_route", "r1");
        run.routing_policy_version = "future-policy".into();
        assert!(validate_outcomes(&[run.clone()], &corpus)
            .unwrap_err()
            .contains("routingPolicyVersion"));
        run = outcome("run-1", "jev_route", "r1");
        run.case_id = "unknown".into();
        assert!(validate_outcomes(&[run.clone()], &corpus)
            .unwrap_err()
            .contains("unknown case"));
        run = outcome("run-1", "jev_route", "r1");
        let mut duplicate = run.clone();
        duplicate.run_id = "run-2".into();
        assert!(validate_outcomes(&[run, duplicate], &corpus)
            .unwrap_err()
            .contains("Duplicate route/case/repeat"));
    }

    #[test]
    fn malformed_distribution_and_unknown_protocol_versions_are_rejected() {
        let corpus = corpus();
        let mut run = outcome("run-1", "jev_route", "r1");
        run.schema_version = "future-evaluation-v9".into();
        assert!(validate_outcomes(&[run.clone()], &corpus)
            .unwrap_err()
            .contains("schemaVersion"));

        run = outcome("run-1", "jev_route", "r1");
        run.question_schema_version = "future-questions".into();
        assert!(validate_outcomes(&[run.clone()], &corpus)
            .unwrap_err()
            .contains("questionSchemaVersion"));

        run = outcome("run-1", "jev_route", "r1");
        let decision = run.classifier_decision.as_mut().unwrap();
        decision
            .question_probabilities
            .as_mut()
            .unwrap()
            .model_tier
            .insert("fast".into(), f32::NAN);
        assert!(validate_outcomes(&[run], &corpus)
            .unwrap_err()
            .contains("probability is outside"));
    }

    #[test]
    fn imported_files_are_bounded_before_parsing() {
        let path = std::env::temp_dir().join(format!(
            "monitter-eval-oversized-{}.json",
            std::process::id()
        ));
        std::fs::File::create(&path)
            .unwrap()
            .set_len(MAX_CORPUS_BYTES + 1)
            .unwrap();
        let error = load_corpus(&path).unwrap_err();
        let _ = std::fs::remove_file(&path);
        assert!(error.contains("import limit"));

        let outcomes_path = std::env::temp_dir().join(format!(
            "monitter-eval-outcomes-oversized-{}.json",
            std::process::id()
        ));
        std::fs::File::create(&outcomes_path)
            .unwrap()
            .set_len(MAX_OUTCOMES_BYTES + 1)
            .unwrap();
        let error = load_outcomes(&outcomes_path, &corpus()).unwrap_err();
        let _ = std::fs::remove_file(&outcomes_path);
        assert!(error.contains("import limit"));
    }

    #[test]
    fn bundled_fixture_is_synthetic_and_contains_no_claimed_measurements() {
        let corpus = bundled_corpus().unwrap();
        let outcomes = bundled_outcomes(&corpus).unwrap();
        assert_eq!(corpus.provenance, Provenance::Synthetic);
        assert_eq!(corpus.cases.len(), 12);
        assert!(
            outcomes
                .iter()
                .filter(|item| item.classifier_decision.is_some())
                .count()
                >= 12
        );
        assert!(outcomes.iter().all(|item| item.task_completed.is_none()
            && item.tests.is_none()
            && item.cost_usd.is_none()
            && item.latency_ms.is_none()));
        let report = report(&corpus, &outcomes).unwrap();
        assert_eq!(report.outcome_provenance, Provenance::Synthetic);
        assert_eq!(report.tier_coverage.decision_count, 12);
        let underallocated = report
            .route_replays
            .iter()
            .find(|entry| entry.case_id == "refactor-api-boundary")
            .unwrap();
        assert_eq!(underallocated.tier_coverage, TierCoverage::Underallocated);
        assert_eq!(underallocated.applied_route.as_deref(), Some("balanced"));
        let abstained = report
            .route_replays
            .iter()
            .find(|entry| entry.case_id == "sensitive-billing")
            .unwrap();
        assert_eq!(abstained.basis, JevAutoRouteBasis::TaskKind);
        assert_eq!(abstained.tier_coverage, TierCoverage::Abstained);
        assert!(abstained.applied_route.is_none());
    }
}
