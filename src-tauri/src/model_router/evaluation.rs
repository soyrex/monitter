//! Offline, provenance-labelled evaluation inputs and summaries.
//!
//! This module never opens a provider or classifier. Synthetic replay data is
//! useful for exercising the evaluator, but it is kept visibly separate from
//! caller-supplied observations.

use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::Path};

pub const EVALUATION_SCHEMA_VERSION: &str = "monitter-routing-evaluation-v1";

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
    /// Stable comparison group, such as `fixed_strong` or `jev_route`.
    pub route_id: String,
    pub provider: String,
    pub provider_version: Option<String>,
    pub model: String,
    pub model_version: Option<String>,
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
    pub completed_count: usize,
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
pub struct EvaluationReport {
    pub schema_version: String,
    pub corpus_provenance: Provenance,
    pub outcome_provenance: Provenance,
    pub case_count: usize,
    pub outcome_count: usize,
    pub metrics: Vec<MetricSummary>,
}

pub fn bundled_corpus() -> Result<LabeledCorpus, String> {
    let corpus: LabeledCorpus =
        serde_json::from_str(include_str!("../../evaluation/jev-routing-corpus-v1.json"))
            .map_err(|error| format!("Could not parse bundled evaluation corpus: {error}"))?;
    validate_corpus(&corpus)?;
    Ok(corpus)
}

pub fn bundled_outcomes(corpus: &LabeledCorpus) -> Result<Vec<RunOutcome>, String> {
    let outcomes: Vec<RunOutcome> =
        serde_json::from_str(include_str!("../../evaluation/jev-routing-replay-v1.json"))
            .map_err(|error| format!("Could not parse bundled evaluation replay: {error}"))?;
    validate_outcomes(&outcomes, corpus)?;
    Ok(outcomes)
}

pub fn load_corpus(path: &Path) -> Result<LabeledCorpus, String> {
    let bytes =
        fs::read(path).map_err(|error| format!("Could not read evaluation corpus: {error}"))?;
    let corpus: LabeledCorpus = serde_json::from_slice(&bytes)
        .map_err(|error| format!("Could not parse evaluation corpus: {error}"))?;
    validate_corpus(&corpus)?;
    Ok(corpus)
}

pub fn load_outcomes(path: &Path, corpus: &LabeledCorpus) -> Result<Vec<RunOutcome>, String> {
    let bytes = fs::read(path).map_err(|error| format!("Could not read run outcomes: {error}"))?;
    let outcomes: Vec<RunOutcome> = serde_json::from_slice(&bytes)
        .map_err(|error| format!("Could not parse run outcomes: {error}"))?;
    validate_outcomes(&outcomes, corpus)?;
    Ok(outcomes)
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
    let mut grouped: BTreeMap<&str, Vec<&RunOutcome>> = BTreeMap::new();
    for outcome in outcomes {
        grouped.entry(&outcome.route_id).or_default().push(outcome);
    }
    let metrics = grouped
        .into_iter()
        .map(|(route_id, records)| summarize(route_id, &records))
        .collect();
    Ok(EvaluationReport {
        schema_version: EVALUATION_SCHEMA_VERSION.into(),
        corpus_provenance: corpus.provenance,
        outcome_provenance,
        case_count: corpus.cases.len(),
        outcome_count: outcomes.len(),
        metrics,
    })
}

