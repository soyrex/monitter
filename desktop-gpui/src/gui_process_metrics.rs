//! Add the independent GUI process to the authority's process-tree sample.
//! Keep its timestamp: the local counters are read immediately after that sample.
use serde_json::Value;

pub(crate) fn include_gui(sample: Value) -> Result<Value, String> {
    #[cfg(target_os = "macos")]
    {
        let sample = normalize_backend(sample, backend_identity)?;
        merge_gui(sample, sample_gui()?)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok(sample)
    }
}

#[cfg(any(target_os = "macos", test))]
struct Identity {
    parent_pid: u32,
    name: String,
    started_at: i64,
}

#[cfg(target_os = "macos")]
fn backend_identity(pid: u32) -> Result<Option<Identity>, String> {
    let pid = libc::pid_t::try_from(pid).map_err(|_| "Invalid backend process ID.".to_owned())?;
    let mut info = unsafe { std::mem::zeroed::<libc::proc_bsdinfo>() };
    let size = libc::c_int::try_from(std::mem::size_of::<libc::proc_bsdinfo>())
        .map_err(|_| "Process identity size is unavailable.".to_owned())?;
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
        let error = std::io::Error::last_os_error();
        if read == 0 && error.raw_os_error() == Some(libc::ESRCH) {
            return Ok(None);
        }
        return Err(format!("Could not inspect backend process {pid}: {error}"));
    }
    if info.pbi_status == libc::SZOMB {
        return Ok(None);
    }
    let name_bytes = if info.pbi_name[0] != 0 {
        &info.pbi_name[..]
    } else {
        &info.pbi_comm[..]
    };
    let name = String::from_utf8_lossy(
        &name_bytes
            .iter()
            .copied()
            .take_while(|byte| *byte != 0)
            .map(|byte| byte as u8)
            .collect::<Vec<_>>(),
    )
    .into_owned();
    Ok(Some(Identity {
        parent_pid: info.pbi_ppid,
        name,
        started_at: info
            .pbi_start_tvsec
            .saturating_mul(1_000)
            .saturating_add(info.pbi_start_tvusec / 1_000)
            .min(i64::MAX as u64) as i64,
    }))
}

/// Older authorities included exited processes with unknown identity and stale RSS.
/// Resolve only those records, then derive memory from the retained live rows.
#[cfg(any(target_os = "macos", test))]
fn normalize_backend(
    mut sample: Value,
    mut resolve: impl FnMut(u32) -> Result<Option<Identity>, String>,
) -> Result<Value, String> {
    let object = sample
        .as_object_mut()
        .ok_or_else(|| "The backend process sample is not an object.".to_owned())?;
    let rows = object
        .get_mut("processes")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| "The backend sample has no process list.".to_owned())?;
    let mut retained = Vec::with_capacity(rows.len());
    let mut memory = 0_u64;
    let mut excluded = 0_u64;
    for mut process in std::mem::take(rows) {
        let unknown = process["parentPid"].as_u64().unwrap_or(0) == 0
            || process["startedAt"].as_i64().unwrap_or(0) == 0;
        if unknown {
            let pid = process["pid"]
                .as_u64()
                .and_then(|pid| u32::try_from(pid).ok())
                .filter(|pid| *pid > 0)
                .ok_or_else(|| "A backend process has no valid process ID.".to_owned())?;
            let Some(identity) = resolve(pid)? else {
                excluded = excluded.saturating_add(1);
                continue;
            };
            process["parentPid"] = Value::from(identity.parent_pid);
            process["name"] = Value::from(identity.name);
            process["startedAt"] = Value::from(identity.started_at);
        }
        memory = memory.saturating_add(
            process["residentMemoryBytes"]
                .as_u64()
                .ok_or_else(|| "A backend process has no resident memory counter.".to_owned())?,
        );
        retained.push(process);
    }
    *rows = retained;
    object.insert("residentMemoryBytes".into(), Value::from(memory));
    object.insert("excludedExitedProcesses".into(), Value::from(excluded));
    Ok(sample)
}

