#![cfg(target_os = "macos")]

use crate::process_metrics;
use std::{
    io::{BufRead, BufReader},
    os::fd::AsRawFd,
    os::unix::process::CommandExt,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

struct ScopedProcessTree {
    root: Child,
}

impl Drop for ScopedProcessTree {
    fn drop(&mut self) {
        // Ask the root to kill and reap both forked children first. The bounded
        // process-group fallback is scoped to this fixture only.
        unsafe {
            libc::kill(self.root.id() as libc::pid_t, libc::SIGTERM);
        }
        if !wait_child_until(&mut self.root, Duration::from_secs(2)) {
            unsafe {
                libc::kill(-(self.root.id() as libc::pid_t), libc::SIGKILL);
            }
            let _ = wait_child_until(&mut self.root, Duration::from_secs(2));
        }
    }
}

fn wait_child_until(child: &mut Child, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if child.try_wait().ok().flatten().is_some() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn start_zombie_and_live_child() -> (ScopedProcessTree, libc::pid_t, libc::pid_t) {
    let script = r#"
import os, signal, subprocess, sys, time

def cleanup(_signum, _frame):
    children = [globals().get("live"), globals().get("zombie")]
    for child in children:
        if child:
            try:
                os.kill(child, signal.SIGKILL)
            except ProcessLookupError:
                pass
    for child in children:
        if child:
            try:
                os.waitpid(child, 0)
            except ChildProcessError:
                pass
    sys.exit(0)

signal.signal(signal.SIGTERM, cleanup)

zombie = os.fork()
if zombie == 0:
    os._exit(0)

live = os.fork()
if live == 0:
    while True:
        time.sleep(1)

# Poll ps for the zombie state without waitpid, leaving a real unreaped child
# for the BSD process inspection exercised by the Rust test.
for _ in range(200):
    result = subprocess.run(
        ["ps", "-o", "stat=", "-p", str(zombie)],
        capture_output=True, text=True, timeout=0.5
    )
    if result.stdout.strip().startswith("Z"):
        break
    time.sleep(0.01)
else:
    raise RuntimeError("zombie fixture child did not exit")

print(f"{os.getpid()} {zombie} {live}", flush=True)
time.sleep(120)
"#;
    let mut command = Command::new("python3");
    command
        .args(["-u", "-c", script])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0);
    let mut root = command.spawn().expect("python3 is required for this macOS fixture");
    let stdout = root.stdout.take().expect("fixture stdout is piped");
    let fixture = ScopedProcessTree { root };

    let mut ready = libc::pollfd {
        fd: stdout.as_raw_fd(),
        events: libc::POLLIN | libc::POLLHUP,
        revents: 0,
    };
    let ready_count = unsafe { libc::poll(&mut ready, 1, 5_000) };
    assert!(ready_count > 0, "fixture did not report PIDs within five seconds");
    let mut line = String::new();
    BufReader::new(stdout)
        .read_line(&mut line)
        .expect("read fixture process identities");
    let mut pids = line
        .split_whitespace()
        .map(|part| part.parse::<libc::pid_t>().expect("fixture PID is numeric"));
    let reported_root = pids.next().expect("fixture reports root PID");
    let zombie = pids.next().expect("fixture reports zombie PID");
    let live = pids.next().expect("fixture reports live PID");
    assert_eq!(reported_root, fixture.root.id() as libc::pid_t);
    (fixture, zombie, live)
}

#[test]
fn retirement_tree_ignores_zombies_but_keeps_live_unknown_children() {
    let (fixture, zombie, live) = start_zombie_and_live_child();
    // The fixture is a child of this test process. Query its own subtree, not
    // the test runner's full process tree.
    let root = fixture.root.id() as libc::pid_t;
    let identities = process_metrics::retirement_process_tree(root)
        .expect("zombie descendants should not make retirement inspection fail");

    assert!(identities.iter().any(|(pid, _)| *pid == root));
    assert!(identities.iter().any(|(pid, _)| *pid == live));
    assert!(
        !identities.iter().any(|(pid, _)| *pid == zombie),
        "unreaped exited descendants must not count as live owned work"
    );

    let (_, live_started_at) = identities
        .iter()
        .find(|(pid, _)| *pid == live)
        .copied()
        .expect("the live unknown sibling remains in the inspected tree");
    assert!(process_metrics::retirement_process_identity_alive(live, live_started_at)
        .expect("live child identity is inspectable"));
    assert!(!process_metrics::retirement_process_identity_alive(live, live_started_at + 1)
        .expect("identity mismatch must fail closed without signalling a reused PID"));
}
