//! SQLite-backed durable state. Service serializes candidate mutations; this
//! module commits them atomically before Service publishes a new revision.
#[path = "legacy_store.rs"]
mod legacy_store;
#[path = "store_migration.rs"]
mod migration;
#[path = "sqlite_backend.rs"]
mod sqlite_backend;

use crate::{
    attachments::StoredAttachment,
    model::{
        now, Host, RequiredUsageTokens, RunUsageAggregate, RunUsageSample, RunUsageSummary,
        Snapshot, UsageOverview, UsageTokens,
    },
};
use sqlite_backend::{Database, StateRef};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
};

struct UsageState {
    committed: crate::history::History<RunUsageSample>,
    staged: crate::history::History<RunUsageSample>,
    captured_since: Option<i64>,
    removed: Vec<RunUsageSample>,
}

pub struct Store {
    // Database drops before the lease, so another owner cannot enter before
    // this connection has finished its last WAL cleanup.
    database: Database,
    usage: Mutex<UsageState>,
    poisoned: AtomicBool,
    _lease: migration::Lease,
}

impl Store {
    pub fn open(
        dir: PathBuf,
    ) -> Result<
        (
            Self,
            Snapshot,
            HashMap<String, Host>,
            HashMap<String, StoredAttachment>,
        ),
        String,
    > {
        migration::private_dir(&dir)?;
        let lease = migration::Lease::acquire(&dir)?;
        let database = migration::open_database(&dir)?;
        let (mut snapshot, mut hosts, attachments, samples, captured_since) =
            database.read_resident_snapshot()?;
        let previous = snapshot.clone();
        let previous_hosts = hosts.clone();
        // Preserve immutable task host snapshots when importing older state.
        for task in &snapshot.tasks {
            if !hosts.contains_key(&task.id) {
                if let Some(host) = snapshot.hosts.iter().find(|host| host.id == task.host_id) {
                    hosts.insert(task.id.clone(), host.clone());
                }
            }
        }
        recover_after_restart(&mut snapshot);
        if snapshot != previous || hosts != previous_hosts {
            database.apply_update(
                (&previous, &previous_hosts, &attachments),
                (&snapshot, &hosts, &attachments),
                &samples,
                &samples,
                captured_since,
            )?;
        }
        let samples: crate::history::History<_> = samples.into();
        let store = Self {
            database,
            usage: Mutex::new(UsageState {
                committed: samples.clone(),
                staged: samples,
                captured_since,
                removed: Vec::new(),
            }),
            poisoned: AtomicBool::new(false),
            _lease: lease,
        };
        Ok((store, snapshot, hosts, attachments))
    }


    pub(crate) fn channel_messages(&self,channel_id:&str,before_id:Option<&str>,limit:Option<u32>,revision:String)->Result<crate::ui_sync::ChannelMessagesPage,String>{self.database.channel_page(channel_id,before_id,limit,revision)}
    pub(crate) fn task_messages(&self, task_id:&str, before_id:Option<&str>,limit:Option<u32>,revision:String)->Result<crate::ui_sync::TaskMessagesPage,String> { self.database.message_page(task_id,before_id,limit,revision) }
    pub(crate) fn messages_through(&self, task_id:&str, through_id:&str)->Result<Vec<crate::model::Message>,String> { self.database.messages_through(task_id,through_id) }
    pub(crate) fn task_message(&self, task_id: &str, message_id: &str) -> Result<crate::model::Message, String> {
        self.database.task_message(task_id, message_id)
    }
    pub(crate) fn task_events(&self,task_id:&str,before:Option<i64>,limit:Option<u32>)->Result<crate::lan_sync::TaskEventsPage,String>{self.database.event_page(task_id,before,limit)}
    pub(crate) fn task_event(&self,task_id:&str,event_id:&str)->Result<crate::model::RunEvent,String>{self.database.task_event(task_id,event_id)}
    fn require_writable(&self) -> Result<(), String> {
        if self.poisoned.load(Ordering::Acquire) {
            Err("Monitter persistence failed; reopen the app to reconcile durable state before making further changes.".into())
        } else {
            Ok(())
        }
    }

    /// Full-state callers are rare (bootstrap/import/test setup). Read the
    /// committed DB once and use the same row differential transaction path.
    pub fn save(
        &self,
        snapshot: &Snapshot,
        hosts: &HashMap<String, Host>,
        attachments: &HashMap<String, StoredAttachment>,
    ) -> Result<(), String> {
        self.require_writable()?;
        let previous = self.database.read_resident_snapshot()?;
        self.save_update(
            (&previous.0, &previous.1, &previous.2),
            (snapshot, hosts, attachments),
        )
    }