#[cfg(target_os = "macos")]
fn sample_gui() -> Result<Value, String> {
    let pid = libc::pid_t::try_from(std::process::id())
        .map_err(|_| "The GUI process ID cannot be sampled.".to_owned())?;
    let mut identity = unsafe { std::mem::zeroed::<libc::proc_bsdinfo>() };
    let identity_size = libc::c_int::try_from(std::mem::size_of::<libc::proc_bsdinfo>())
        .map_err(|_| "The GUI process identity size is unavailable.".to_owned())?;
    let read = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            (&mut identity as *mut libc::proc_bsdinfo).cast(),
            identity_size,
        )
    };
    if read != identity_size || identity.pbi_pid != pid as u32 {
        return Err(format!(
            "Could not sample GUI process identity {pid}: {}",
            std::io::Error::last_os_error()
        ));
    }
    let mut usage = unsafe { std::mem::zeroed::<libc::rusage_info_v2>() };
    let status = unsafe {
        libc::proc_pid_rusage(
            pid,
            libc::RUSAGE_INFO_V2,
            (&mut usage as *mut libc::rusage_info_v2).cast::<libc::rusage_info_t>(),
        )
    };
    if status != 0 {
        return Err(format!(
            "Could not sample GUI process usage {pid}: {}",
            std::io::Error::last_os_error()
        ));
    }
    let executable = std::env::current_exe()
        .map_err(|error| format!("Could not identify the GUI executable: {error}"))?;
    let started_at = identity
        .pbi_start_tvsec
        .saturating_mul(1_000)
        .saturating_add(identity.pbi_start_tvusec / 1_000)
        .min(i64::MAX as u64) as i64;
    // Only this process's own counters: GUI descendants are not part of this row.
    let cpu_time_ms = usage.ri_user_time.saturating_add(usage.ri_system_time) / 1_000_000;
    Ok(serde_json::json!({
        "pid": pid,
        "parentPid": identity.pbi_ppid,
        "startedAt": started_at,
        "role": "gui",
        "name": "Monitter GPUI",
        "commandLine": executable.to_string_lossy(),
        "cpuTimeMs": cpu_time_ms,
        "residentMemoryBytes": usage.ri_resident_size,
    }))
}

#[cfg(any(target_os = "macos", test))]
fn merge_gui(mut sample: Value, gui: Value) -> Result<Value, String> {
    let pid = gui["pid"]
        .as_u64()
        .filter(|pid| *pid > 0)
        .ok_or_else(|| "The GUI sample has no valid process ID.".to_owned())?;
    let gui_cpu = gui["cpuTimeMs"]
        .as_u64()
        .ok_or_else(|| "The GUI sample has no CPU counter.".to_owned())?;
    let gui_memory = gui["residentMemoryBytes"]
        .as_u64()
        .ok_or_else(|| "The GUI sample has no resident memory counter.".to_owned())?;
    let object = sample
        .as_object_mut()
        .ok_or_else(|| "The backend process sample is not an object.".to_owned())?;
    let cpu = object
        .get("cpuTimeMs")
        .and_then(Value::as_u64)
        .ok_or_else(|| "The backend sample has no CPU counter.".to_owned())?;
    let memory = object
        .get("residentMemoryBytes")
        .and_then(Value::as_u64)
        .ok_or_else(|| "The backend sample has no resident memory counter.".to_owned())?;
    let processes = object
        .get_mut("processes")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| "The backend sample has no process list.".to_owned())?;
    let present = if let Some(existing) = processes
        .iter_mut()
        .find(|process| process["pid"].as_u64() == Some(pid))
    {
        existing["role"] = Value::String("gui".into());
        true
    } else {
        processes.push(gui);
        false
    };
    object.insert("guiPid".into(), Value::from(pid));
    if !present {
        object.insert("cpuTimeMs".into(), Value::from(cpu.saturating_add(gui_cpu)));
        object.insert(
            "residentMemoryBytes".into(),
            Value::from(memory.saturating_add(gui_memory)),
        );
    }
    Ok(sample)
}

#[cfg(test)]
mod tests {
    use super::merge_gui;
    use serde_json::json;

