"""No SSH server or model calls: exercise the exact embedded remote supervisor."""
import json
import os
from pathlib import Path
import select
import signal
import subprocess
import sys
import tempfile
import threading
import time
import unittest

SUPERVISOR = Path(__file__).resolve().parent.parent / "src-tauri/src/acp_remote.py"


def frame(process, timeout=5):
    if not select.select([process.stdout], [], [], timeout)[0]:
        raise AssertionError("Remote supervisor frame timed out")
    return json.loads(process.stdout.readline())


def gone(pid):
    try:
        os.kill(pid, 0)
        return False
    except ProcessLookupError:
        return True


class RemoteAcp(unittest.TestCase):
    def setUp(self):
        self.folder = tempfile.TemporaryDirectory(prefix="monitter-acp-supervisor-")
        self.processes = []

    def tearDown(self):
        for process in self.processes:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=2)
            for stream in [process.stdin, process.stdout, process.stderr]:
                if stream and not stream.closed:
                    stream.close()
        self.folder.cleanup()

    def start(self, code, *args):
        process = subprocess.Popen(
            [sys.executable, "-u", str(SUPERVISOR), self.folder.name,
             sys.executable, "-u", "-c", code, *args],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            bufsize=0,
        )
        self.processes.append(process)
        ready = frame(process)
        self.assertEqual(ready["method"], "_monitter/acp_transport_ready")
        self.assertEqual(ready["params"]["cwd"], self.folder.name)
        return process

    def test_argv_and_protocol_bytes_are_forwarded_without_shell_evaluation(self):
        code = """import json,os,sys
request=json.loads(sys.stdin.buffer.readline())
print(json.dumps({'jsonrpc':'2.0','id':request['id'],'result':{'protocolVersion':1,'cwd':os.getcwd(),'args':sys.argv[1:],'text':request['params']['text']}}),flush=True)
sys.stdin.buffer.read()
"""
        args = ["literal $(not-a-command)", "quoted ' argument", "space and\nnewline"]
        process = self.start(code, *args)
        process.stdin.write(json.dumps({"jsonrpc": "2.0", "id": "opaque", "method": "initialize", "params": {"text": "one\u2028two\u2029three"}}).encode()+b"\n")
        result = frame(process)["result"]
        self.assertEqual(result["args"], args)
        self.assertEqual(result["cwd"], os.path.realpath(self.folder.name))
        self.assertEqual(result["text"], "one\u2028two\u2029three")
        process.stdin.close()
        process.wait(timeout=3)

    def test_disconnect_cleans_owned_resistant_process_group(self):
        code = """import json,os,signal,subprocess,sys,time
signal.signal(signal.SIGINT,signal.SIG_IGN)
signal.signal(signal.SIGTERM,signal.SIG_IGN)
child=subprocess.Popen([sys.executable,'-c','import signal,time;signal.signal(signal.SIGINT,signal.SIG_IGN);signal.signal(signal.SIGTERM,signal.SIG_IGN);time.sleep(60)'])
print(json.dumps({'pid':os.getpid(),'child':child.pid}),flush=True)
time.sleep(60)
"""
        process = self.start(code)
        pids = frame(process)
        process.stdin.close()
        process.wait(timeout=4)
        deadline = time.monotonic()+4
        while time.monotonic()<deadline and not all(gone(pid) for pid in pids.values()):
            time.sleep(0.025)
        self.assertTrue(all(gone(pid) for pid in pids.values()), "owned agent/tool remained after stdin disconnect")

    def test_signal_cleanup_does_not_wait_for_blocked_child_stdin(self):
        process = self.start("import json,os,time;print(json.dumps({'pid':os.getpid()}),flush=True);time.sleep(60)")
        pid = frame(process)["pid"]
        def saturate():
            try:
                pending = memoryview(b"x"*(2*1024*1024))
                while pending:
                    count = process.stdin.write(pending)
                    pending = pending[count:]
            except (OSError, ValueError):
                pass
        writer = threading.Thread(target=saturate, daemon=True)
        writer.start()
        process.terminate()
        process.wait(timeout=4)
        writer.join(timeout=2)
        self.assertFalse(writer.is_alive())
        self.assertTrue(gone(pid))


if __name__ == "__main__":
    unittest.main()