    pub fn save_update(&self, before: StateRef<'_>, after: StateRef<'_>) -> Result<(), String> {
        self.save_update_work_log(before,after,&[])
    }
    pub(crate) fn save_update_work_log(&self,before:StateRef<'_>,after:StateRef<'_>,records:&[crate::work_log::PendingRecord])->Result<(),String>{
        self.require_writable()?;
        let mut usage = self
            .usage
            .lock()
            .map_err(|_| "Monitter usage state lock failed.".to_string())?;
        let captured = usage
            .captured_since
            .or_else(|| (!usage.staged.is_empty()).then(now));
        let result =
            self.database
                .apply_update_history_usage(before, after, &usage.committed, &usage.staged, captured, records, &usage.removed);
        match result {
            Ok(()) => {
                usage.removed.clear();
                usage.committed = usage.staged.clone();
                // Start a new direct lineage for the next usage transaction.
                usage.staged = usage.committed.clone();
                usage.captured_since = captured;
                Ok(())
            }
            Err(error) => {
                // A COMMIT I/O error can have an uncertain acknowledgement.
                // Do not continue from an unpublished candidate: reopen and
                // let SQLite recover the authoritative committed transaction.
                self.poisoned.store(true, Ordering::Release);
                Err(error)
            }
        }
    }

    pub fn stage_usage(&self, sample: RunUsageSample) -> Result<(), String> {
        self.require_writable()?;
        let mut usage = self
            .usage
            .lock()
            .map_err(|_| "Monitter usage state lock failed.".to_string())?;
        if usage.staged.get_by_id(&sample.sample_id).map_err(|error| error.to_string())?.is_none() {
            usage.staged.push(sample);
        }
        Ok(())
    }

    pub fn remove_task_usage(&self, task_id: &str) -> Result<(), String> {
        self.require_writable()?;
        let mut usage=self.usage.lock().map_err(|_| "Monitter usage state lock failed.".to_string())?;
        let removed=usage.staged.iter().filter(|sample|sample.task_id==task_id).cloned().collect::<Vec<_>>();
        usage.removed.extend(removed);
        usage.staged.retain(|sample| sample.task_id != task_id);
        Ok(())
    }

    pub(crate) fn work_log_active_context(&self,run:&str)->Result<Option<crate::work_log::ContextRef>,String>{self.database.work_log_active_context(run)}
    pub(crate) fn work_log_segment_through(&self,reference:&crate::work_log::ContextRef)->Result<Option<String>,String>{self.database.work_log_segment_through(reference)}
    pub(crate) fn work_log_context_anchors(&self,task:&str)->Result<(Option<String>,Option<String>),String>{self.database.work_log_context_anchors(task)}
    pub(crate) fn work_log_lineage(&self,task:&str)->Result<(Option<String>,Option<String>,Option<String>,Option<String>),String>{self.database.work_log_lineage(task)}
    pub(crate) fn work_log_stats(&self)->Result<(u64,u64,Option<String>),String>{self.database.work_log_stats()}
    pub(crate) fn work_log_grants_json(&self)->Result<Option<String>,String>{self.database.work_log_grants_json()}
    pub(crate) fn work_log_set_grants_json(&self,value:&str)->Result<(),String>{self.require_writable()?;self.database.work_log_set_grants_json(value)}
    pub(crate) fn work_log_settings(&self)->Result<crate::work_log::CollectionSettings,String>{self.database.work_log_settings()}
    pub(crate) fn work_log_identity(&self)->Result<(String,String),String>{self.database.work_log_identity()}
    pub(crate) fn work_log_active(&self)->Result<Vec<crate::work_log::WorkRun>,String>{self.database.work_log_active()}
    pub(crate) fn work_log_append(&self,records:&[crate::work_log::PendingRecord])->Result<(),String>{self.require_writable()?;self.database.work_log_append(records).inspect_err(|_|{self.poisoned.store(true,Ordering::Release);})}
    pub(crate) fn work_log_configure(&self,settings:&crate::work_log::CollectionSettings)->Result<(),String>{self.require_writable()?;self.database.work_log_configure(settings).inspect_err(|_|{self.poisoned.store(true,Ordering::Release);})}
    pub(crate) fn work_log_configure_with_records(&self,settings:&crate::work_log::CollectionSettings,records:&mut [crate::work_log::PendingRecord])->Result<(),String>{self.require_writable()?;self.database.work_log_configure_with_records(settings,records).inspect_err(|_|{self.poisoned.store(true,Ordering::Release);})}
    pub(crate) fn work_log_append_recovery(&self,records:&[crate::work_log::PendingRecord])->Result<(),String>{self.require_writable()?;self.database.work_log_append_recovery(records).inspect_err(|_|{self.poisoned.store(true,Ordering::Release);})}
    pub(crate) fn work_log_report(&self,run:&crate::work_log::WorkRun,input:&crate::work_log::ReportInput)->Result<crate::work_log::ReportReceipt,String>{self.require_writable()?;self.database.work_log_report(run,input).inspect_err(|e|{if e.starts_with("Cannot commit work log report"){self.poisoned.store(true,Ordering::Release);}})}

    pub fn usage_overview(&self) -> Result<UsageOverview, String> {
        let usage = self
            .usage
            .lock()
            .map_err(|_| "Monitter usage state lock failed.".to_string())?;
        Ok(aggregate_usage(usage.staged.iter().cloned().collect(), usage.captured_since))
    }
}

