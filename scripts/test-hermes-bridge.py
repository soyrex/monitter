#!/usr/bin/env python3
"""Offline protocol and lifecycle tests for Monitter's Hermes bridge."""

from __future__ import annotations

import importlib.util
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import textwrap
import time


ROOT = Path(__file__).resolve().parents[1]
BRIDGE_PATH = ROOT / "src-tauri" / "src" / "adapters" / "hermes_bridge.py"
BRIDGE = BRIDGE_PATH.read_text(encoding="utf-8")


def install_fake(root: Path, source: str) -> Path:
    (root / "tui_gateway").mkdir(parents=True)
    (root / "venv" / "bin").mkdir(parents=True)
    (root / "venv" / "bin" / "python").symlink_to(sys.executable)
    (root / "hermes").write_text("# fake entry marker\n", encoding="utf-8")
    (root / "tui_gateway" / "entry.py").write_text(
        textwrap.dedent(source), encoding="utf-8"
    )
    wrapper = root.parent / "hermes"
    wrapper.write_text(
        "#!/usr/bin/env bash\n"
        f'exec "{root}/venv/bin/python" "{root}/hermes" "$@"\n',
        encoding="utf-8",
    )
    wrapper.chmod(0o755)
    return wrapper


PROTOCOL_GATEWAY = r'''
import json
import sys

resumed = False

def send(frame):
    print(json.dumps(frame), flush=True)

def event(kind, payload=None):
    frame = {
        "jsonrpc": "2.0",
        "method": "event",
        "params": {"type": kind, "session_id": "live"},
    }
    if payload is not None:
        frame["params"]["payload"] = payload
    send(frame)

event("gateway.ready", {})
while line := sys.stdin.readline():
    request = json.loads(line)
    method = request["method"]
    if method == "session.create":
        send({
            "jsonrpc": "2.0", "id": request["id"],
            "result": {"session_id": "live", "stored_session_id": "new-key"},
        })
    elif method == "session.resume":
        resumed = True
        key = request["params"]["session_id"]
        send({
            "jsonrpc": "2.0", "id": request["id"],
            "result": {"session_id": "live", "session_key": key, "resumed": key},
        })
    elif method == "prompt.submit" and resumed:
        # Exercise the race where a very fast turn completes before the
        # prompt.submit response reaches the bridge.
        event("message.complete", {
            "status": "complete", "text": "Resumed response", "usage": {"input_tokens": 1},
        })
        send({"jsonrpc": "2.0", "id": request["id"], "result": {"task_id": "task"}})
    elif method == "prompt.submit":
        send({"jsonrpc": "2.0", "id": request["id"], "result": {"task_id": "task"}})
        event("tool.complete", {
            "tool_id": "tool-1", "name": "computer_use", "summary": "clicked",
            "args": {"action": "click"}, "result": "ok",
        })
        event("approval.request", {"request_id": "approval-1", "command": "danger"})
    elif method == "approval.respond":
        assert request["params"]["choice"] == "deny"
        event("clarify.request", {"request_id": "clarify-1", "question": "Choose?"})
        send({"jsonrpc": "2.0", "id": request["id"], "result": {"resolved": True}})
    elif method == "clarify.respond":
        assert request["params"]["answer"] == ""
        event("error", {"message": "tool was denied"})
        event("status.update", {"kind": "goal", "text": "Goal remains active"})
        event("message.complete", {
            "status": "complete", "text": "Finished", "usage": {"input_tokens": 3},
        })
        send({"jsonrpc": "2.0", "id": request["id"], "result": {"resolved": True}})
    elif method == "session.close":
        send({"jsonrpc": "2.0", "id": request["id"], "result": {"closed": True}})
        break
'''


CANCELLATION_GATEWAY = r'''
import json
from pathlib import Path
import sys
import time

marker = Path(__file__).parent / "cancelled"

def send(frame):
    print(json.dumps(frame), flush=True)

send({"jsonrpc": "2.0", "method": "event", "params": {"type": "gateway.ready", "payload": {}}})
try:
    while line := sys.stdin.readline():
        request = json.loads(line)
        if request["method"] == "session.create":
            send({
                "jsonrpc": "2.0", "id": request["id"],
                "result": {"session_id": "live", "stored_session_id": "key"},
            })
        elif request["method"] == "prompt.submit":
            send({"jsonrpc": "2.0", "id": request["id"], "result": {"task_id": "task"}})
            while True:
                time.sleep(1)
except KeyboardInterrupt:
    marker.write_text("yes", encoding="utf-8")
'''


