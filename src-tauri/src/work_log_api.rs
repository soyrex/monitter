//! Owner-private control and separately granted agent operations.
use crate::{
    work_log::{self, access::*, *},
    Service,
};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde_json::{json, Value};

pub(crate) struct ExportJob {
    pub(crate) cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    result:
        std::sync::Arc<std::sync::Mutex<Option<Result<work_log::export::ExportReceipt, String>>>>,
    created_at: i64,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Service {
    pub(crate) fn work_log_reader(&self) -> Result<Connection, String> {
        let path = self
            .work_log_dir
            .canonicalize()
            .map_err(|_| "Work log history is unavailable.")?
            .join("work-log.sqlite3");
        let c = work_log::schema::open_reader_connection(&path)
            .map_err(|_| "Work log history is unavailable.")?;
        work_log::schema::validate_identity(&c).map_err(|_| "Work log history is unavailable.")?;
        c.busy_timeout(std::time::Duration::from_millis(100))
            .map_err(|_| "Work log reader unavailable.")?;
        c.execute_batch("PRAGMA query_only=ON")
            .map_err(|_| "Work log reader unavailable.")?;
        Ok(c)
    }
    fn work_log_caller(&self, task: &str) -> Result<CallerIdentity, String> {
        self.require_collaboration_caller(task)?;
        let run_id = self
            .runs
            .lock()
            .map_err(|_| "Run identity unavailable.")?
            .tasks
            .get(task)
            .ok_or("Run identity unavailable.")?
            .current_run_id()?;
        let (installation_id, profile_id) = self.store.work_log_identity()?;
        Ok(CallerIdentity {
            installation_id,
            profile_id,
            task_id: task.into(),
            run_id,
        })
    }
    pub(crate) fn work_log_grants(&self) -> Result<Vec<OwnerGrant>, String> {
        self.store
            .work_log_grants_json()?
            .map(|v| serde_json::from_str(&v).map_err(|_| "Work log grants unavailable.".into()))
            .unwrap_or_else(|| Ok(vec![]))
    }
    pub(crate) fn work_log_set_grants(&self, grants: Vec<OwnerGrant>) -> Result<(), String> {
        if grants.len() > 1024 {
            return Err("Too many work log grants.".into());
        }
        let _writer = self
            .state_writes
            .lock()
            .map_err(|_| "State writer unavailable.")?;
        let identity = self.store.work_log_identity()?;
        let data = self.committed_data()?;
        for grant in &grants {
            if grant.installation_id != identity.0
                || grant.profile_id != identity.1
                || !data
                    .snapshot
                    .tasks
                    .iter()
                    .any(|t| t.id == grant.caller_task_id)
            {
                return Err("Invalid work log grant identity.".into());
            }
            let id = match &grant.scope {
                GrantScope::Project(id) | GrantScope::Lineage(id) | GrantScope::Task(id) => id,
            };
            if uuid::Uuid::parse_str(id).is_err() {
                return Err("Invalid work log grant scope.".into());
            }
        }
        self.store.work_log_set_grants_json(
            &serde_json::to_string(&grants).map_err(|_| "Invalid work log grants.")?,
        )
    }
    pub(crate) fn work_log_report_protocol(
        &self,
        task: &str,
        input: ReportInput,
    ) -> Result<Value, String> {
        work_log::reports::validate(&input)?;
        let _writer = self
            .state_writes
            .lock()
            .map_err(|_| "State writer unavailable.")?;
        let caller = self.work_log_caller(task)?;
        let run = self
            .store
            .work_log_active()?
            .into_iter()
            .find(|r| r.run_id == caller.run_id && r.task_id == task)
            .ok_or("Work log run is unavailable or collection is paused.")?;
        let source = SourceIdentity {
            installation_id: caller.installation_id.clone(),
            profile_id: caller.profile_id.clone(),
            task_id: run.task_id.clone(),
            project_id: run.project_id.clone(),
            lineage_id: self
                .store
                .work_log_active_context(&run.run_id)?
                .and_then(|c| c.lineage_root_task_id),
        };
        authorize(
            &caller,
            &source,
            Operation::Report,
            &self.work_log_grants()?,
        )
        .map_err(|_| "Work log report access is not granted.")?;
        if let Some(item) = input.plan_item_id.as_deref() {
            if !self
                .committed_data()?
                .snapshot
                .work_plans
                .iter()
                .any(|p| p.task_id == task && p.items.iter().any(|i| i.id == item))
            {
                return Err("Work plan item does not belong to this task.".into());
            }
        }
        serde_json::to_value(self.store.work_log_report(&run, &input)?)
            .map_err(|_| "Work log receipt unavailable.".into())
    }
    pub(crate) fn work_log_read_context_protocol(
        &self,
        task: &str,
        reference_id: &str,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Value, String> {
        if uuid::Uuid::parse_str(reference_id).is_err() {
            return Err("Invalid work log context reference.".into());
        }
        let caller = {
            let _writer = self
                .state_writes
                .lock()
                .map_err(|_| "State writer unavailable.")?;
            self.work_log_caller(task)?
        };
        let reader = self.work_log_reader()?;
        let raw:Option<String>=reader.query_row("SELECT raw_json FROM context_refs WHERE context_ref=?1 ORDER BY version DESC LIMIT 1",[reference_id],|r|r.get(0)).optional().map_err(|_|"Work log context unavailable.")?;
        let reference: ContextRef =
            serde_json::from_str(&raw.ok_or("Work log context unavailable.")?)
                .map_err(|_| "Work log context unavailable.")?;
        let source = SourceIdentity {
            installation_id: reference.installation_id.clone(),
            profile_id: reference.profile_id.clone(),
            task_id: reference.task_id.clone(),
            project_id: reference.project_id.clone(),
            lineage_id: reference.lineage_root_task_id.clone(),
        };
        let history = Connection::open_with_flags(
            self.work_log_dir
                .canonicalize()
                .map_err(|_| "Chat history unavailable.")?
                .join("state.sqlite3"),
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .map_err(|_| "Chat history unavailable.")?;
        history
            .busy_timeout(std::time::Duration::from_millis(100))
            .map_err(|_| "Chat history unavailable.")?;
        let mut page = work_log::context::read_page(
            &history,
            &reference,
            &source,
            &caller,
            || {
                self.work_log_grants()
                    .map_err(|_| work_log::context::ContextReadError::Denied)
            },
            cursor,
            limit,
        )
        .map_err(|error| match error {
            work_log::context::ContextReadError::Denied => {
                "Work log context access is not granted."
            }
            _ => "Work log context unavailable.",
        })?;
        // Final recheck serializes with revocation/deletion without holding a
        // state writer fence during history or Work Log I/O.
        let _writer = self
            .state_writes
            .lock()
            .map_err(|_| "State writer unavailable.")?;
        if self.work_log_caller(task)? != caller {
            return Err("Work log caller changed; request a fresh context page.".into());
        }
        authorize(
            &caller,
            &source,
            Operation::ReadContext,
            &self.work_log_grants()?,
        )
        .map_err(|_| "Work log context access is not granted.")?;
        if !self
            .committed_data()?
            .snapshot
            .tasks
            .iter()
            .any(|t| t.id == reference.task_id)
        {
            page.availability = ContextAvailability::Deleted;
            page.messages.clear();
            page.next_cursor = None;
        }
        serde_json::to_value(page).map_err(|_| "Work log context unavailable.".into())
    }
    pub(crate) fn work_log_query_protocol(
        &self,
        task: &str,
        query: WorkQuery,
    ) -> Result<Value, String> {
        let caller = {
            let _writer = self
                .state_writes
                .lock()
                .map_err(|_| "State writer unavailable.")?;
            self.work_log_caller(task)?
        };
        let identity = self.store.work_log_identity()?;
        let grants = self.work_log_grants()?;
        let scope = read_scope(&caller, &grants);
        if scope.projects.is_empty() && scope.tasks.is_empty() && scope.lineages.is_empty() {
            return Err("Work log read access is not granted.".into());
        }
        let reader = self.work_log_reader()?;
        let page = work_log::query::page(&reader, &query, Some(&scope))?;
        let _writer = self
            .state_writes
            .lock()
            .map_err(|_| "State writer unavailable.")?;
        let current = self.work_log_caller(task)?;
        if current != caller
            || identity != self.store.work_log_identity()?
            || read_scope(&caller, &self.work_log_grants()?) != scope
        {
            return Err("Work log read access changed; request a fresh page.".into());
        }
        serde_json::to_value(page).map_err(|_| "Work log page unavailable.".into())
    }
    pub(crate) fn stop_work_log_exports(&self) {
        let jobs = self
            .work_log_exports
            .lock()
            .map(|mut j| j.drain().map(|(_, v)| v).collect::<Vec<_>>())
            .unwrap_or_default();
        for job in &jobs {
            job.cancel.store(true, std::sync::atomic::Ordering::Release);
        }
        for mut job in jobs {
            if let Some(worker) = job.worker.take() {
                let _ = worker.join();
            }
        }
    }
    pub(crate) fn work_log_owner(&self, operation: &str, args: Value) -> Result<Value, String> {
        match operation {
            "settings" => {
                let (installation_id, profile_id) = self.store.work_log_identity()?;
                Ok(
                    json!({"settings":self.store.work_log_settings()?,"identity":{"installationId":installation_id,"profileId":profile_id}}),
                )
            }
            "configure" => Ok(json!(self.work_log_configure(
                serde_json::from_value(args).map_err(|_| "Invalid work log settings.")?
            )?)),
            "health" => Ok(json!(self
                .work_log_health
                .lock()
                .map_err(|_| "Collector health unavailable.")?
                .clone())),
            "query" => {
                let query: WorkQuery =
                    serde_json::from_value(args).map_err(|_| "Invalid work log query.")?;
                Ok(json!(work_log::query::page(
                    &self.work_log_reader()?,
                    &query,
                    None
                )?))
            }
            "summary" => {
                let query: WorkQuery =
                    serde_json::from_value(args).map_err(|_| "Invalid work log query.")?;
                Ok(json!(work_log::query::summary(
                    &self.work_log_reader()?,
                    &query,
                    None
                )?))
            }
            "flush" => self.work_log_control(crate::work_log_runtime::ControlOperation::Flush),
            "deleteHistory" => {
                #[derive(serde::Deserialize)]
                #[serde(rename_all = "camelCase", deny_unknown_fields)]
                struct Input {
                    run_ids: Vec<String>,
                }
                let input: Input =
                    serde_json::from_value(args).map_err(|_| "Invalid history selection.")?;
                self.work_log_control(crate::work_log_runtime::ControlOperation::Delete(
                    input.run_ids,
                ))
            }
            "export" => {
                #[derive(serde::Deserialize)]
                #[serde(rename_all = "camelCase", deny_unknown_fields)]
                struct Input {
                    query: WorkQuery,
                    path: std::path::PathBuf,
                    format: String,
                }
                let input: Input =
                    serde_json::from_value(args).map_err(|_| "Invalid export request.")?;
                if !matches!(input.format.as_str(), "json" | "csv") {
                    return Err("Unsupported work log export format.".into());
                }
                let path = private_export_path(&input.path)?;
                let reader = self.work_log_reader()?;
                let mut jobs = self
                    .work_log_exports
                    .lock()
                    .map_err(|_| "Export jobs unavailable.")?;
                if self.stopping.load(std::sync::atomic::Ordering::Acquire) {
                    return Err("Backend is stopping.".into());
                }
                if jobs
                    .values()
                    .filter(|j| j.result.lock().map(|r| r.is_none()).unwrap_or(true))
                    .count()
                    >= 2
                {
                    return Err("Two exports are already active.".into());
                }
                if jobs.len() >= 16 {
                    if let Some(id) = jobs
                        .iter()
                        .filter(|(_, j)| j.result.lock().map(|r| r.is_some()).unwrap_or(false))
                        .min_by_key(|(_, j)| j.created_at)
                        .map(|(id, _)| id.clone())
                    {
                        jobs.remove(&id);
                    }
                }
                let id = uuid::Uuid::new_v4().to_string();
                let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                let result = std::sync::Arc::new(std::sync::Mutex::new(None));
                let worker_cancel = cancel.clone();
                let worker_result = result.clone();
                let worker = std::thread::Builder::new()
                    .name("work-log-export".into())
                    .spawn(move || {
                        let value = work_log::export::export(
                            &reader,
                            &input.query,
                            &path,
                            &input.format,
                            &worker_cancel,
                        );
                        if let Ok(mut output) = worker_result.lock() {
                            *output = Some(value);
                        }
                    })
                    .map_err(|_| "Export worker unavailable.")?;
                jobs.insert(
                    id.clone(),
                    ExportJob {
                        cancel,
                        result,
                        created_at: crate::model::now(),
                        worker: Some(worker),
                    },
                );
                Ok(json!({"exportId":id,"status":"running"}))
            }
            "exportStatus" | "cancelExport" => {
                #[derive(serde::Deserialize)]
                #[serde(rename_all = "camelCase", deny_unknown_fields)]
                struct Input {
                    export_id: String,
                }
                let input: Input =
                    serde_json::from_value(args).map_err(|_| "Invalid export identity.")?;
                let jobs = self
                    .work_log_exports
                    .lock()
                    .map_err(|_| "Export jobs unavailable.")?;
                let job = jobs
                    .get(&input.export_id)
                    .ok_or("Export job unavailable.")?;
                if operation == "cancelExport" {
                    job.cancel.store(true, std::sync::atomic::Ordering::Release);
                }
                let result = job
                    .result
                    .lock()
                    .map_err(|_| "Export result unavailable.")?;
                match result.as_ref() {
                    None => Ok(
                        json!({"exportId":input.export_id,"status":"running","cancelRequested":job.cancel.load(std::sync::atomic::Ordering::Acquire)}),
                    ),
                    Some(Ok(receipt)) => Ok(
                        json!({"exportId":input.export_id,"status":"succeeded","receipt":receipt}),
                    ),
                    Some(Err(error)) => Ok(
                        json!({"exportId":input.export_id,"status":if error=="Work log export cancelled." {"cancelled"}else{"failed"},"error":error}),
                    ),
                }
            }
            "grants" => Ok(json!(self.work_log_grants()?)),
            "setGrants" => {
                self.work_log_set_grants(
                    serde_json::from_value(args).map_err(|_| "Invalid work log grants.")?,
                )?;
                Ok(Value::Null)
            }
            _ => Err("Unknown owner work log operation.".into()),
        }
    }
}

fn read_scope(caller: &CallerIdentity, grants: &[OwnerGrant]) -> work_log::query::ReadScope {
    let mut scope = work_log::query::ReadScope {
        installation_id: Some(caller.installation_id.clone()),
        profile_id: Some(caller.profile_id.clone()),
        ..Default::default()
    };
    for grant in grants.iter().filter(|g| {
        !g.revoked
            && g.operation == Operation::ReadLog
            && g.installation_id == caller.installation_id
            && g.profile_id == caller.profile_id
            && g.caller_task_id == caller.task_id
    }) {
        match &grant.scope {
            GrantScope::Project(id) => scope.projects.push(id.clone()),
            GrantScope::Task(id) => scope.tasks.push(id.clone()),
            GrantScope::Lineage(id) => scope.lineages.push(id.clone()),
        }
    }
    for ids in [&mut scope.projects, &mut scope.tasks, &mut scope.lineages] {
        ids.sort();
        ids.dedup();
    }
    scope
}
fn private_export_path(path: &std::path::Path) -> Result<std::path::PathBuf, String> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    if !path.is_absolute() {
        return Err("Export needs an absolute local file path.".into());
    }
    let parent = path.parent().ok_or("Invalid export destination.")?;
    let meta = std::fs::symlink_metadata(parent).map_err(|_| "Export directory unavailable.")?;
    if !meta.is_dir()
        || meta.file_type().is_symlink()
        || meta.uid() != unsafe { libc::geteuid() }
        || meta.permissions().mode() & 0o022 != 0
    {
        return Err(
            "Export directory must be owned by the local owner and not writable by others.".into(),
        );
    }
    let name = path.file_name().ok_or("Invalid export destination.")?;
    Ok(parent
        .canonicalize()
        .map_err(|_| "Export directory unavailable.")?
        .join(name))
}