    fn gui() -> serde_json::Value {
        json!({"pid":7,"parentPid":1,"startedAt":1000,"role":"gui","name":"Monitter GPUI","commandLine":"/test/Monitter GPUI","cpuTimeMs":30,"residentMemoryBytes":400})
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn reads_native_gui_usage_without_changing_sample_timestamp() {
        let sample = json!({"rootPid":2,"sampledAt":1234,"cpuTimeMs":100,"residentMemoryBytes":0,"processes":[]});
        let merged = super::include_gui(sample).expect("sample current GUI process");
        assert_eq!(merged["guiPid"], std::process::id());
        assert_eq!(merged["sampledAt"], 1234);
        assert!(merged["residentMemoryBytes"].as_u64().unwrap() > 0);
        assert_eq!(merged["processes"][0]["role"], "gui");
    }

    #[test]
    fn adds_gui_counters_once_and_preserves_backend_timestamp_and_identity() {
        let sample = json!({"rootPid":2,"sampledAt":1234,"cpuTimeMs":100,"residentMemoryBytes":200,"processes":[{"pid":2}]});
        let merged = merge_gui(sample, gui()).unwrap();
        assert_eq!(merged["cpuTimeMs"], 130);
        assert_eq!(merged["residentMemoryBytes"], 600);
        assert_eq!(merged["sampledAt"], 1234);
        assert_eq!(merged["rootPid"], 2);
        assert_eq!(merged["guiPid"], 7);
        assert_eq!(merged["processes"].as_array().unwrap().len(), 2);
        assert_eq!(merged["processes"][1], gui());
        assert_eq!(merge_gui(merged.clone(), gui()).unwrap(), merged);
    }

    #[test]
    fn existing_gui_pid_only_gets_role_without_replacing_counters_or_identity() {
        let sample = json!({"rootPid":2,"sampledAt":1234,"cpuTimeMs":100,"residentMemoryBytes":200,"processes":[{"pid":7,"name":"existing","cpuTimeMs":5,"residentMemoryBytes":8}]});
        let mut expected = sample.clone();
        expected["guiPid"] = json!(7);
        expected["processes"][0]["role"] = json!("gui");
        assert_eq!(merge_gui(sample, gui()).unwrap(), expected);
    }

    #[test]
    fn totals_saturate_and_malformed_samples_are_rejected() {
        let sample = json!({"cpuTimeMs":u64::MAX,"residentMemoryBytes":u64::MAX,"processes":[]});
        let merged = merge_gui(sample, gui()).unwrap();
        assert_eq!(merged["cpuTimeMs"].as_u64(), Some(u64::MAX));
        assert_eq!(merged["residentMemoryBytes"].as_u64(), Some(u64::MAX));
        assert!(merge_gui(json!({}), gui()).is_err());
        assert!(merge_gui(json!(null), gui()).is_err());
    }

    #[test]
    fn normalization_removes_exits_repairs_live_identity_and_recomputes_only_memory() {
        let sample = json!({"sampledAt":1234,"rootPid":2,"cpuTimeMs":900,"residentMemoryBytes":9999,"processes":[
            {"pid":2,"parentPid":1,"startedAt":50,"name":"authority","residentMemoryBytes":100},
            {"pid":3,"parentPid":0,"startedAt":0,"name":"Process","residentMemoryBytes":8000},
            {"pid":4,"parentPid":0,"startedAt":0,"name":"Process","residentMemoryBytes":200}
        ]});
        let mut resolved = Vec::new();
        let normalized = super::normalize_backend(sample, |pid| {
            resolved.push(pid);
            Ok(if pid == 3 {
                None
            } else {
                Some(super::Identity {
                    parent_pid: 2,
                    name: "live worker".into(),
                    started_at: 80,
                })
            })
        })
        .unwrap();
        assert_eq!(resolved, vec![3, 4]);
        assert_eq!(normalized["cpuTimeMs"], 900);
        assert_eq!(normalized["residentMemoryBytes"], 300);
        assert_eq!(normalized["sampledAt"], 1234);
        assert_eq!(normalized["excludedExitedProcesses"], 1);
        assert_eq!(normalized["processes"][1]["name"], "live worker");
        assert_eq!(normalized["processes"][1]["parentPid"], 2);
        assert_eq!(normalized["processes"][1]["startedAt"], 80);
        let merged = merge_gui(normalized, gui()).unwrap();
        assert_eq!(merged["cpuTimeMs"], 930);
        assert_eq!(merged["residentMemoryBytes"], 700);
    }
}