NO_CLOSE_RESPONSE_GATEWAY = r'''
import json
import sys
import time

def send(frame):
    print(json.dumps(frame), flush=True)

def event(kind, payload=None):
    frame = {
        "jsonrpc": "2.0", "method": "event",
        "params": {"type": kind, "session_id": "live"},
    }
    if payload is not None:
        frame["params"]["payload"] = payload
    send(frame)

event("gateway.ready", {})
while line := sys.stdin.readline():
    request = json.loads(line)
    if request["method"] == "session.create":
        send({
            "jsonrpc": "2.0", "id": request["id"],
            "result": {"session_id": "live", "stored_session_id": "key"},
        })
    elif request["method"] == "prompt.submit":
        send({"jsonrpc": "2.0", "id": request["id"], "result": {"task_id": "task"}})
        event("message.complete", {"status": "complete", "text": "Finished"})
    elif request["method"] == "session.close":
        # Simulate a wedged gateway that consumes close but never responds or exits.
        while True:
            time.sleep(1)
'''


def bridge_module():
    spec = importlib.util.spec_from_file_location("monitter_hermes_bridge", BRIDGE_PATH)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def records(stdout: str) -> list[dict]:
    return [json.loads(line) for line in stdout.splitlines() if line.strip()]


def invoke(wrapper: Path, cwd: Path, *extra: str) -> list[dict]:
    process = subprocess.run(
        [
            sys.executable,
            "-u",
            "-c",
            BRIDGE,
            "--hermes",
            str(wrapper),
            "--cwd",
            str(cwd),
            *extra,
        ],
        input="Prompt\n",
        text=True,
        capture_output=True,
        timeout=20,
        check=False,
    )
    parsed = records(process.stdout)
    assert process.returncode == 0, (process.stderr, parsed)
    return parsed


def test_locator_keeps_venv_interpreter() -> None:
    module = bridge_module()
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary) / "hermes-agent"
        wrapper = install_fake(root, "")
        found_root, found_python = module.locate_gateway(str(wrapper))
        expected_python = (root / "venv" / "bin" / "python").absolute()
        assert found_root == root.resolve()
        assert found_python == expected_python
        assert found_python.is_symlink()


def test_protocol_create_resume_and_race() -> None:
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary) / "hermes-agent"
        wrapper = install_fake(root, PROTOCOL_GATEWAY)

        created = invoke(wrapper, Path(temporary))
        assert created[0] == {"type": "session", "session_id": "new-key"}
        assert any(row.get("request") == "approval" for row in created)
        assert any(row.get("request") == "clarification" for row in created)
        assert any(
            row.get("type") == "error" and row.get("fatal") is False
            for row in created
        )
        assert any(
            row.get("type") == "tool"
            and row.get("name") == "computer_use"
            and row.get("phase") == "completed"
            for row in created
        )
        assert any(
            row.get("type") == "status"
            and row.get("status_kind") == "goal"
            and row.get("detail") == "Goal remains active"
            for row in created
        )
        assert [row["text"] for row in created if row.get("type") == "output"] == [
            "Finished"
        ]

        resumed = invoke(wrapper, Path(temporary), "--session", "old-key")
        assert resumed[0] == {"type": "session", "session_id": "old-key"}
        assert [row["text"] for row in resumed if row.get("type") == "output"] == [
            "Resumed response"
        ]
        assert not any(
            row.get("type") == "session" and row.get("session_id") != "old-key"
            for row in resumed
        )


def test_process_group_cancellation_reaches_gateway() -> None:
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary) / "hermes-agent"
        wrapper = install_fake(root, CANCELLATION_GATEWAY)
        process = subprocess.Popen(
            [
                sys.executable,
                "-u",
                "-c",
                BRIDGE,
                "--hermes",
                str(wrapper),
                "--cwd",
                temporary,
            ],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            start_new_session=True,
        )
        assert process.stdin and process.stdout
        process.stdin.write("Prompt\n")
        process.stdin.close()
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            line = process.stdout.readline()
            if line and json.loads(line).get("type") == "session":
                break
        else:
            raise AssertionError("bridge did not create the fake Hermes session")

        os.killpg(process.pid, signal.SIGINT)
        assert process.wait(timeout=8) == 130
        marker = root / "tui_gateway" / "cancelled"
        deadline = time.monotonic() + 2
        while time.monotonic() < deadline and not marker.exists():
            time.sleep(0.05)
        assert marker.exists(), "gateway child did not receive process-group SIGINT"


def test_unresponsive_close_is_bounded() -> None:
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary) / "hermes-agent"
        wrapper = install_fake(root, NO_CLOSE_RESPONSE_GATEWAY)
        started = time.monotonic()
        parsed = invoke(wrapper, Path(temporary))
        elapsed = time.monotonic() - started
        assert [row["text"] for row in parsed if row.get("type") == "output"] == [
            "Finished"
        ]
        assert elapsed < 5, f"unresponsive close took {elapsed:.2f}s"


def main() -> None:
    test_locator_keeps_venv_interpreter()
    test_protocol_create_resume_and_race()
    test_process_group_cancellation_reaches_gateway()
    test_unresponsive_close_is_bounded()
    print("Hermes bridge offline tests passed")


if __name__ == "__main__":
    main()
