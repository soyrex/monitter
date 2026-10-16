use super::*;
use crate::model::{id, now, Host, Snapshot, Task};
use std::{
    collections::{HashMap, HashSet},
    time::Instant,
};

#[derive(Clone, Default)]
pub struct Lifecycle {
    spans: HashMap<String, ActiveSpan>,
    pub active: HashMap<String, (WorkRun, ContextRef, Instant)>,
    paused: bool,
    observed_elapsed: HashMap<String, u64>,
    resumed_at: HashMap<String, (i64, Option<String>)>,
}
#[derive(Clone)]
struct ActiveSpan {
    kind: String,
    wall: i64,
    clock: Instant,
}
impl Lifecycle {
    pub fn clear(&mut self) {
        self.active.clear();
        self.spans.clear();
        self.observed_elapsed.clear();
        self.resumed_at.clear();
        self.paused = false;
    }
    pub fn is_paused(&self) -> bool {
        self.paused
    }
    /// End only the observed span. The real provider run remains active while
    /// collection is disabled; its elapsed clock is frozen at this boundary.
    pub fn pause(&mut self) -> Vec<PendingRecord> {
        if self.paused {
            return vec![];
        }
        self.paused = true;
        let at = now();
        let mut records = Vec::new();
        for (id, (run, context, clock)) in &mut self.active {
            if let Some(span) = self.spans.remove(id) {
                let duration = span.clock.elapsed().as_millis().min(u64::MAX as u128) as u64;
                let gap = at < span.wall
                    || (at.saturating_sub(span.wall) as i128 - duration as i128).abs() > 2_000
                    || duration > 30_000;
                let interval = WorkInterval {
                    interval_id: crate::model::id(),
                    run_id: id.clone(),
                    kind: if gap { "unknown".into() } else { span.kind },
                    started_at: span.wall.min(at),
                    ended_at: span.wall.max(at),
                    duration_ms: if gap { 0 } else { duration },
                    coverage: if gap {
                        "clockOrObservationGap"
                    } else {
                        "observed"
                    }
                    .into(),
                };
                records.push(PendingRecord {
                    source_key: format!("interval:{}", interval.interval_id),
                    record: WorkRecord::Interval(interval),
                });
            }
            let elapsed = self.observed_elapsed.entry(id.clone()).or_default();
            *elapsed =
                elapsed.saturating_add(clock.elapsed().as_millis().min(u64::MAX as u128) as u64);
            run.elapsed_ms = Some(*elapsed);
            run.observed_at = at;
            run.coverage = "collectionPaused".into();
            records.push(PendingRecord {
                source_key: format!("{id}:pause:{}", crate::model::id()),
                record: WorkRecord::Run(run.clone()),
            });
            records.push(PendingRecord {
                source_key: format!("{id}:context:pause:{}", crate::model::id()),
                record: WorkRecord::Context(context.clone()),
            });
        }
        records
    }
    pub fn apply_paused_contexts(&mut self, records: &[PendingRecord]) {
        for record in records {
            if let WorkRecord::Context(context) = &record.record {
                if let Some((_, current, _)) = self.active.get_mut(&context.run_id) {
                    if current.context_ref == context.context_ref {
                        *current = context.clone();
                    }
                }
            }
        }
    }
    /// Resume observation on the same real run ID. A new context reference
    /// starts at the first post-resume message so paused text cannot leak.
    pub fn resume(
        &mut self,
        state: &Snapshot,
        current_run_ids: &HashSet<String>,
    ) -> Vec<PendingRecord> {
        if !self.paused {
            return vec![];
        }
        self.paused = false;
        let at = now();
        let mut records = Vec::new();
        let mut ended = Vec::new();
        for (id, (run, context, clock)) in &mut self.active {
            let running = state
                .tasks
                .iter()
                .any(|t| t.id == run.task_id && t.status == "running")
                && current_run_ids.contains(id);
            if !running {
                run.ended_at = Some(run.observed_at);
                run.outcome = Some("unknownEndDuringPause".into());
                run.coverage = "unknownEndDuringPause".into();
                records.push(PendingRecord {
                    source_key: format!("{id}:endedWhilePaused"),
                    record: WorkRecord::Run(run.clone()),
                });
                records.push(PendingRecord {
                    source_key: format!("{id}:endedWhilePaused:event"),
                    record: WorkRecord::Event(event(
                        run,
                        "runEnd",
                        None,
                        "backend",
                        run.observed_at,
                    )),
                });
                ended.push(id.clone());
                continue;
            }
            *clock = Instant::now();
            let pending: Vec<_> = state
                .approval_requests
                .iter()
                .filter(|a| {
                    a.task_id == run.task_id
                        && a.created_at >= run.started_at
                        && a.status == "pending"
                })
                .collect();
            let kind = if pending.is_empty() {
                "runnable"
            } else if pending.iter().any(|a| a.input.is_none()) {
                "approvalWait"
            } else {
                "inputWait"
            };
            self.spans.insert(
                id.clone(),
                ActiveSpan {
                    kind: kind.into(),
                    wall: at,
                    clock: Instant::now(),
                },
            );
            let boundary = state
                .messages
                .iter()
                .rev()
                .find(|m| m.task_id == run.task_id)
                .map(|m| m.id.clone());
            self.resumed_at.insert(id.clone(), (at, boundary));
            let mut next = context.clone();
            next.context_ref = crate::model::id();
            next.segment_start_id = None;
            next.source_message_id = None;
            next.through_message_id = None;
            next.through_message_text_bytes = None;
            next.availability = ContextAvailability::NotRecorded;
            next.unavailable_reason = Some("awaitingObservedMessage".into());
            *context = next.clone();
            run.context_ref = next.context_ref.clone();
            run.coverage = "partial".into();
            run.observed_at = at;
            records.push(PendingRecord {
                source_key: format!("{id}:context:resume:{}", next.context_ref),
                record: WorkRecord::Context(next),
            });
            records.push(PendingRecord {
                source_key: format!("{id}:resume:{}", crate::model::id()),
                record: WorkRecord::Run(run.clone()),
            });
            records.push(PendingRecord {
                source_key: format!("{id}:resume:event:{}", crate::model::id()),
                record: WorkRecord::Event(event(run, "resume", None, "backend", at)),
            });
        }
        for id in ended {
            self.active.remove(&id);
            self.observed_elapsed.remove(&id);
            self.resumed_at.remove(&id);
        }
        records
    }
    pub fn register(
        &mut self,
        task: &Task,
        host: &Host,
        state: &Snapshot,
        run_id: String,
        started_at: i64,
        session: &str,
        identity: (String, String),
        anchors: (Option<String>, Option<String>),
        lineage: (
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
        ),
    ) -> Vec<PendingRecord> {
        if self.paused {
            return vec![];
        }
        if self.active.contains_key(&run_id) {
            return vec![];
        }
        let at = now();
        let reference = id();
        let agent = state.agents.iter().find(|a| a.id == task.agent_id);
        let internal = agent.is_some_and(|a| a.internal);
        let context = ContextRef {
            context_ref: reference.clone(),
            installation_id: identity.0,
            profile_id: identity.1,
            project_id: task.project_id.clone(),
            task_id: task.id.clone(),
            run_id: run_id.clone(),
            provider_turn_id: None,
            segment_start_id: anchors.0,
            source_message_id: anchors.1,
            through_message_id: None,
            through_message_text_bytes: None,
            title: task.title.chars().take(240).collect(),
            agent_name: agent.map(|a| a.name.clone()).unwrap_or_default(),
            harness: task.provider.clone(),
            native_session_id: task.native_session_id.clone(),
            lineage_kind: lineage.0,
            lineage_root_task_id: lineage.3,
            source_task_id: lineage.1,
            source_through_message_id: lineage.2,
            availability: if internal {
                ContextAvailability::NotRecorded
            } else {
                ContextAvailability::Available
            },
            unavailable_reason: internal.then(|| "internalTranscriptNotRecorded".into()),
        };
        let parent_run_id = task.parent_task_id.as_ref().and_then(|parent| {
            self.active
                .values()
                .find(|(r, _, _)| &r.task_id == parent)
                .map(|(r, _, _)| r.run_id.clone())
        });
        let run = WorkRun {
            run_id: run_id.clone(),
            task_id: task.id.clone(),
            project_id: task.project_id.clone(),
            agent_id: task.agent_id.clone(),
            host_id: host.id.clone(),
            backend_session_id: session.into(),
            parent_run_id,
            parent_task_id: task.parent_task_id.clone(),
            context_ref: reference,
            title: task.title.chars().take(240).collect(),
            project_name: state
                .projects
                .iter()
                .find(|p| Some(&p.id) == task.project_id.as_ref())
                .map(|p| p.name.clone()),
            agent_name: context.agent_name.clone(),
            host_name: host.name.clone(),
            harness: task.provider.clone(),
            configured_model: (!task.model.is_empty()).then(|| task.model.clone()),
            observed_model: None,
            cwd: (!task.cwd.is_empty()).then(|| task.cwd.clone()),
            repository_root: None,
            commit_sha: None,
            adapter_version: None,
            reasoning_setting: task
                .model_settings
                .as_ref()
                .and_then(|m| m.reasoning_effort.clone()),
            trigger: "unknown".into(),
            remote: host.kind == "ssh",
            started_at,
            observed_at: at,
            ended_at: None,
            elapsed_ms: None,
            outcome: None,
            coverage: if started_at < at {
                "registrationDelay"
            } else {
                "observed"
            }
            .into(),
        };
        let event = event(&run, "runStart", None, "backend", at);
        self.spans.insert(
            run_id.clone(),
            ActiveSpan {
                kind: "runnable".into(),
                wall: at,
                clock: Instant::now(),
            },
        );
        self.active.insert(
            run_id.clone(),
            (run.clone(), context.clone(), Instant::now()),
        );
        self.observed_elapsed.insert(run_id.clone(), 0);
        vec![
            PendingRecord {
                source_key: format!("{run_id}:context"),
                record: WorkRecord::Context(context),
            },
            PendingRecord {
                source_key: format!("{run_id}:start"),
                record: WorkRecord::Run(run),
            },
            PendingRecord {
                source_key: format!("{run_id}:start:event"),
                record: WorkRecord::Event(event),
            },
        ]
    }

