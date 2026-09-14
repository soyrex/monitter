use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProcessMetricsSample {
    pub cpu_time_ms: u64,
    pub resident_memory_bytes: u64,
    pub sampled_at: i64,
}

#[cfg(target_os = "macos")]
pub fn sample() -> Result<ProcessMetricsSample, String> {
    let mut info = unsafe { std::mem::zeroed::<libc::mach_task_basic_info_data_t>() };
    let mut count = libc::MACH_TASK_BASIC_INFO_COUNT;
    #[allow(deprecated)]
    let task = unsafe { libc::mach_task_self() };
    let status = unsafe {
        libc::task_info(
            task,
            libc::MACH_TASK_BASIC_INFO,
            (&mut info as *mut libc::mach_task_basic_info_data_t).cast::<libc::integer_t>(),
            &mut count,
        )
    };
    if status != libc::KERN_SUCCESS {
        return Err(format!(
            "Could not read Monitter process metrics ({status})."
        ));
    }

    // Mach's task structure is packed, so copy its fields without creating
    // potentially unaligned references.
    let resident_memory_bytes = unsafe { std::ptr::addr_of!(info.resident_size).read_unaligned() };
    let user_time = unsafe { std::ptr::addr_of!(info.user_time).read_unaligned() };
    let system_time = unsafe { std::ptr::addr_of!(info.system_time).read_unaligned() };
    let time_ms = |value: libc::time_value_t| {
        (value.seconds.max(0) as u64)
            .saturating_mul(1_000)
            .saturating_add(value.microseconds.max(0) as u64 / 1_000)
    };

    Ok(ProcessMetricsSample {
        cpu_time_ms: time_ms(user_time).saturating_add(time_ms(system_time)),
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
    fn samples_the_current_process() {
        let sample = super::sample().expect("process metrics should be readable");
        assert!(sample.sampled_at > 0);
        assert!(sample.resident_memory_bytes > 0);
    }
}
