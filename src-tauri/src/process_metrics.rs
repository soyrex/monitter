use serde::Serialize;

#[cfg(target_os = "macos")]
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProcessMetricsSample {
    pub cpu_time_ms: u64,
    pub resident_memory_bytes: u64,
    pub sampled_at: i64,
    pub root_pid: libc::pid_t,
    pub processes: Vec<ProcessMetricsProcess>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProcessMetricsProcess {
    pub pid: libc::pid_t,
    pub parent_pid: libc::pid_t,
    pub name: String,
    pub started_at: i64,
    pub cpu_time_ms: u64,
    pub resident_memory_bytes: u64,
}

/// Process identities used to fence idle retirement, without reading command
/// arguments or environments. A child which appeared after the provider's
/// handshake may be a live background tool and must not be killed by GC.
/// Unlike the UI's best-effort sample, incomplete inspection fails closed.
#[cfg(target_os = "macos")]
pub(crate) fn retirement_process_tree(
    root: libc::pid_t,
) -> Result<Vec<(libc::pid_t, i64)>, String> {
    let capacity = process_list_capacity()?;
    let mut pending = vec![root];
    let mut visited = HashSet::new();
    let mut identities = Vec::new();
    while let Some(pid) = pending.pop() {
        if !visited.insert(pid) {
            continue;
        }
        let (_, _, started_at) = process_identity(pid)
            .ok_or_else(|| "Could not verify the idle runtime's process ownership.".to_string())?;
        identities.push((pid, started_at));
        pending.extend(child_pids(pid, capacity)?);
    }
    Ok(identities)
}

/// Checks that a previously captured `(pid, started_at)` still identifies the
/// same process. A PID alone is never safe to signal because macOS may have
/// reused it for an unrelated process after a provider helper exited.
#[cfg(target_os = "macos")]
pub(crate) fn retirement_process_identity_alive(
    pid: libc::pid_t,
    started_at: i64,
) -> Result<bool, String> {
    if let Some((_, _, current_started_at)) = process_identity(pid) {
        return Ok(current_started_at == started_at);
    }
    let exists = unsafe { libc::kill(pid, 0) } == 0;
    if !exists && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
        return Ok(false);
    }
    Err(format!(
        "Could not verify whether idle runtime process identity {pid} is still alive."
    ))
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn retirement_process_identity_alive(
    _pid: libc::pid_t,
    _started_at: i64,
) -> Result<bool, String> {
    Err("Idle runtime process ownership inspection is unavailable on this platform.".into())
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn retirement_process_tree(
    _root: libc::pid_t,
) -> Result<Vec<(libc::pid_t, i64)>, String> {
    Err("Idle runtime process ownership inspection is unavailable on this platform.".into())
}

#[cfg(target_os = "macos")]
fn process_list_capacity() -> Result<usize, String> {
    let estimate = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    if estimate <= 0 {
        return Err("Could not list processes for Monitter metrics.".into());
    }
    Ok((estimate as usize).saturating_add(64))
}

#[cfg(target_os = "macos")]
fn child_pids(parent: libc::pid_t, initial_capacity: usize) -> Result<Vec<libc::pid_t>, String> {
    let mut capacity = initial_capacity.max(64);
    for _ in 0..3 {
        let mut pids = vec![0; capacity];
        let buffer_bytes = capacity
            .checked_mul(std::mem::size_of::<libc::pid_t>())
            .and_then(|bytes| libc::c_int::try_from(bytes).ok())
            .ok_or("The process list is too large to sample.")?;
        let count =
            unsafe { libc::proc_listchildpids(parent, pids.as_mut_ptr().cast(), buffer_bytes) };
        if count < 0 {
            return Err(format!("Could not list child processes for PID {parent}."));
        }
        let count = count as usize;
        if count < capacity {
            pids.truncate(count);
            pids.retain(|pid| *pid > 0);
            return Ok(pids);
        }
        capacity = capacity.saturating_mul(2);
    }
    Err(format!(
        "The child process list for PID {parent} changed too quickly to sample."
    ))
}

#[cfg(target_os = "macos")]
fn process_tree_pids(root: libc::pid_t) -> Result<Vec<libc::pid_t>, String> {
    let capacity = process_list_capacity()?;
    let mut tree = Vec::new();
    let mut pending = vec![root];
    let mut visited = HashSet::new();
    while let Some(pid) = pending.pop() {
        if !visited.insert(pid) {
            continue;
        }
        tree.push(pid);
        match child_pids(pid, capacity) {
            Ok(descendants) => pending.extend(descendants),
            // A descendant can disappear while the tree is being walked.
            Err(_) if pid != root => {}
            Err(reason) => return Err(reason),
        }
    }
    Ok(tree)
}

#[cfg(target_os = "macos")]
fn process_identity(pid: libc::pid_t) -> Option<(libc::pid_t, String, i64)> {
    let mut info = unsafe { std::mem::zeroed::<libc::proc_bsdinfo>() };
    let size = libc::c_int::try_from(std::mem::size_of::<libc::proc_bsdinfo>()).ok()?;
    let read = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            (&mut info as *mut libc::proc_bsdinfo).cast(),
            size,
        )
    };
    if read != size {
        return None;
    }
    let bytes = info
        .pbi_name
        .iter()
        .take_while(|byte| **byte != 0)
        .map(|byte| *byte as u8)
        .collect::<Vec<_>>();
    let fallback = info
        .pbi_comm
        .iter()
        .take_while(|byte| **byte != 0)
        .map(|byte| *byte as u8)
        .collect::<Vec<_>>();
    let name = String::from_utf8_lossy(if bytes.is_empty() { &fallback } else { &bytes })
        .trim()
        .to_string();
    let started_at = info
        .pbi_start_tvsec
        .saturating_mul(1_000)
        .saturating_add(info.pbi_start_tvusec / 1_000)
        .min(i64::MAX as u64) as i64;
    Some((info.pbi_ppid as libc::pid_t, name, started_at))
}

