use super::types::{ProcessIdentity, ProcessObservation, ResourceSample, RunRoot};
use std::collections::{BTreeMap, HashMap, HashSet};

const MAX_COVERED_MS: u64 = 30_000;

/// Stateful, filesystem-free accounting of sanitized process counters.
#[derive(Default)]
pub struct ResourceTracker {
    previous: HashMap<ProcessIdentity, (u64, String)>,
    previous_members: HashMap<String, HashSet<ProcessIdentity>>,
    last_utc_ms: Option<i64>,
    last_monotonic_ms: Option<u64>,
    serial: u64,
}

impl ResourceTracker {
    pub fn observe(
        &mut self,
        utc_ms: i64,
        monotonic_ms: u64,
        roots: &[RunRoot],
        processes: &[ProcessObservation],
    ) -> Vec<ResourceSample> {
        let mut out = Vec::new();
        let mut remote_runs: Vec<_> = roots
            .iter()
            .filter(|r| r.remote)
            .map(|r| r.run_id.clone())
            .collect();
        remote_runs.sort();
        remote_runs.dedup();
        for run_id in remote_runs {
            out.push(self.sample(
                &run_id,
                utc_ms,
                utc_ms,
                0,
                None,
                None,
                None,
                None,
                0,
                "remoteUnavailable",
            ));
        }
        let has_baseline = self.last_utc_ms.is_some() && self.last_monotonic_ms.is_some();
        let discontinuity = match (self.last_utc_ms, self.last_monotonic_ms) {
            (Some(previous_utc), Some(previous_mono)) => {
                let wall = utc_ms.saturating_sub(previous_utc);
                let mono = monotonic_ms.saturating_sub(previous_mono);
                wall < 0
                    || mono == 0
                    || mono > MAX_COVERED_MS
                    || wall.abs_diff(i64::try_from(mono).unwrap_or(i64::MAX)) > MAX_COVERED_MS
            }
            _ => true,
        };
        let mut live: HashMap<ProcessIdentity, &ProcessObservation> = HashMap::new();
        for p in processes {
            live.entry(p.identity.clone()).or_insert(p);
        }
        let ownership = assign_roots(roots, &live);
        let mut run_processes: BTreeMap<String, Vec<&ProcessObservation>> = BTreeMap::new();
        for (identity, owner) in ownership {
            if let Some(owner) = owner {
                if let Some(p) = live.get(&identity) {
                    run_processes.entry(owner).or_default().push(*p);
                }
            }
        }
        let covered = self
            .last_monotonic_ms
            .map(|old| monotonic_ms.saturating_sub(old))
            .unwrap_or(0);
        let mut current_members: HashMap<String, HashSet<ProcessIdentity>> = HashMap::new();
        for (run_id, members) in &run_processes {
            current_members.insert(
                run_id.clone(),
                members.iter().map(|p| p.identity.clone()).collect(),
            );
        }
        for (run_id, members) in run_processes {
            let mut cpu_delta = 0_u64;
            let mut has_delta = false;
            let mut cpu_overflow = false;
            let mut memory = 0_u64;
            let mut rss_overflow = false;
            let mut rss_byte_ms_overflow = false;
            let mut reset = false;
            let mut owner_changed = !has_baseline;
            for p in &members {
                if let Some(total) = memory.checked_add(p.resident_memory_bytes) {
                    memory = total;
                } else {
                    rss_overflow = true;
                }
                if self.previous.keys().any(|old| {
                    old.host_id == p.identity.host_id
                        && old.pid == p.identity.pid
                        && old.started_at != p.identity.started_at
                }) {
                    reset = true;
                }
                if let Some((old_cpu, old_owner)) = self.previous.get(&p.identity) {
                    if old_owner == &run_id && p.cpu_time_ms >= *old_cpu {
                        if let Some(total) = cpu_delta.checked_add(p.cpu_time_ms - *old_cpu) {
                            cpu_delta = total;
                            has_delta = true;
                        } else {
                            cpu_overflow = true;
                        }
                    } else if old_owner != &run_id {
                        owner_changed = true;
                    } else {
                        reset = true;
                    }
                } else {
                    owner_changed = true;
                }
            }
            let gap = discontinuity || reset;
            let rss_byte_ms = if gap || rss_overflow {
                None
            } else {
                match memory.checked_mul(covered) {
                    Some(value) => Some(value),
                    None => {
                        rss_byte_ms_overflow = true;
                        None
                    }
                }
            };
            if self.previous_members.get(&run_id).is_some_and(|old| {
                current_members
                    .get(&run_id)
                    .is_some_and(|current| old != current)
            }) {
                owner_changed = true;
            }
            let mut quality_parts = Vec::new();
            if has_baseline && discontinuity {
                quality_parts.push("clockGap");
            }
            if reset {
                quality_parts.push("counterReset");
            }
            if cpu_overflow {
                quality_parts.push("cpuOverflow");
            }
            if rss_overflow {
                quality_parts.push("rssOverflow");
            }
            if rss_byte_ms_overflow {
                quality_parts.push("rssByteMsOverflow");
            }
            if !has_baseline {
                quality_parts.push("baseline");
            } else if owner_changed {
                quality_parts.push("partialMembership");
            }
            if quality_parts.is_empty() {
                quality_parts.push("observed");
            }
            let quality = quality_parts.join("+");
            let sample_covered = if gap { 0 } else { covered };
            out.push(self.sample(
                &run_id,
                utc_ms,
                utc_ms.saturating_sub(i64::try_from(sample_covered).unwrap_or(i64::MAX)),
                sample_covered,
                if gap || cpu_overflow || !has_delta {
                    None
                } else {
                    Some(cpu_delta)
                },
                (!rss_overflow).then_some(memory),
                rss_byte_ms,
                (!rss_overflow).then_some(memory),
                members.len() as u32,
                &quality,
            ));
        }
        // Every active local root gets an explicit sample when ownership cannot
        // be established, so absence of a row is never mistaken for zero use.
        let mut root_ids: HashMap<ProcessIdentity, usize> = HashMap::new();
        for root in roots.iter().filter(|r| !r.remote) {
            *root_ids.entry(root.identity.clone()).or_default() += 1;
        }
        let mut reported = HashSet::new();
        for root in roots.iter().filter(|r| !r.remote) {
            if !reported.insert(root.run_id.clone()) || current_members.contains_key(&root.run_id) {
                continue;
            }
            let quality = if !live.contains_key(&root.identity) {
                "rootUnavailable"
            } else if root_ids.get(&root.identity).copied().unwrap_or(0) > 1 {
                "ambiguousRoot"
            } else {
                "unattributed"
            };
            out.push(self.sample(
                &root.run_id,
                utc_ms,
                utc_ms,
                0,
                None,
                None,
                None,
                None,
                0,
                quality,
            ));
        }
        self.previous.clear();
        for (identity, owner) in assign_roots(roots, &live) {
            if let Some(owner) = owner {
                if let Some(p) = live.get(&identity) {
                    self.previous.insert(identity, (p.cpu_time_ms, owner));
                }
            }
        }
        self.previous_members = current_members;
        self.last_utc_ms = Some(utc_ms);
        self.last_monotonic_ms = Some(monotonic_ms);
        out.sort_by(|a, b| {
            a.run_id
                .cmp(&b.run_id)
                .then(a.sampled_at.cmp(&b.sampled_at))
        });
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn sample(
        &mut self,
        run_id: &str,
        at: i64,
        start: i64,
        covered_ms: u64,
        cpu_delta_ms: Option<u64>,
        rss_bytes: Option<u64>,
        rss_byte_ms: Option<u64>,
        peak: Option<u64>,
        process_count: u32,
        quality: &str,
    ) -> ResourceSample {
        self.serial = self.serial.saturating_add(1);
        ResourceSample {
            sample_id: format!("resource:{}:{}:{}", run_id, at, self.serial),
            run_id: run_id.into(),
            sampled_at: at,
            interval_start: start,
            covered_ms,
            cpu_delta_ms,
            rss_bytes,
            rss_byte_ms,
            sampled_peak_rss_bytes: peak,
            process_count,
            quality: quality.into(),
        }
    }
}

/// Assign each observed process to its nearest verified root. Equal-depth or
/// multiply-owned roots are intentionally left unattributed.
fn assign_roots(
    roots: &[RunRoot],
    live: &HashMap<ProcessIdentity, &ProcessObservation>,
) -> HashMap<ProcessIdentity, Option<String>> {
    let local: Vec<_> = roots
        .iter()
        .filter(|r| !r.remote && live.contains_key(&r.identity))
        .collect();
    let mut output = HashMap::new();
    for identity in live.keys() {
        let mut candidates: Vec<(usize, String)> = Vec::new();
        for root in &local {
            if root.identity.host_id != identity.host_id {
                continue;
            }
            if let Some(depth) = ancestry_depth(identity, &root.identity, live) {
                candidates.push((depth, root.run_id.clone()));
            }
        }
        candidates.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        let owner = candidates.first().and_then(|(depth, run)| {
            if candidates.iter().filter(|(d, _)| d == depth).count() == 1 {
                Some(run.clone())
            } else {
                None
            }
        });
        output.insert(identity.clone(), owner);
    }
    output
}

fn ancestry_depth(
    identity: &ProcessIdentity,
    root: &ProcessIdentity,
    live: &HashMap<ProcessIdentity, &ProcessObservation>,
) -> Option<usize> {
    let mut cursor = identity.clone();
    let mut depth = 0usize;
    let mut visited = HashSet::new();
    loop {
        if &cursor == root {
            return Some(depth);
        }
        if !visited.insert(cursor.clone()) {
            return None;
        }
        let process = live.get(&cursor)?;
        let parent = process.parent_pid;
        if parent <= 0 {
            return None;
        }
        cursor = live
            .keys()
            .find(|candidate| candidate.host_id == cursor.host_id && candidate.pid == parent)?
            .clone();
        depth += 1;
        if depth > live.len() {
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn id(pid: i32, start: i64) -> ProcessIdentity {
        ProcessIdentity {
            host_id: "h".into(),
            pid,
            started_at: start,
        }
    }
    fn process(pid: i32, start: i64, parent: i32, cpu: u64, rss: u64) -> ProcessObservation {
        ProcessObservation {
            identity: id(pid, start),
            parent_pid: parent,
            cpu_time_ms: cpu,
            resident_memory_bytes: rss,
        }
    }
    fn root(run: &str, pid: i32, start: i64) -> RunRoot {
        RunRoot {
            run_id: run.into(),
            identity: id(pid, start),
            remote: false,
        }
    }
    #[test]
    fn attributes_nearest_root_and_tracks_membership_and_pid_reuse() {
        let mut tracker = ResourceTracker::default();
        let roots = [root("a", 1, 1), root("child", 2, 2)];
        let first = [
            process(1, 1, 0, 100, 10),
            process(2, 2, 1, 40, 20),
            process(3, 3, 2, 5, 30),
        ];
        let baseline = tracker.observe(1000, 1000, &roots, &first);
        assert_eq!(baseline.len(), 2);
        assert!(baseline.iter().all(|s| s.quality == "baseline"));
        let second = [
            process(1, 1, 0, 110, 11),
            process(2, 2, 1, 45, 21),
            process(3, 3, 2, 9, 31),
            process(4, 4, 2, 3, 40),
        ];
        let samples = tracker.observe(2000, 2000, &roots, &second);
        let child = samples.iter().find(|x| x.run_id == "child").unwrap();
        assert_eq!(child.process_count, 3);
        assert_eq!(child.cpu_delta_ms, Some(9));
        assert_eq!(child.rss_bytes, Some(92));
        let reused = [process(1, 99, 0, 2, 10)];
        let gap = tracker.observe(3000, 3000, &[root("a", 1, 99)], &reused);
        assert!(gap[0].quality.contains("counterReset"));
        assert_eq!(gap[0].cpu_delta_ms, None);
    }
    #[test]
    fn long_sleep_and_remote_are_gaps_or_unavailable() {
        let mut tracker = ResourceTracker::default();
        let local = root("local", 1, 1);
        let remote = RunRoot {
            run_id: "remote".into(),
            remote: true,
            ..root("remote", 9, 9)
        };
        tracker.observe(1_000, 1_000, &[local.clone()], &[process(1, 1, 0, 1, 5)]);
        let samples = tracker.observe(50_000, 50_000, &[local, remote], &[process(1, 1, 0, 20, 9)]);
        assert!(samples.iter().any(|s| s.run_id == "remote"
            && s.quality == "remoteUnavailable"
            && s.cpu_delta_ms.is_none()));
        assert!(samples
            .iter()
            .any(|s| s.run_id == "local" && s.quality == "clockGap" && s.covered_ms == 0));
    }

    #[test]
    fn new_runs_and_transferred_children_never_inherit_foreign_cpu() {
        let mut tracker = ResourceTracker::default();
        let parent = root("parent", 1, 1);
        let processes = [process(1, 1, 0, 100, 10), process(2, 2, 1, 50, 20)];
        tracker.observe(1_000, 1_000, &[parent.clone()], &processes);

        // A run appears after its root has already accumulated CPU. That prior
        // interval belongs to the parent and must not be charged to the child.
        let roots = [parent.clone(), root("child", 2, 2)];
        let next = [process(1, 1, 0, 110, 12), process(2, 2, 1, 80, 22)];
        let samples = tracker.observe(2_000, 2_000, &roots, &next);
        let child = samples.iter().find(|s| s.run_id == "child").unwrap();
        assert_eq!(child.cpu_delta_ms, None);
        assert_eq!(child.quality, "partialMembership");
        assert_eq!(child.sampled_peak_rss_bytes, Some(22));

        let grandchildren = [
            process(1, 1, 0, 120, 14),
            process(2, 2, 1, 90, 23),
            process(3, 3, 2, 12, 31),
        ];
        let moved = tracker.observe(3_000, 3_000, &roots, &grandchildren);
        let child = moved.iter().find(|s| s.run_id == "child").unwrap();
        assert_eq!(child.cpu_delta_ms, Some(10));
        assert_eq!(child.rss_bytes, Some(54));
        assert_eq!(child.sampled_peak_rss_bytes, Some(54));
        assert_eq!(child.quality, "partialMembership");
    }

    #[test]
    fn missing_and_ambiguous_roots_emit_coverage_and_reordered_clock_rebaselines() {
        let mut tracker = ResourceTracker::default();
        let missing = tracker.observe(1_000, 1_000, &[root("gone", 8, 8)], &[]);
        assert_eq!(missing[0].quality, "rootUnavailable");

        let shared = root("shared-a", 1, 1);
        let same = root("shared-b", 1, 1);
        let observation = process(1, 1, 0, 10, 4);
        let ambiguous = tracker.observe(2_000, 2_000, &[shared, same], &[observation.clone()]);
        assert_eq!(
            ambiguous
                .iter()
                .filter(|s| s.quality == "ambiguousRoot")
                .count(),
            2
        );

        let valid_root = root("run", 1, 1);
        tracker.observe(3_000, 3_000, &[valid_root.clone()], &[observation]);
        let reordered = tracker.observe(
            4_000,
            2_500,
            &[valid_root.clone()],
            &[process(1, 1, 0, 20, 5)],
        );
        assert_eq!(reordered[0].quality, "clockGap");
        assert_eq!(reordered[0].covered_ms, 0);
        let recovered = tracker.observe(5_000, 3_500, &[valid_root], &[process(1, 1, 0, 30, 6)]);
        assert_eq!(recovered[0].quality, "observed");
        assert_eq!(recovered[0].cpu_delta_ms, Some(10));
    }

    #[test]
    fn cpu_aggregation_overflow_is_nullable_and_explicit() {
        let mut tracker = ResourceTracker::default();
        let root = root("run", 1, 1);
        tracker.observe(
            1_000,
            1_000,
            std::slice::from_ref(&root),
            &[process(1, 1, 0, 0, 10), process(2, 2, 1, 0, 20)],
        );
        let samples = tracker.observe(
            2_000,
            2_000,
            &[root],
            &[
                process(1, 1, 0, u64::MAX, 11),
                process(2, 2, 1, u64::MAX, 21),
            ],
        );
        assert_eq!(samples[0].cpu_delta_ms, None);
        assert!(samples[0].quality.contains("cpuOverflow"));
        assert_eq!(samples[0].rss_bytes, Some(32));
    }

    #[test]
    fn rss_sum_and_byte_millisecond_overflow_never_saturate_to_fake_totals() {
        let mut tracker = ResourceTracker::default();
        let sum_root = root("sum", 1, 1);
        let high = u64::MAX / 2 + 1;
        tracker.observe(
            1_000,
            1_000,
            std::slice::from_ref(&sum_root),
            &[process(1, 1, 0, 0, u64::MAX), process(2, 2, 1, 0, 1)],
        );
        let summed = tracker.observe(
            2_000,
            2_000,
            std::slice::from_ref(&sum_root),
            &[process(1, 1, 0, 1, u64::MAX), process(2, 2, 1, 1, 1)],
        );
        assert_eq!(summed[0].rss_bytes, None);
        assert_eq!(summed[0].sampled_peak_rss_bytes, None);
        assert_eq!(summed[0].rss_byte_ms, None);
        assert!(summed[0].quality.contains("rssOverflow"));

        let mut area_tracker = ResourceTracker::default();
        let area_root = root("area", 3, 3);
        area_tracker.observe(
            1_000,
            1_000,
            std::slice::from_ref(&area_root),
            &[process(3, 3, 0, 0, high)],
        );
        let area = area_tracker.observe(2_000, 2_000, &[area_root], &[process(3, 3, 0, 1, high)]);
        assert_eq!(area[0].rss_bytes, Some(high));
        assert_eq!(area[0].rss_byte_ms, None);
        assert!(area[0].quality.contains("rssByteMsOverflow"));
    }

    #[test]
    fn a_fresh_tracker_after_restart_establishes_a_zero_coverage_baseline() {
        let run = root("restart", 1, 1);
        let first = process(1, 1, 0, 100, 64);
        let mut before_restart = ResourceTracker::default();
        let original = before_restart.observe(
            1_000,
            1_000,
            std::slice::from_ref(&run),
            std::slice::from_ref(&first),
        );
        assert_eq!(original[0].quality, "baseline");

        let mut after_restart = ResourceTracker::default();
        let restarted =
            after_restart.observe(90_000, 90_000, &[run], &[process(1, 1, 0, 9_000, 80)]);
        assert_eq!(restarted[0].quality, "baseline");
        assert_eq!(restarted[0].covered_ms, 0);
        assert_eq!(restarted[0].cpu_delta_ms, None);
        assert_eq!(restarted[0].rss_bytes, Some(80));
        assert_eq!(restarted[0].rss_byte_ms, None);
    }
}