    pub fn changes(&mut self, before: &Snapshot, after: &Snapshot) -> Vec<PendingRecord> {
        if self.paused {
            // An actual task ending while collection is disabled has an
            // unknown tail. Close it at its last observed pause boundary.
            let mut records = Vec::new();
            let ended = self
                .active
                .iter()
                .filter(|(_, (run, _, _))| {
                    after
                        .tasks
                        .iter()
                        .find(|t| t.id == run.task_id)
                        .is_none_or(|t| t.status != "running")
                })
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>();
            for id in ended {
                if let Some((mut run, _, _)) = self.active.remove(&id) {
                    run.ended_at = Some(run.observed_at);
                    run.outcome = Some("unknownEndDuringPause".into());
                    run.coverage = "unknownEndDuringPause".into();
                    records.push(PendingRecord {
                        source_key: format!("{id}:endedWhilePaused"),
                        record: WorkRecord::Run(run.clone()),
                    });
                    records.push(PendingRecord {
                        source_key: format!("{id}:endedWhilePaused:event"),
                        record: WorkRecord::Event(event(
                            &run,
                            "runEnd",
                            None,
                            "backend",
                            run.observed_at,
                        )),
                    });
                }
                self.observed_elapsed.remove(&id);
                self.resumed_at.remove(&id);
            }
            return records;
        }
        let at = now();
        let mut records = Vec::new();
        let mut ended = Vec::new();
        let old_events: HashSet<_> = before.events.iter().map(|e| e.id.as_str()).collect();
        for (id, (run, context, clock)) in &mut self.active {
            let task = after.tasks.iter().find(|t| t.id == run.task_id);
            let terminal = task.is_none_or(|t| t.status != "running");
            let waiting: Vec<_> = after
                .approval_requests
                .iter()
                // ApprovalRequest.run_id is a provider request key, not the
                // Monitter run UUID. A task has one active provider run.
                .filter(|a| {
                    a.task_id == run.task_id
                        && a.created_at >= run.started_at
                        && a.status == "pending"
                })
                .collect();
            if let Some((resumed_at, boundary)) = self.resumed_at.get(id).cloned() {
                if context.source_message_id.is_none() {
                    let boundary_index = boundary
                        .as_ref()
                        .and_then(|anchor| after.messages.iter().position(|m| m.id == *anchor));
                    let first = after
                        .messages
                        .iter()
                        .enumerate()
                        .find(|(index, m)| {
                            m.task_id == run.task_id
                                && matches!(m.role.as_str(), "user" | "assistant")
                                && match boundary_index {
                                    Some(boundary) => *index > boundary,
                                    None => m.created_at > resumed_at,
                                }
                        })
                        .map(|(_, m)| m);
                    if let Some(first) = first {
                        context.segment_start_id = Some(first.id.clone());
                        context.source_message_id = Some(first.id.clone());
                        // A streaming first message can grow in place. Freeze
                        // this provisional reference at the observed prefix.
                        context.through_message_id = Some(first.id.clone());
                        context.through_message_text_bytes =
                            Some(first.text.len().min(u32::MAX as usize) as u32);
                        context.availability = ContextAvailability::Available;
                        context.unavailable_reason = None;
                        records.push(PendingRecord {
                            source_key: format!("{id}:context:anchor:{}", context.context_ref),
                            record: WorkRecord::Context(context.clone()),
                        });
                        self.resumed_at.remove(id);
                    }
                }
            }
            let kind = if waiting.is_empty() {
                "runnable"
            } else if waiting.iter().any(|a| a.input.is_none()) {
                "approvalWait"
            } else {
                "inputWait"
            };
            let span = self.spans.entry(id.clone()).or_insert_with(|| ActiveSpan {
                kind: kind.into(),
                wall: at,
                clock: Instant::now(),
            });
            let checkpoint = span.clock.elapsed().as_secs() >= 10;
            let mut changed_model = false;
            if let Some(model) = after
                .messages
                .iter()
                .rev()
                .filter(|m| {
                    m.task_id == run.task_id
                        && before
                            .messages
                            .iter()
                            .find(|old| old.id == m.id)
                            .and_then(|old| old.response_metadata.as_ref())
                            != m.response_metadata.as_ref()
                })
                .find_map(|m| m.response_metadata.as_ref().map(|meta| meta.model.as_str()))
                .filter(|m| !m.is_empty())
            {
                if run.observed_model.as_deref() != Some(model) {
                    run.observed_model = Some(model.chars().take(240).collect());
                    changed_model = true;
                }
            }
            if terminal || span.kind != kind || checkpoint {
                let duration = span.clock.elapsed().as_millis().min(u64::MAX as u128) as u64;
                let gap = at < span.wall
                    || (at.saturating_sub(span.wall) as i128 - duration as i128).abs() > 2_000
                    || duration > 30_000;
                let interval = WorkInterval {
                    interval_id: crate::model::id(),
                    run_id: id.clone(),
                    kind: if gap {
                        "unknown".into()
                    } else {
                        span.kind.clone()
                    },
                    started_at: span.wall.min(at),
                    ended_at: span.wall.max(at),
                    duration_ms: if gap { 0 } else { duration },
                    coverage: if gap {
                        "clockOrObservationGap"
                    } else {
                        "observed"
                    }
                    .into(),
                };
                records.push(PendingRecord {
                    source_key: format!("interval:{}", interval.interval_id),
                    record: WorkRecord::Interval(interval),
                });
                if gap {
                    run.coverage = "clockOrObservationGap".into();
                }
                if span.kind != kind && !terminal {
                    let ev = event(
                        run,
                        if kind == "runnable" {
                            "resume"
                        } else {
                            "waitStart"
                        },
                        None,
                        "backend",
                        at,
                    );
                    records.push(PendingRecord {
                        source_key: format!("transition:{}", ev.event_id),
                        record: WorkRecord::Event(ev),
                    });
                }
                *span = ActiveSpan {
                    kind: kind.into(),
                    wall: at,
                    clock: Instant::now(),
                };
            }
            if checkpoint || changed_model {
                run.observed_at = at;
                run.elapsed_ms = Some(
                    self.observed_elapsed
                        .get(id)
                        .copied()
                        .unwrap_or(0)
                        .saturating_add(clock.elapsed().as_millis().min(u64::MAX as u128) as u64),
                );
                records.push(PendingRecord {
                    source_key: format!("checkpoint:{}", crate::model::id()),
                    record: WorkRecord::Run(run.clone()),
                });
            }
            if terminal {
                let mut final_run = run.clone();
                final_run.observed_at = at;
                final_run.ended_at = Some(at);
                final_run.elapsed_ms = Some(
                    self.observed_elapsed
                        .get(id)
                        .copied()
                        .unwrap_or(0)
                        .saturating_add(clock.elapsed().as_millis().min(u64::MAX as u128) as u64),
                );
                final_run.outcome = Some(
                    task.map(|t| t.status.clone())
                        .unwrap_or_else(|| "deleted".into()),
                );
                if context.availability == ContextAvailability::Available {
                    // Terminal state advances the same segment through its
                    // final observed message, with a new immutable byte cap.
                    // The earlier anchor version remains in destination history.
                    if let Some(last) = after.messages.iter().rev().find(|m| {
                        m.task_id == run.task_id && matches!(m.role.as_str(), "user" | "assistant")
                    }) {
                        context.through_message_id = Some(last.id.clone());
                        context.through_message_text_bytes =
                            Some(last.text.len().min(u32::MAX as usize) as u32);
                    }
                } else {
                    context.through_message_id = None;
                    context.through_message_text_bytes = None;
                }
                if task.is_none() {
                    context.availability = ContextAvailability::Deleted;
                    context.unavailable_reason = Some("sourceDeleted".into());
                }
                records.push(PendingRecord {
                    source_key: format!("{id}:context:end"),
                    record: WorkRecord::Context(context.clone()),
                });
                records.push(PendingRecord {
                    source_key: format!("{id}:end"),
                    record: WorkRecord::Run(final_run.clone()),
                });
                records.push(PendingRecord {
                    source_key: format!("{id}:end:event"),
                    record: WorkRecord::Event(event(&final_run, "runEnd", None, "backend", at)),
                });
                ended.push(id.clone());
            }
            for plan in after.work_plans.iter().filter(|p| p.task_id == run.task_id) {
                let old = before.work_plans.iter().find(|p| p.id == plan.id);
                if old != Some(plan) {
                    let mut ev = event(
                        run,
                        "workPlan",
                        Some(plan.title.chars().take(1000).collect()),
                        "agentDeclared",
                        at,
                    );
                    ev.plan_id = Some(plan.id.clone());
                    records.push(PendingRecord {
                        source_key: format!("planTransition:{}", ev.event_id),
                        record: WorkRecord::Event(ev),
                    });
                    for item in &plan.items {
                        if old.and_then(|p| p.items.iter().find(|i| i.id == item.id)) != Some(item)
                        {
                            let mut ev = event(
                                run,
                                "planItem",
                                Some(item.title.chars().take(1000).collect()),
                                "agentDeclared",
                                at,
                            );
                            ev.plan_id = Some(plan.id.clone());
                            ev.plan_item_id = Some(item.id.clone());
                            ev.coverage = item.status.clone();
                            records.push(PendingRecord {
                                source_key: format!("planItemTransition:{}", ev.event_id),
                                record: WorkRecord::Event(ev),
                            });
                        }
                    }
                }
            }
            // Only structured event identity/kind/title; raw detail/tool args never enter telemetry.
            for source in after
                .events
                .iter()
                .filter(|e| e.task_id == run.task_id && !old_events.contains(e.id.as_str()))
            {
                if matches!(
                    source.kind.as_str(),
                    "tool" | "approval" | "input" | "status"
                ) {
                    let kind = if source.kind == "tool" {
                        "toolObserved"
                    } else {
                        source.kind.as_str()
                    };
                    let ev = event(run, kind, None, "backend", source.created_at);
                    records.push(PendingRecord {
                        source_key: format!("event:{}", source.id),
                        record: WorkRecord::Event(ev),
                    });
                }
            }
        }
        for id in ended {
            self.active.remove(&id);
            self.spans.remove(&id);
            self.observed_elapsed.remove(&id);
            self.resumed_at.remove(&id);
        }
        records
    }
}
pub fn event(
    run: &WorkRun,
    kind: &str,
    summary: Option<String>,
    source: &str,
    at: i64,
) -> WorkEvent {
    WorkEvent {
        event_id: id(),
        run_id: run.run_id.clone(),
        context_ref: run.context_ref.clone(),
        kind: kind.into(),
        occurred_at: at,
        observed_at: now(),
        phase: None,
        summary,
        plan_id: None,
        plan_item_id: None,
        labels: vec![],
        source: source.into(),
        coverage: "observed".into(),
    }
}
