use serde::Serialize;

#[cfg(target_os = "macos")]
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProcessMetricsSample {
    pub cpu_time_ms: u64,
    pub resident_memory_bytes: u64,
    pub sampled_at: i64,
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
fn process_usage(pid: libc::pid_t) -> Option<(u64, u64)> {
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
    let cpu_time_ns = info
        .ri_user_time
        .saturating_add(info.ri_system_time)
        .saturating_add(info.ri_child_user_time)
        .saturating_add(info.ri_child_system_time);
    Some((cpu_time_ns, info.ri_resident_size))
}

#[cfg(target_os = "macos")]
pub fn sample() -> Result<ProcessMetricsSample, String> {
    let root = unsafe { libc::getpid() };
    let root_usage = process_usage(root)
        .ok_or_else(|| "Could not read Monitter process metrics.".to_string())?;
    let mut cpu_time_ns = root_usage.0;
    let mut resident_memory_bytes = root_usage.1;

    for pid in process_tree_pids(root)?
        .into_iter()
        .filter(|pid| *pid != root)
    {
        // Processes can exit between discovery and sampling. Their CPU usage
        // moves into the parent's child counters once reaped, so skipping a
        // vanished PID is safer than failing the whole widget.
        if let Some((cpu, memory)) = process_usage(pid) {
            cpu_time_ns = cpu_time_ns.saturating_add(cpu);
            resident_memory_bytes = resident_memory_bytes.saturating_add(memory);
        }
    }

    Ok(ProcessMetricsSample {
        cpu_time_ms: cpu_time_ns / 1_000_000,
        resident_memory_bytes,
        sampled_at: crate::model::now(),
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
        let _ = child.kill();
        let _ = child.wait();
    }
}