fn recover_after_restart(snapshot: &mut Snapshot) {
    for message in &mut snapshot.messages {
        if message.stream_status.as_deref() == Some("streaming") {
            message.stream_status = Some("interrupted".into());
        }
    }
    let mut interrupted = std::collections::HashSet::new();
    for task in &mut snapshot.tasks {
        if task.status == "running" {
            task.status = "interrupted".into();
            task.updated_at = now();
            interrupted.insert(task.id.clone());
        }
    }
    let resolved_at = now();
    for request in &mut snapshot.approval_requests {
        if request.status == "pending" && interrupted.contains(&request.task_id) {
            request.status = "expired".into();
            request.resolved_at = Some(resolved_at);
        }
    }
    for queued in &mut snapshot.queued_messages {
        if queued.status == "sending" {
            queued.status = "error".into();
            queued.error = Some("Monitter restarted before this queued message could be confirmed. Review and retry it manually.".into());
        }
    }
    // Older collaboration records are the durable source for routed
    // delegations. Rebuild their compact sub-agent projection before the
    // recovery write below makes the upgraded state durable.
    crate::model::sync_collaboration_subagent_sessions(snapshot);
    crate::model::prune_parent_subagent_sessions(snapshot);
    crate::Service::recover_collaborations(snapshot);
}

fn aggregate_usage(samples: Vec<RunUsageSample>, captured_since: Option<i64>) -> UsageOverview {
    use std::collections::HashMap;
    let mut runs: HashMap<String, RunUsageSummary> = HashMap::new();
    for sample in samples {
        let entry = runs
            .entry(sample.run_id.clone())
            .or_insert_with(|| RunUsageSummary {
                run_id: sample.run_id.clone(),
                task_id: sample.task_id.clone(),
                provider: sample.provider.clone(),
                configured_model: sample.configured_model.clone(),
                started_at: sample.started_at,
                finished_at: None,
                final_: false,
                tokens: UsageTokens::default(),
                cost_usd: None,
                duration_ms: None,
                api_duration_ms: None,
                provider_turns: None,
                context: None,
            });
        entry.started_at = entry.started_at.min(sample.started_at);
        entry.final_ |= sample.final_sample;
        if sample.final_sample {
            entry.finished_at = Some(
                entry
                    .finished_at
                    .unwrap_or(sample.observed_at)
                    .max(sample.observed_at),
            );
        }
        let set = sample.classification == "cumulative";
        macro_rules! value {
            ($field:ident) => {
                if let Some(value) = sample.tokens.$field {
                    if set {
                        entry.tokens.$field = Some(value)
                    } else {
                        entry.tokens.$field =
                            Some(entry.tokens.$field.unwrap_or(0).saturating_add(value))
                    }
                }
            };
        }
        value!(input);
        value!(output);
        value!(cache_read);
        value!(cache_write);
        value!(reasoning);
        value!(total);
        macro_rules! scalar {
            ($field:ident) => {
                if let Some(value) = sample.$field {
                    entry.$field = Some(if set {
                        value
                    } else {
                        entry.$field.unwrap_or(0).saturating_add(value)
                    })
                }
            };
        }
        scalar!(duration_ms);
        scalar!(api_duration_ms);
        scalar!(provider_turns);
        if let Some(value) = sample.cost_usd {
            entry.cost_usd = Some(if set {
                value
            } else {
                entry.cost_usd.unwrap_or(0.0) + value
            });
        }
        if sample.context.is_some() {
            entry.context = sample.context;
        }
    }
    let all_runs: Vec<_> = runs.into_values().collect();
    let mut recent_runs = all_runs.clone();
    recent_runs.sort_by_key(|run| std::cmp::Reverse(run.started_at));
    recent_runs.truncate(50);
    let mut totals: HashMap<String, RunUsageAggregate> = HashMap::new();
    for run in &all_runs {
        let total = totals
            .entry(run.provider.clone())
            .or_insert_with(|| RunUsageAggregate {
                provider: run.provider.clone(),
                runs: 0,
                final_runs: 0,
                tokens: RequiredUsageTokens::default(),
                cost_usd: None,
                duration_ms: None,
            });
        total.runs += 1;
        total.final_runs += i64::from(run.final_);
        macro_rules! add {
            ($field:ident) => {
                total.tokens.$field += run.tokens.$field.unwrap_or(0);
            };
        }
        add!(input);
        add!(output);
        add!(cache_read);
        add!(cache_write);
        add!(reasoning);
        add!(total);
        if let Some(v) = run.cost_usd {
            total.cost_usd = Some(total.cost_usd.unwrap_or(0.0) + v)
        }
        if let Some(v) = run.duration_ms {
            total.duration_ms = Some(total.duration_ms.unwrap_or(0).saturating_add(v))
        }
    }
    UsageOverview {
        generated_at: now(),
        captured_since,
        subscriptions: vec![],
        provider_totals: totals.into_values().collect(),
        recent_runs,
    }
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;

impl crate::work_log::OutboxSource for Store {
 fn pending(&self,limit:usize)->Result<Vec<crate::work_log::CommittedRecord>,String>{self.database.work_log_pending(limit)}
 fn acknowledge(&self,through:i64)->Result<(),String>{self.require_writable()?;self.database.work_log_acknowledge(through)}
}
