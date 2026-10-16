//! Backend-owned collection. The GUI never owns or schedules this worker.
use crate::{
    work_log::{self, *},
    Service,
};
use std::{
    sync::{atomic::Ordering, Arc},
    thread,
    time::{Duration, Instant},
};

pub(crate) enum ControlOperation {
    Delete(Vec<String>),
    Flush,
}
pub(crate) struct Control {
    operation: ControlOperation,
    reply: std::sync::mpsc::SyncSender<Result<serde_json::Value, String>>,
}

impl Service {
    pub(crate) fn work_log_register_run(
        &self,
        task_id: &str,
        control: &Arc<crate::runner::RunControl>,
    ) {
        let result = (|| -> Result<(), String> {
            let run_id = control.current_run_id()?;
            if self
                .work_log_lifecycle
                .lock()
                .map_err(|_| "Work log lifecycle unavailable.")?
                .active
                .contains_key(&run_id)
            {
                return Ok(());
            }
            let _writer = self
                .state_writes
                .lock()
                .map_err(|_| "State writer unavailable.")?;
            if !self.store.work_log_settings()?.enabled {
                return Ok(());
            }
            let data = self.committed_data()?;
            let task = data
                .snapshot
                .tasks
                .iter()
                .find(|t| t.id == task_id)
                .ok_or("Task unavailable.")?;
            if task.status != "running" {
                return Ok(());
            }
            let host = data.task_hosts.get(task_id).ok_or("Host unavailable.")?;
            let identity = self.store.work_log_identity()?;
            let anchors = self.store.work_log_context_anchors(task_id)?;
            let lineage = self.store.work_log_lineage(task_id)?;
            let mut lifecycle = self
                .work_log_lifecycle
                .lock()
                .map_err(|_| "Work log lifecycle unavailable.")?;
            let mut next = lifecycle.clone();
            let records = next.register(
                task,
                host,
                &data.snapshot,
                run_id.clone(),
                control.run_started_at()?,
                &self.revision_epoch,
                identity,
                anchors,
                lineage,
            );
            self.store.work_log_append(&records)?;
            *lifecycle = next;
            Ok(())
        })();
        if result.is_err() {
            self.work_log_degraded("sourceRegistration");
        }
    }
    pub(crate) fn work_log_degraded(&self, reason: &str) {
        if let Ok(mut health) = self.work_log_health.lock() {
            health.state = "degraded".into();
            health.reason = Some(reason.into());
        }
    }
    pub(crate) fn work_log_recover(&self) {
        let result = (|| -> Result<(), String> {
            let at = crate::model::now();
            let paused = !self.store.work_log_settings()?.enabled;
            let mut records = Vec::new();
            for mut run in self.store.work_log_active()? {
                if let Some(mut context) = self.store.work_log_active_context(&run.run_id)? {
                    // Disabled collection already froze its endpoint in the
                    // pause transaction. Never retarget it after a restart.
                    if !paused { context.through_message_id = self.store.work_log_segment_through(&context)?; }
                    records.push(PendingRecord {
                        source_key: format!("{}:context:recovered", run.run_id),
                        record: WorkRecord::Context(context),
                    });
                }
                let last = run.observed_at;
                run.outcome = Some(if paused {"unknownEndDuringPause"}else{"interrupted"}.into());
                run.ended_at = Some(last);
                run.coverage = if paused {"unknownEndDuringPause"}else{"interruptedTailUnknown"}.into();
                records.push(PendingRecord {
                    source_key: format!("{}:recovered", run.run_id),
                    record: WorkRecord::Run(run.clone()),
                });
                if !paused {
                    records.push(PendingRecord {
                        source_key: format!("{}:restartGap", run.run_id),
                        record: WorkRecord::Gap(CollectorGap {
                            gap_id: crate::model::id(),
                            run_id: Some(run.run_id),
                            started_at: last,
                            ended_at: Some(at),
                            reason: "backendRestart".into(),
                            dropped: 0,
                        }),
                    });
                }
            }
            self.store.work_log_append_recovery(&records)
        })();
        if result.is_err() {
            self.work_log_degraded("sourceRecovery")
        }
    }
    pub(crate) fn work_log_configure(
        &self,
        settings: CollectionSettings,
    ) -> Result<CollectionSettings, String> {
        settings.validate()?;
        let _writer = self
            .state_writes
            .lock()
            .map_err(|_| "State writer unavailable.")?;
        let previous=self.store.work_log_settings()?;
        let mut lifecycle=self.work_log_lifecycle.lock().map_err(|_| "Work log lifecycle unavailable.")?;
        let mut next=lifecycle.clone();
        let mut records=if previous.enabled && !settings.enabled {
            next.pause()
        } else if !previous.enabled && settings.enabled {
            let snapshot=self.committed_data()?;
            let current=self.runs.lock().map_err(|_| "Run identity unavailable.")?.tasks.values().filter_map(|control|control.current_run_id().ok()).collect::<std::collections::HashSet<_>>();
            next.resume(&snapshot.snapshot,&current)
        } else {Vec::new()};
        self.store.work_log_configure_with_records(&settings,&mut records)?;
        if previous.enabled && !settings.enabled {next.apply_paused_contexts(&records);}
        *lifecycle=next;
        if let Ok(mut health) = self.work_log_health.lock() {
            health.state = if settings.enabled {
                "starting"
            } else {
                "paused"
            }
            .into();
            health.reason = None;
        }
        Ok(settings)
    }
    pub(crate) fn work_log_checkpoint(&self) -> Result<(), String> {
        let _writer = self
            .state_writes
            .lock()
            .map_err(|_| "State writer unavailable.")?;
        let data = self.committed_data()?;
        let mut lifecycle = self
            .work_log_lifecycle
            .lock()
            .map_err(|_| "Lifecycle unavailable.")?;
        let mut next = lifecycle.clone();
        let records = next.changes(&data.snapshot, &data.snapshot);
        if !records.is_empty() {
            self.store.work_log_append(&records)?;
            *lifecycle = next;
        }
        Ok(())
    }
    pub(crate) fn work_log_control(
        &self,
        operation: ControlOperation,
    ) -> Result<serde_json::Value, String> {
        if self.stopping.load(Ordering::Acquire) || self.work_log_stop.load(Ordering::Acquire) {
            return Err("Collector is stopping.".into());
        }
        if let ControlOperation::Delete(ids) = &operation {
            if ids.is_empty()
                || ids.len() > 100
                || ids.iter().any(|id| uuid::Uuid::parse_str(id).is_err())
            {
                return Err("Invalid history selection.".into());
            }
            let _writer = self
                .state_writes
                .lock()
                .map_err(|_| "State writer unavailable.")?;
            let active = self.store.work_log_active()?;
            if ids.iter().any(|id| active.iter().any(|r| &r.run_id == id)) {
                return Err("Finish or pause selected runs before deleting their history.".into());
            }
        }
        let worker = self
            .work_log_worker
            .lock()
            .map_err(|_| "Collector unavailable.")?;
        let handle = worker.as_ref().ok_or("Collector unavailable.")?;
        let (send, receive) = std::sync::mpsc::sync_channel(1);
        let mut queue = self
            .work_log_commands
            .lock()
            .map_err(|_| "Collector unavailable.")?;
        if queue.len() >= 16 {
            return Err("Collector control queue is full.".into());
        }
        queue.push_back(Control {
            operation,
            reply: send,
        });
        handle.thread().unpark();
        drop(queue);
        drop(worker);
        receive.recv_timeout(Duration::from_secs(5)).map_err(|_| {
            "Collector request is still pending; retrying the same selection is safe."
        })?
    }
    pub(crate) fn stop_work_log_collector(&self) {
        self.work_log_stop.store(true, Ordering::Release);
        if let Ok(mut worker) = self.work_log_worker.lock() {
            if let Some(handle) = worker.take() {
                handle.thread().unpark();
                if handle.thread().id() != thread::current().id() {
                    let _ = handle.join();
                }
            }
        }
    }
    pub(crate) fn start_work_log_collector(self: &Arc<Self>) {
        let Ok(mut handle) = self.work_log_worker.lock() else {
            self.work_log_degraded("workerStart");
            return;
        };
        if handle.is_some() {
            return;
        }
        self.work_log_stop.store(false, Ordering::Release);
        let weak = Arc::downgrade(self);
        let worker=thread::Builder::new().name("monitter-work-log".into()).spawn(move|| {
            let mut database=None;
            let mut resources=work_log::resources::ResourceTracker::default();
            let started=Instant::now();
            let mut sampled=Instant::now()-Duration::from_secs(10);
            let mut maintained=Instant::now()-Duration::from_secs(60);
            let mut loss:Option<CollectorGap>=None;
            let mut maintenance_pending=false;
            loop {
                let Some(service)=weak.upgrade() else {break};
                let settings=service.store.work_log_settings();
                let mut failure=settings.as_ref().err().map(|_|"sourceSettings");
                let settings=settings.unwrap_or_default();
                let enabled=settings.enabled;
                let pending=match service.store.pending(1) {Ok(p)=>!p.is_empty(),Err(_)=>{failure=Some("sourcePending");false}};
                let control_pending=service.work_log_commands.lock().map(|q|!q.is_empty()).unwrap_or(false);
                if database.is_none() && (enabled || pending || control_pending) {
                    match work_log::store::WorkLogDb::open(&service.work_log_dir) {Ok(db)=>database=Some(db),Err(_)=>failure=Some("destinationOpen")}
                }
                if let Some(db)=database.as_ref() {
                    db.set_budget(settings.hard_limit_bytes);
                    let health=db.health().unwrap_or_default();
                    let size=health.database_bytes.saturating_add(health.wal_bytes);
                    let pressure=size>=settings.soft_limit_bytes;
                    if maintained.elapsed()>=Duration::from_secs(60) || pressure || maintenance_pending {
                        maintained=Instant::now();
                        match db.compact(&settings,crate::model::now(),pressure) {Ok(remaining)=>maintenance_pending=remaining,Err(_)=>failure=Some("maintenance")}
                    }
                    let size=db.health().map(|h|h.database_bytes.saturating_add(h.wal_bytes)).unwrap_or(size);
                    let hard=size>=settings.hard_limit_bytes;
                    let mut writer=work_log::writer::Writer::new(db).with_batch_limit(if settings.hard_limit_bytes<16*1024*1024 {5}else{100});
                    // Already committed duplicate replay must still advance
                    // the source acknowledgement at the destination limit.
                    if writer.flush(&service.store,10).is_err() {failure=Some("destinationDrain");}
                    if hard {failure=Some("historyHardLimit");}
                    if enabled && sampled.elapsed()>=Duration::from_secs(10) {
                        sampled=Instant::now();
                        if service.work_log_checkpoint().is_err() {failure=Some("sourceCheckpoint");}
                        if size<settings.soft_limit_bytes {
                            if service.work_log_sample(db,&mut resources,started.elapsed().as_millis().min(u64::MAX as u128) as u64).is_err() {failure=Some("resourceSample");}
                        } else {failure=Some(if hard {"historyHardLimit"}else{"resourceSoftLimit"});}
                    }
                    // A finite queue is drained only by this destination owner.
                    let controls:Vec<_>=service.work_log_commands.lock().map(|mut q|q.drain(..).collect()).unwrap_or_default();
                    for control in controls {
                        let result=match control.operation {
                            ControlOperation::Delete(ids)=>db.delete_runs(&ids).map(|n|serde_json::json!({"deletedRuns":n,"replayTombstonesRetained":true})).map_err(|_|"History deletion failed.".into()),
                            ControlOperation::Flush=>{
                                writer.flush(&service.store,100).map_err(|_|"Work log flush failed.".to_string()).and_then(|_|service.store.pending(1)).and_then(|p|if p.is_empty(){Ok(serde_json::json!({"flushed":true}))}else{Err("Work log flush remains pending.".into())})
                            }
                        };
                        let _=control.reply.send(result);
                    }
                    if let Ok(mut health)=db.health() {
                        if !enabled {health.state="paused".into();}
                        if let Ok((rows,bytes,degraded))=service.store.work_log_stats() {
                            health.pending_rows=rows;health.pending_bytes=bytes;
                            if let Some(reason)=degraded {health.state="degraded".into();health.reason=Some(reason);}
                        }
                        if let Some(reason)=failure {health.state="degraded".into();health.reason=Some(reason.into());}
                        if let Ok(mut current)=service.work_log_health.lock() {*current=health;}
                    }
                } else if let Some(reason)=failure {service.work_log_degraded(reason);}
                if enabled {
                    if let Some(reason)=failure {
                        if loss.is_none() {
                            let gap=CollectorGap {gap_id:crate::model::id(),run_id:None,started_at:crate::model::now(),ended_at:None,reason:reason.into(),dropped:0};
                            if service.store.work_log_append(&[PendingRecord{source_key:format!("collectorGap:{}:start",gap.gap_id),record:WorkRecord::Gap(gap.clone())}]).is_err(){service.work_log_degraded("sourceGap");}
                            loss=Some(gap);
                        }
                    } else if let Some(mut gap)=loss.take() {
                        gap.ended_at=Some(crate::model::now());
                        if service.store.work_log_append(&[PendingRecord{source_key:format!("collectorGap:{}:end",gap.gap_id),record:WorkRecord::Gap(gap)}]).is_err(){service.work_log_degraded("sourceGap");}
                    }
                }
                let stopping=service.stopping.load(Ordering::Acquire) || service.work_log_stop.load(Ordering::Acquire);
                drop(service);
                if stopping {break;}
                thread::park_timeout(Duration::from_secs(2));
            }
        });
        match worker {
            Ok(worker) => *handle = Some(worker),
            Err(_) => self.work_log_degraded("workerStart"),
        }
    }
    fn work_log_sample(
        &self,
        db: &work_log::store::WorkLogDb,
        tracker: &mut work_log::resources::ResourceTracker,
        mono: u64,
    ) -> Result<(), String> {
        let result = (|| -> Result<(), String> {
            let data = self.committed_data()?;
            let active = self
                .runs
                .lock()
                .map_err(|_| "Run registry unavailable.")?
                .tasks
                .iter()
                .filter_map(|(task, control)| {
                    data.snapshot
                        .tasks
                        .iter()
                        .find(|t| &t.id == task && t.status == "running")
                        .map(|t| (t.clone(), control.clone()))
                })
                .collect::<Vec<_>>();
            if active.is_empty() {
                return Ok(());
            }
            // Catch genuine native continuations which acquired their run ID
            // in the provider adapter rather than an ordinary user send.
            for (task, control) in &active {
                self.work_log_register_run(&task.id, control);
            }
            let host = data
                .snapshot
                .hosts
                .iter()
                .find(|h| h.kind == "local")
                .map(|h| h.id.clone())
                .ok_or("Local host unavailable.")?;
            let processes = work_log::sampler::LocalProcessSource::new(&host).sample()?;
            let mut roots = Vec::new();
            for (task, control) in active {
                let remote = data
                    .task_hosts
                    .get(&task.id)
                    .is_some_and(|h| h.kind == "ssh");
                let run_id = control.current_run_id()?;
                let root = control.work_log_root();
                let (pid, started_at) = root.unwrap_or((0, 0));
                roots.push(RunRoot {
                    run_id,
                    identity: ProcessIdentity {
                        host_id: if remote { task.host_id } else { host.clone() },
                        pid,
                        started_at,
                    },
                    remote,
                });
            }
            let samples = tracker.observe(crate::model::now(), mono, &roots, &processes);
            db.append_resources(&samples)
                .map_err(|_| "destinationResources")?;
            Ok(())
        })();
        result
    }
}