pub fn validate_corpus(corpus: &LabeledCorpus) -> Result<(), String> {
    if corpus.schema_version != EVALUATION_SCHEMA_VERSION {
        return Err("Unsupported evaluation corpus schemaVersion.".into());
    }
    if corpus.cases.is_empty() || corpus.cases.len() > 500 {
        return Err("Evaluation corpus must contain between 1 and 500 cases.".into());
    }
    let mut ids = std::collections::BTreeSet::new();
    for case in &corpus.cases {
        bounded_text(&case.id, 96, "case id")?;
        bounded_text(&case.label, 128, "case label")?;
        bounded_text(&case.prompt, 4096, "case prompt")?;
        bounded_text(&case.expected_minimum_tier, 32, "expected minimum tier")?;
        if !["fast", "balanced", "strong", "frontier"]
            .contains(&case.expected_minimum_tier.as_str())
        {
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
    let case_ids: std::collections::BTreeSet<_> =
        corpus.cases.iter().map(|case| case.id.as_str()).collect();
    let mut run_ids = std::collections::BTreeSet::new();
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
            (&outcome.route_id, 96, "route id"),
            (&outcome.provider, 128, "provider"),
            (&outcome.model, 160, "model"),
        ] {
            bounded_text(value, limit, name)?;
        }
        for (value, limit, name) in [
            (&outcome.provider_version, 160, "provider version"),
            (&outcome.model_version, 160, "model version"),
        ] {
            if let Some(value) = value {
                bounded_text(value, limit, name)?;
            }
        }
        if !run_ids.insert(outcome.run_id.as_str()) {
            return Err(format!("Duplicate run id '{}'.", outcome.run_id));
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
    let provider_models: std::collections::BTreeSet<_> = records
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
    MetricSummary {
        route_id: route_id.into(),
        outcome_count: records.len(),
        completed_count: completed.len(),
        completion_rate: (!completed.is_empty()).then(|| {
            completed.iter().filter(|value| **value).count() as f64 / completed.len() as f64
        }),
        test_observation_count: test_records.len(),
        tests_passed: (!test_records.is_empty()).then_some(tests_passed),
        tests_executed: (!test_records.is_empty()).then_some(tests_executed),
        test_pass_rate: (tests_executed > 0).then(|| tests_passed as f64 / tests_executed as f64),
        cost_observation_count: costs.len(),
        mean_cost_usd: (!costs.is_empty()).then(|| costs.iter().sum::<f64>() / costs.len() as f64),
        latency_observation_count: latencies.len(),
        mean_latency_ms: (!latencies.is_empty())
            .then(|| latencies.iter().sum::<u64>() as f64 / latencies.len() as f64),
        provider_models: provider_models.into_iter().collect(),
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
            cases: vec![LabeledCase {
                id: "case-1".into(),
                label: "answer".into(),
                prompt: "Explain a small behavior.".into(),
                expected_minimum_tier: "fast".into(),
            }],
        }
    }

    #[test]
    fn missing_run_observations_stay_missing_in_report() {
        let corpus = corpus();
        let outcome = RunOutcome {
            schema_version: EVALUATION_SCHEMA_VERSION.into(),
            provenance: Provenance::Synthetic,
            run_id: "run-1".into(),
            case_id: "case-1".into(),
            route_id: "jev_route".into(),
            provider: "fixture".into(),
            provider_version: None,
            model: "fast".into(),
            model_version: None,
            task_completed: None,
            tests: None,
            cost_usd: None,
            latency_ms: None,
        };
        let report = report(&corpus, &[outcome]).unwrap();
        let metrics = &report.metrics[0];
        assert_eq!(metrics.outcome_count, 1);
        assert_eq!(metrics.completed_count, 0);
        assert_eq!(metrics.completion_rate, None);
        assert_eq!(metrics.mean_cost_usd, None);
        assert_eq!(metrics.mean_latency_ms, None);
        assert_eq!(metrics.test_pass_rate, None);
    }

    #[test]
    fn measured_and_synthetic_outcomes_cannot_be_mixed() {
        let corpus = corpus();
        let make = |run_id: &str, provenance| RunOutcome {
            schema_version: EVALUATION_SCHEMA_VERSION.into(),
            provenance,
            run_id: run_id.into(),
            case_id: "case-1".into(),
            route_id: "jev_route".into(),
            provider: "fixture".into(),
            provider_version: None,
            model: "fast".into(),
            model_version: None,
            task_completed: Some(true),
            tests: None,
            cost_usd: None,
            latency_ms: None,
        };
        assert!(report(
            &corpus,
            &[
                make("s", Provenance::Synthetic),
                make("m", Provenance::Measured)
            ]
        )
        .is_err());
    }

    #[test]
    fn observed_values_drive_route_comparison_summaries() {
        let mut corpus = corpus();
        corpus.provenance = Provenance::Measured;
        let outcome = RunOutcome {
            schema_version: EVALUATION_SCHEMA_VERSION.into(),
            provenance: Provenance::Measured,
            run_id: "observed-run".into(),
            case_id: "case-1".into(),
            route_id: "jev_route".into(),
            provider: "local-cli".into(),
            provider_version: Some("1.2.3".into()),
            model: "model-a".into(),
            model_version: Some("2026-09".into()),
            task_completed: Some(true),
            tests: Some(TestEvidence {
                executed: 3,
                passed: 2,
                failed: 1,
            }),
            cost_usd: Some(0.125),
            latency_ms: Some(700),
        };
        let report = report(&corpus, &[outcome]).unwrap();
        let metrics = &report.metrics[0];
        assert_eq!(report.outcome_provenance, Provenance::Measured);
        assert_eq!(metrics.completion_rate, Some(1.0));
        assert_eq!(metrics.test_pass_rate, Some(2.0 / 3.0));
        assert_eq!(metrics.mean_cost_usd, Some(0.125));
        assert_eq!(metrics.mean_latency_ms, Some(700.0));
        assert!(metrics.provider_models[0].contains("local-cli@1.2.3/model-a@2026-09"));
    }
}
