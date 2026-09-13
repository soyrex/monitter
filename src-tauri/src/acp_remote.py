"""Ephemeral ACP stdio supervisor, embedded in Monitter's SSH command.

No files, listeners, authentication changes or persistent remote service.
Only the child process group created here is eligible for cleanup.
"""
import json
import os
import queue
import shutil
import signal
import subprocess
import sys
import threading
import time

READY = "_monitter/acp_transport_ready"
process = None
stopping = False
stop_requested = threading.Event()
chunks = queue.Queue(maxsize=64)  # <=512 KiB, independent of child's stdin pace


def stop_owned():
    global stopping
    if stopping or process is None:
        return
    stopping = True
    for sig in (signal.SIGHUP, signal.SIGINT, signal.SIGTERM):
        signal.signal(sig, signal.SIG_IGN)
    for sig, seconds in ((signal.SIGINT, 0.5), (signal.SIGTERM, 0.5), (signal.SIGKILL, 0)):
        try:
            os.killpg(process.pid, sig)
        except ProcessLookupError:
            return
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            try:
                os.killpg(process.pid, 0)
            except ProcessLookupError:
                return
            time.sleep(0.025)


def on_signal(signum, _frame):
    raise SystemExit(128 + signum)


def read_input():
    try:
        while True:
            chunk = sys.stdin.buffer.read1(8192)
            if not chunk:
                return
            # A blocked child must not stop disconnect cleanup indefinitely.
            chunks.put(chunk, timeout=5)
    except (OSError, queue.Full):
        pass
    finally:
        stop_requested.set()


def write_input():
    try:
        while not stop_requested.is_set():
            try:
                chunk = chunks.get(timeout=0.1)
            except queue.Empty:
                continue
            process.stdin.write(chunk)
            process.stdin.flush()
    except (BrokenPipeError, OSError):
        stop_requested.set()


def executable(name):
    expanded = os.path.expanduser(name)
    if os.path.isabs(expanded):
        candidates = [expanded]
    elif "/" in expanded or "\\" in expanded:
        raise ValueError("ACP executable must be absolute or a name from PATH")
    else:
        dirs = [os.path.expanduser("~/" + suffix) for suffix in (
            ".local/bin", ".npm-global/bin", ".opencode/bin", ".bun/bin", ".cargo/bin"
        )]
        dirs += [p for p in os.environ.get("PATH", "").split(os.pathsep)[:128] if os.path.isabs(p)]
        dirs += ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"]
        candidates = [os.path.join(p, expanded) for p in dirs]
    for path in candidates:
        if os.path.isfile(path) and os.access(path, os.X_OK):
            return path
    raise ValueError("ACP executable was not found or is not executable on the SSH host")


try:
    if len(sys.argv) < 3:
        raise ValueError("ACP transport needs a folder and executable")
    cwd = os.path.abspath(os.path.expanduser(sys.argv[1]))
    os.chdir(cwd)  # Fail before readiness, never silently launch elsewhere.
    program = executable(sys.argv[2])
    for sig in (signal.SIGHUP, signal.SIGINT, signal.SIGTERM):
        signal.signal(sig, on_signal)
    # A private transport notification, consumed only during Monitter bootstrap.
    # Flush before the child can write any ACP output.
    print(json.dumps({"jsonrpc": "2.0", "method": READY, "params": {"cwd": cwd}}), flush=True)
    process = subprocess.Popen([program, *sys.argv[3:]], cwd=cwd,
                               stdin=subprocess.PIPE, stdout=sys.stdout.buffer,
                               stderr=sys.stderr.buffer, start_new_session=True)
    threading.Thread(target=read_input, daemon=True).start()
    threading.Thread(target=write_input, daemon=True).start()
    while process.poll() is None and not stop_requested.wait(0.05):
        pass
    code = process.poll()
except (ValueError, OSError) as error:
    # Paths and argument values are not needed to diagnose a failed bootstrap.
    print("ERROR: ACP SSH bootstrap failed (" + type(error).__name__ + "). Check Python, executable and folder.", file=sys.stderr)
    code = 1
finally:
    stop_owned()
    if process is not None:
        try:
            process.wait(timeout=1)
        except subprocess.TimeoutExpired:
            pass
sys.exit(code if isinstance(code, int) and code >= 0 else 1)