#[cfg(target_os = "macos")]
fn process_usage(pid: libc::pid_t) -> Option<(u64, u64, u64)> {
    let mut info = unsafe { std::mem::zeroed::<libc::rusage_info_v2>() };
    let status = unsafe {
        libc::proc_pid_rusage(
            pid,
            libc::RUSAGE_INFO_V2,
            (&mut info as *mut libc::rusage_info_v2).cast::<libc::rusage_info_t>(),
        )
    };
    if status != 0 {
        return None;
    }
    // Darwin reports these CPU counters in nanoseconds. Child counters cover
    // already-reaped descendants; live descendants are sampled separately.
    let own_cpu_time_ns = info.ri_user_time.saturating_add(info.ri_system_time);
    let tree_cpu_time_ns = own_cpu_time_ns
        .saturating_add(info.ri_child_user_time)
        .saturating_add(info.ri_child_system_time);
    Some((own_cpu_time_ns, tree_cpu_time_ns, info.ri_resident_size))
}

#[cfg(target_os = "macos")]
pub fn sample() -> Result<ProcessMetricsSample, String> {
    let root = unsafe { libc::getpid() };
    let mut cpu_time_ns = 0_u64;
    let mut resident_memory_bytes = 0_u64;
    let mut processes = Vec::new();
    for pid in process_tree_pids(root)? {
        // Processes can exit between discovery and sampling. Their CPU usage
        // moves into the parent's child counters once reaped, so skipping a
        // vanished PID is safer than failing the whole widget.
        if let Some((own_cpu, tree_cpu, memory)) = process_usage(pid) {
            cpu_time_ns = cpu_time_ns.saturating_add(tree_cpu);
            resident_memory_bytes = resident_memory_bytes.saturating_add(memory);
            let (parent_pid, name, started_at) = process_identity(pid).unwrap_or((
                0,
                if pid == root { "Monitter" } else { "Process" }.into(),
                0,
            ));
            processes.push(ProcessMetricsProcess {
                pid,
                parent_pid,
                name,
                started_at,
                cpu_time_ms: own_cpu / 1_000_000,
                resident_memory_bytes: memory,
            });
        } else if pid == root {
            return Err("Could not read Monitter process metrics.".into());
        }
    }

    Ok(ProcessMetricsSample {
        cpu_time_ms: cpu_time_ns / 1_000_000,
        resident_memory_bytes,
        sampled_at: crate::model::now(),
        root_pid: root,
        processes,
    })
}

#[cfg(not(target_os = "macos"))]
pub fn sample() -> Result<ProcessMetricsSample, String> {
    Err("Monitter process metrics are currently available on macOS only.".into())
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "macos")]
    #[test]
    fn samples_the_process_tree() {
        let sample = super::sample().expect("process metrics should be readable");
        assert!(sample.sampled_at > 0);
        assert!(sample.resident_memory_bytes > 0);
        assert_eq!(sample.processes[0].pid, sample.root_pid);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn discovers_spawned_child_processes() {
        let mut child = std::process::Command::new("/bin/sleep")
            .arg("2")
            .spawn()
            .expect("spawn child fixture");
        let root = unsafe { libc::getpid() };
        let tree = super::process_tree_pids(root).expect("discover process tree");
        assert!(tree.contains(&(child.id() as libc::pid_t)));
        let sample = super::sample().expect("sample process tree");
        assert!(sample
            .processes
            .iter()
            .any(|process| process.pid == child.id() as libc::pid_t));
        let _ = child.kill();
        let _ = child.wait();
    }
}
