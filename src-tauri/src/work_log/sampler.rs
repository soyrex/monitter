//! Sanitized adapter from the existing local process sampler.
use super::types::{ProcessIdentity, ProcessObservation, ProcessSource};

pub struct LocalProcessSource {
    host_id: String,
}

impl LocalProcessSource {
    pub fn new(host_id: impl Into<String>) -> Self {
        Self {
            host_id: host_id.into(),
        }
    }
}

impl ProcessSource for LocalProcessSource {
    fn sample(&self) -> Result<Vec<ProcessObservation>, String> {
        let sample = crate::process_metrics::sample()?;
        Ok(sample
            .processes
            .into_iter()
            .map(|p| ProcessObservation {
                identity: ProcessIdentity {
                    host_id: self.host_id.clone(),
                    pid: p.pid,
                    started_at: p.started_at,
                },
                parent_pid: p.parent_pid,
                cpu_time_ms: p.cpu_time_ms,
                resident_memory_bytes: p.resident_memory_bytes,
            })
            .collect())
    }
}
