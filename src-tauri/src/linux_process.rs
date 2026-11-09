//! Read-only Linux process accounting, shared by the authority and independent GUI.
//! /proc/PID/stat CPU counters are clock ticks; RSS is in system pages.
use std::{
    collections::{HashMap, HashSet},
    fs,
};

pub struct Process {
    pub pid: i32,
    pub parent_pid: i32,
    pub name: String,
    pub command_line: String,
    pub started_at: i64,
    pub cpu_time_ms: u64,
    pub resident_memory_bytes: u64,
}
struct Config {
    ticks: u64,
    page: u64,
    boot: u64,
}
impl Config {
    fn read() -> Result<Self, String> {
        let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
        let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        if ticks <= 0 || page <= 0 {
            return Err("Could not read process clock or page size.".into());
        }
        let stat = fs::read_to_string("/proc/stat")
            .map_err(|e| format!("Could not read boot time: {e}"))?;
        let boot = stat
            .lines()
            .find_map(|l| l.strip_prefix("btime ")?.trim().parse().ok())
            .ok_or("Could not read boot time.")?;
        Ok(Self {
            ticks: ticks as u64,
            page: page as u64,
            boot,
        })
    }
    fn ms(&self, ticks: u64) -> u64 {
        ((ticks as u128 * 1000) / self.ticks as u128).min(u64::MAX as u128) as u64
    }
}
struct Stat {
    pid: i32,
    ppid: i32,
    name: String,
    state: char,
    own: u64,
    tree: u64,
    start: u64,
    rss: u64,
}
fn parse_stat(text: &str) -> Option<Stat> {
    let left = text.find('(')?;
    // comm can itself contain spaces and parentheses.
    let right = text.rfind(')')?;
    if right <= left {
        return None;
    }
    let pid = text[..left].trim().parse().ok()?;
    let name = text[left + 1..right].to_owned();
    let f: Vec<&str> = text[right + 1..].split_whitespace().collect();
    let state = f.first()?.chars().next()?;
    let ppid = f.get(1)?.parse().ok()?;
    let user: u64 = f.get(11)?.parse().ok()?;
    let system: u64 = f.get(12)?.parse().ok()?;
    let child_user: i64 = f.get(13)?.parse().ok()?;
    let child_system: i64 = f.get(14)?.parse().ok()?;
    let start = f.get(19)?.parse().ok()?;
    let rss: i64 = f.get(21)?.parse().ok()?;
    let own = user.saturating_add(system);
    let tree = own
        .saturating_add(child_user.max(0) as u64)
        .saturating_add(child_system.max(0) as u64);
    Some(Stat {
        pid,
        ppid,
        name,
        state,
        own,
        tree,
        start,
        rss: rss.max(0) as u64,
    })
}
fn read_stat(pid: i32) -> Result<Stat, String> {
    let text = fs::read_to_string(format!("/proc/{pid}/stat"))
        .map_err(|e| format!("Could not read process {pid}: {e}"))?;
    let stat = parse_stat(&text).ok_or_else(|| format!("Could not parse process {pid}."))?;
    if stat.pid != pid {
        return Err(format!("Invalid identity for process {pid}."));
    }
    Ok(stat)
}
fn live(s: &Stat) -> bool {
    !matches!(s.state, 'Z' | 'X' | 'x')
}
fn row(s: &Stat, c: &Config) -> Process {
    let command_line = fs::read(format!("/proc/{}/cmdline", s.pid))
        .ok()
        .map(|b| {
            b.split(|x| *x == 0)
                .filter(|x| !x.is_empty())
                .map(|x| String::from_utf8_lossy(x).into_owned())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| s.name.clone());
    Process {
        pid: s.pid,
        parent_pid: s.ppid,
        name: s.name.clone(),
        command_line,
        started_at: c
            .boot
            .saturating_mul(1000)
            .saturating_add(c.ms(s.start))
            .min(i64::MAX as u64) as i64,
        cpu_time_ms: c.ms(s.own),
        resident_memory_bytes: s.rss.saturating_mul(c.page),
    }
}
// Each consumer uses only one entry point; both share the same accounting rules.
#[allow(dead_code)]
pub fn sample_process(pid: i32) -> Result<Process, String> {
    let c = Config::read()?;
    let s = read_stat(pid)?;
    if !live(&s) {
        return Err(format!("Process {pid} has exited."));
    }
    Ok(row(&s, &c))
}
#[allow(dead_code)]
pub fn sample_tree(root: i32) -> Result<(u64, Vec<Process>), String> {
    let c = Config::read()?;
    let entries = fs::read_dir("/proc").map_err(|e| format!("Could not list processes: {e}"))?;
    let mut stats = HashMap::new();
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|s| s.parse::<i32>().ok())
            .filter(|p| *p > 0)
        else {
            continue;
        };
        // Descendants may exit while enumerating. Require the root below.
        if let Ok(s) = read_stat(pid) {
            stats.insert(pid, s);
        }
    }
    if !stats.get(&root).is_some_and(live) {
        return Err("Could not read Monitter process metrics.".into());
    }
    let mut children: HashMap<i32, Vec<i32>> = HashMap::new();
    for s in stats.values() {
        children.entry(s.ppid).or_default().push(s.pid);
    }
    let mut pending = vec![root];
    let mut seen = HashSet::new();
    let mut total = 0_u64;
    let mut rows = Vec::new();
    while let Some(pid) = pending.pop() {
        if !seen.insert(pid) {
            continue;
        }
        let Some(s) = stats.get(&pid) else { continue };
        // Waited-for child time survives reaping; unreaped zombies retain their
        // own CPU here, but never contribute live rows or resident memory.
        total = total.saturating_add(s.tree);
        if live(s) {
            rows.push(row(s, &c));
        }
        if let Some(kids) = children.get(&pid) {
            pending.extend(kids);
        }
    }
    Ok((c.ms(total), rows))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_tricky_names_and_converts_kernel_units() {
        let mut fields = vec!["0"; 22];
        fields[0] = "S";
        fields[1] = "7";
        fields[11] = "120";
        fields[12] = "30";
        fields[13] = "40";
        fields[14] = "10";
        fields[19] = "250";
        fields[21] = "9";
        let s = parse_stat(&format!("42 (name ) with space) {}", fields.join(" "))).unwrap();
        assert_eq!(
            (s.pid, s.ppid, s.own, s.tree, s.start, s.rss),
            (42, 7, 150, 200, 250, 9)
        );
        assert_eq!(s.name, "name ) with space");
        let c = Config {
            ticks: 100,
            page: 4096,
            boot: 1000,
        };
        let p = row(&s, &c);
        assert_eq!(
            (p.cpu_time_ms, p.resident_memory_bytes, p.started_at),
            (1500, 36864, 1002500)
        );
        assert!(parse_stat("1 (broken) S 0").is_none());
        fields[11] = "-1";
        assert!(parse_stat(&format!("42 (bad) {}", fields.join(" "))).is_none());
    }
    #[test]
    fn cpu_time_survives_child_reaping() {
        let root = std::process::id() as i32;
        let mut child = std::process::Command::new("/bin/sh")
            .args([
                "-c",
                "i=0; while [ \"$i\" -lt 100000 ]; do i=$((i+1)); done",
            ])
            .spawn()
            .unwrap();
        let pid = child.id() as i32;
        let mut exited = false;
        for _ in 0..500 {
            if read_stat(pid).is_ok_and(|s| !live(&s)) {
                exited = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let before = sample_tree(root);
        let child_cpu = read_stat(pid).map(|s| s.own);
        if !exited {
            let _ = child.kill();
        }
        let _ = child.wait();
        assert!(exited, "CPU fixture should exit within five seconds");
        assert!(
            child_cpu.unwrap() > 0,
            "fixture must consume measurable CPU"
        );
        let (before_cpu, rows) = before.unwrap();
        assert!(!rows.iter().any(|p| p.pid == pid));
        let (after_cpu, _) = sample_tree(root).unwrap();
        assert!(
            after_cpu >= before_cpu,
            "waited child CPU must remain in the total"
        );
    }

    #[test]
    fn samples_live_child_and_excludes_zombie() {
        let mut child = std::process::Command::new("/bin/sleep")
            .arg("10")
            .spawn()
            .unwrap();
        let pid = child.id() as i32;
        let root = std::process::id() as i32;
        let (_, rows) = sample_tree(root).unwrap();
        let found = rows
            .iter()
            .any(|p| p.pid == pid && p.parent_pid == root && p.started_at > 0);
        child.kill().unwrap();
        let mut zombie = false;
        for _ in 0..100 {
            if read_stat(pid).is_ok_and(|s| !live(&s)) {
                zombie = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let result = sample_tree(root);
        let exited = sample_process(pid).is_err();
        let _ = child.wait();
        let (_, rows) = result.unwrap();
        assert!(found && zombie && exited);
        assert!(!rows.iter().any(|p| p.pid == pid));
        let own = sample_process(root).unwrap();
        assert!(own.resident_memory_bytes > 0);
        assert_eq!(rows[0].pid, root);
        assert_eq!(own.started_at, rows[0].started_at);
    }
}
