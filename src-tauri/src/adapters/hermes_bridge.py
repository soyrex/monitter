"""Adapt Hermes' full-duplex TUI gateway to Monitter's one-shot JSONL runner."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import threading
from typing import Any


class BridgeError(RuntimeError):
    pass


_OUTPUT_LOCK = threading.Lock()


def emit(kind: str, **fields: Any) -> None:
    with _OUTPUT_LOCK:
        print(json.dumps({"type": kind, **fields}, ensure_ascii=False), flush=True)


def _expand_executable(raw: str) -> Path:
    expanded = os.path.expanduser(raw.strip())
    resolved = shutil.which(expanded) if not os.path.isabs(expanded) else expanded
    if not resolved and expanded == "hermes":
        home = Path.home()
        resolved = next(
            (
                str(candidate)
                for candidate in [
                    home / ".local" / "bin" / "hermes",
                    home / ".npm-global" / "bin" / "hermes",
                    Path("/opt/homebrew/bin/hermes"),
                    Path("/usr/local/bin/hermes"),
                ]
                if candidate.is_file()
            ),
            None,
        )
    if not resolved:
        raise BridgeError(
            f"Hermes executable was not found: {raw}. Set the Hermes path on this host."
        )
    path = Path(resolved).resolve()
    if not path.is_file() or not os.access(path, os.X_OK):
        raise BridgeError(f"Hermes executable is not runnable: {path}")
    return path


def _read_head(path: Path) -> list[str]:
    try:
        with path.open("r", encoding="utf-8", errors="replace") as source:
            return [source.readline().rstrip("\n") for _ in range(12)]
    except OSError as exc:
        raise BridgeError(f"Could not inspect Hermes executable {path}: {exc}") from exc


def _is_python(path: Path) -> bool:
    return path.is_file() and os.access(path, os.X_OK) and "python" in path.name.lower()


def locate_gateway(raw: str) -> tuple[Path, Path]:
    """Resolve a Hermes install without evaluating its wrapper as shell code."""
    executable = _expand_executable(raw)
    lines = _read_head(executable)
    roots: list[Path] = []
    interpreters: list[Path] = []

    if lines and lines[0].startswith("#!"):
        try:
            shebang = shlex.split(lines[0][2:].strip())
        except ValueError:
            shebang = []
        if shebang:
            candidate = Path(shebang[0]).expanduser()
            if _is_python(candidate):
                # Keep the venv-facing path. Resolving this symlink to the base
                # interpreter disables Python's pyvenv.cfg discovery.
                interpreters.append(candidate.absolute())
                if candidate.parent.name in {"bin", "Scripts"}:
                    roots.append(candidate.parent.parent)

    # The official installer currently writes a small bash wrapper whose exec
    # line names both the install venv's Python and the source entry script.
    for line in lines:
        if not line.lstrip().startswith("exec "):
            continue
        try:
            words = shlex.split(line.strip())
        except ValueError:
            continue
        if len(words) < 3 or words[0] != "exec":
            continue
        python = Path(os.path.expanduser(words[1]))
        entry = Path(os.path.expanduser(words[2]))
        if _is_python(python) and entry.is_file():
            interpreters.append(python.absolute())
            roots.append(entry.resolve().parent)

    # Console scripts installed in <root>/venv/bin or <root>/.venv/bin.
    for ancestor in [executable.parent, *executable.parents]:
        if ancestor.name in {"bin", "Scripts"} and ancestor.parent.name in {
            "venv",
            ".venv",
        }:
            roots.append(ancestor.parent.parent)
            break
    roots.extend([executable.parent, executable.parent.parent])

    unique_roots: list[Path] = []
    for root in roots:
        root = root.resolve()
        if root not in unique_roots:
            unique_roots.append(root)
    unique_pythons: list[Path] = []
    for python in interpreters:
        if python not in unique_pythons:
            unique_pythons.append(python)

    for root in unique_roots:
        if not (root / "tui_gateway" / "entry.py").is_file():
            continue
        candidates = [
            *unique_pythons,
            root / "venv" / "bin" / "python",
            root / ".venv" / "bin" / "python",
            root / "venv" / "Scripts" / "python.exe",
            root / ".venv" / "Scripts" / "python.exe",
        ]
        for python in candidates:
            if _is_python(python):
                return root, python.absolute()
    raise BridgeError(
        f"Hermes TUI gateway was not found beside {executable}. "
        "Set Hermes path to the CLI installed by the official installer."
    )


class Gateway:
    def __init__(self, root: Path, python: Path, cwd: Path):
        environment = os.environ.copy()
        environment["HERMES_PYTHON_SRC_ROOT"] = str(root)
        old_pythonpath = environment.get("PYTHONPATH", "")
        environment["PYTHONPATH"] = str(root) + (
            os.pathsep + old_pythonpath if old_pythonpath else ""
        )
        self.process = subprocess.Popen(
            [str(python), "-m", "tui_gateway.entry"],
            cwd=str(root),
            env=environment,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            encoding="utf-8",
            errors="replace",
            bufsize=1,
        )
        if not self.process.stdin or not self.process.stdout or not self.process.stderr:
            raise BridgeError("Could not open Hermes gateway pipes")
        self.stdin = self.process.stdin
        self.stdout = self.process.stdout
        self._ids = 0
        self._write_lock = threading.Lock()
        self.live_session_id = ""
        self.durable_session_id = ""
        self.cwd = cwd
        self.terminal_status: str | None = None
        self._stderr_thread = threading.Thread(target=self._drain_stderr, daemon=True)
        self._stderr_thread.start()

    def _drain_stderr(self) -> None:
        assert self.process.stderr is not None
        for line in self.process.stderr:
            text = line.rstrip("\r\n")
            if text:
                emit("log", session_id=self.durable_session_id, text=text)

    def send(self, method: str, params: dict[str, Any]) -> int:
        self._ids += 1
        request_id = self._ids
        frame = {
            "jsonrpc": "2.0",
            "id": request_id,
            "method": method,
            "params": params,
        }
        with self._write_lock:
            self.stdin.write(json.dumps(frame, ensure_ascii=False) + "\n")
            self.stdin.flush()
        return request_id

    def receive(self) -> dict[str, Any]:
        raw = self.stdout.readline()
        if not raw:
            code = self.process.poll()
            raise BridgeError(f"Hermes gateway closed unexpectedly (exit {code})")
        try:
            frame = json.loads(raw)
        except json.JSONDecodeError as exc:
            raise BridgeError(f"Hermes gateway emitted invalid JSON: {exc}") from exc
        if not isinstance(frame, dict):
            raise BridgeError("Hermes gateway emitted a non-object JSON frame")
        return frame

    def call(self, method: str, params: dict[str, Any]) -> dict[str, Any]:
        request_id = self.send(method, params)
        while True:
            frame = self.receive()
            if frame.get("method") == "event":
                self.handle_event(frame)
                continue
            if frame.get("id") != request_id:
                # Responses to fire-and-forget denial RPCs are intentionally
                # ignored; their request IDs remain unique for this process.
                continue
            if "error" in frame:
                error = frame.get("error") or {}
                raise BridgeError(
                    f"Hermes {method} failed: {error.get('message') or error}"
                )
            result = frame.get("result")
            return result if isinstance(result, dict) else {}

    def _detail(self, event_type: str, payload: Any) -> dict[str, Any]:
        return {"event": event_type, "payload": payload}

    def handle_event(self, frame: dict[str, Any]) -> str:
        params = frame.get("params") or {}
        event_type = str(params.get("type") or "")
        live_sid = str(params.get("session_id") or self.live_session_id)
        payload = params.get("payload")
        payload_obj = payload if isinstance(payload, dict) else {}
        durable_sid = self.durable_session_id

        if event_type == "gateway.ready":
            return event_type
        if event_type == "session.info":
            # Monitter retains the root durable ID. Hermes resume follows its
            # compression-continuation chain to the current tip internally.
            return event_type
        if event_type == "approval.request":
            deny = {"session_id": live_sid, "choice": "deny"}
            if payload_obj.get("request_id"):
                deny["request_id"] = payload_obj["request_id"]
            self.send("approval.respond", deny)
            emit(
                "permission",
                session_id=durable_sid,
                request="approval",
                decision="denied",
                detail=self._detail(event_type, payload),
            )
            return event_type
        if event_type in {"clarify.request", "sudo.request", "secret.request"}:
            response_method = event_type.replace(".request", ".respond")
            response: dict[str, Any] = {
                "session_id": live_sid,
                "request_id": payload_obj.get("request_id"),
            }
            if event_type == "clarify.request":
                response["answer"] = ""
                request_name = "clarification"
            elif event_type == "sudo.request":
                response["password"] = ""
                request_name = "sudo"
            else:
                response["value"] = ""
                request_name = "secret"
            self.send(response_method, response)
            emit(
                "permission",
                session_id=durable_sid,
                request=request_name,
                decision="denied",
                detail=self._detail(event_type, payload),
            )
            return event_type
        if event_type in {"tool.start", "tool.complete"}:
            emit(
                "tool",
                session_id=durable_sid,
                name=str(payload_obj.get("name") or "Tool activity"),
                phase="started" if event_type.endswith("start") else "completed",
                detail=self._detail(event_type, payload),
            )
        elif event_type == "reasoning.available":
            text = str(payload_obj.get("text") or "")
            if text.strip():
                emit("reasoning", session_id=durable_sid, text=text)
        elif event_type == "todo.updated":
            emit(
                "status",
                session_id=durable_sid,
                status_kind="todo",
                detail=self._detail(event_type, payload),
            )
        elif event_type == "status.update":
            status_kind = str(payload_obj.get("kind") or "status")
            emit(
                "status",
                session_id=durable_sid,
                status_kind=status_kind,
                # Goal progress is free-form in Hermes v0.21.0. Preserve the
                # actual text rather than inventing objective/budget metrics.
                detail=(
                    str(payload_obj.get("text") or "")
                    if status_kind == "goal"
                    else self._detail(event_type, payload)
                ),
            )
        elif event_type == "error":
            emit(
                "error",
                session_id=durable_sid,
                message=str(payload_obj.get("message") or "Hermes gateway error"),
                fatal=False,
            )
        elif event_type == "message.complete":
            self.terminal_status = str(payload_obj.get("status") or "complete")
            reasoning = payload_obj.get("reasoning")
            if isinstance(reasoning, str) and reasoning.strip():
                emit("reasoning", session_id=durable_sid, text=reasoning)
            usage = payload_obj.get("usage")
            if usage is not None:
                emit(
                    "usage",
                    session_id=durable_sid,
                    detail={"event": event_type, "usage": usage},
                )
            text = str(payload_obj.get("text") or "")
            if text.strip():
                emit("output", session_id=durable_sid, text=text)
            if payload_obj.get("status") == "error":
                emit(
                    "error",
                    session_id=durable_sid,
                    message=str(payload_obj.get("error") or text or "Hermes turn failed"),
                    fatal=True,
                )
        return event_type

    def wait_ready(self) -> None:
        while True:
            frame = self.receive()
            if frame.get("method") == "event" and self.handle_event(frame) == "gateway.ready":
                return

    def close(self) -> None:
        if self.process.poll() is not None:
            return
        if self.live_session_id:
            try:
                # Shutdown is best-effort and bounded. Waiting for a JSON-RPC
                # response here could hang a completed Monitter run forever if
                # the gateway stopped dispatching while its process stayed up.
                self.send("session.close", {"session_id": self.live_session_id})
            except Exception:
                pass
        try:
            self.stdin.close()
        except Exception:
            pass
        try:
            self.process.wait(timeout=2)
        except subprocess.TimeoutExpired:
            self.process.terminate()
            try:
                self.process.wait(timeout=1)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait()
        self._stderr_thread.join(timeout=1)


def run(options: argparse.Namespace, prompt: str) -> int:
    root, python = locate_gateway(options.hermes)
    cwd = Path(os.path.expanduser(options.cwd)).resolve()
    if not cwd.is_dir():
        raise BridgeError(f"Task folder does not exist: {cwd}")
    gateway = Gateway(root, python, cwd)
    try:
        gateway.wait_ready()
        if options.session:
            result = gateway.call(
                "session.resume",
                {
                    "session_id": options.session,
                    "cols": 120,
                    "omit_messages": True,
                },
            )
            gateway.live_session_id = str(result.get("session_id") or "")
            # Keep the original routing ID stable. Hermes resolves an ended
            # compression parent to its live continuation during resume.
            gateway.durable_session_id = options.session
        else:
            create: dict[str, Any] = {
                "cwd": str(cwd),
                "source": "tui",
                "cols": 120,
            }
            if options.model:
                create["model"] = options.model
            result = gateway.call("session.create", create)
            gateway.live_session_id = str(result.get("session_id") or "")
            gateway.durable_session_id = str(result.get("stored_session_id") or "")
        if not gateway.live_session_id or not gateway.durable_session_id:
            raise BridgeError("Hermes did not return live and durable session IDs")
        emit("session", session_id=gateway.durable_session_id)

        gateway.call(
            "prompt.submit",
            {"session_id": gateway.live_session_id, "text": prompt},
        )
        if gateway.terminal_status is not None:
            return 1 if gateway.terminal_status == "error" else 0
        while True:
            frame = gateway.receive()
            if frame.get("method") != "event":
                continue
            if gateway.handle_event(frame) == "message.complete":
                payload = (frame.get("params") or {}).get("payload") or {}
                return 1 if payload.get("status") == "error" else 0
    finally:
        gateway.close()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--hermes", default="hermes")
    parser.add_argument("--cwd", required=True)
    parser.add_argument("--session", default="")
    parser.add_argument("--model", default="")
    options = parser.parse_args()
    prompt = sys.stdin.read()
    if prompt.endswith("\n"):
        prompt = prompt[:-1]
    if not prompt.strip():
        raise BridgeError("Hermes prompt is empty")
    return run(options, prompt)


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except KeyboardInterrupt:
        raise SystemExit(130)
    except BridgeError as exc:
        emit("error", message=str(exc), fatal=True)
        raise SystemExit(1)
    except (BrokenPipeError, OSError, subprocess.SubprocessError) as exc:
        emit("error", message=f"Hermes bridge failed: {exc}", fatal=True)
        raise SystemExit(1)
