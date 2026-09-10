#!/usr/bin/env python3
"""Opt-in real Codex collaboration proof. Uses isolated state and existing host authentication."""
import argparse
import json
import os
from pathlib import Path
import signal
import subprocess
import time
import uuid

parser = argparse.ArgumentParser()
parser.add_argument("--peer", choices=["local", "mira"], required=True)
parser.add_argument("--coordinator", choices=["local", "mira"], default="local")
parser.add_argument("--model", default="gpt-5.6-luna", help="Coordinator model supported by its installed Codex version")
parser.add_argument("--peer-model", help="Optional peer model when hosts support different versions")
args = parser.parse_args()
root = Path(__file__).resolve().parents[1]
binary = root / "src-tauri/target/debug/monitter-smoke"
if not binary.exists():
    raise SystemExit("Build the native monitter-smoke binary first.")
run = root / "artifacts" / f"collaboration-live-{args.coordinator}-to-{args.peer}-{time.time_ns()}"
run.mkdir(parents=True, mode=0o700)
state_dir = run / "state"
workspace = run / "workspace"
state_dir.mkdir(mode=0o700)
workspace.mkdir(mode=0o700)

# Only read connection definitions. No existing agents, messages, tasks or credentials are copied.
settings_file = Path.home() / "Library/Application Support/com.monitter.desktop/state.json"
configured = json.loads(settings_file.read_text()) if settings_file.exists() else {"hosts": []}
local = next((dict(h) for h in configured["hosts"] if h["kind"] == "local"), None)
if local is None:
    local = dict(id="proof-local", name="This Mac", kind="local", address="", user="", port=0,
                 identityFile="", defaultCwd=str(workspace), codexPath="", claudePath="", opencodePath="", hermesPath="")
hosts = [local]
mira = None
if "mira" in (args.peer, args.coordinator):
    mira = next((dict(h) for h in configured["hosts"] if h["kind"] == "ssh" and "mira" in (h["name"] + " " + h["address"]).lower()), None)
    if mira is None:
        raise SystemExit("The authorized Mira host is not configured in Monitter.")
    hosts.append(mira)
peer_host = local if args.peer == "local" else mira
coordinator_host = local if args.coordinator == "local" else mira
coordinator_cwd = str(workspace) if args.coordinator == "local" else coordinator_host["defaultCwd"]

def agent(name, host, cwd):
    return dict(id=str(uuid.uuid4()), name=name, description="Monitter protocol proof", instructions="Complete only the small supplied protocol proof; no file, shell, network, or external app tools are needed.",
                provider="codex", model=args.model, hostId=host["id"], cwd=cwd, color="#3f9d6a", sandbox="read-only", avatar=None,
                expertise=["protocol proof"], responsibilities=["Verify collaboration messages"], skills=["protocol acknowledgement"], collaborationEnabled=True)

coordinator = agent("Proof coordinator", coordinator_host, coordinator_cwd)
reviewer = agent("Proof reviewer", peer_host, str(workspace) if args.peer == "local" else peer_host["defaultCwd"])
reviewer["model"] = args.peer_model or args.model
state = dict(hosts=hosts, agents=[coordinator, reviewer], tasks=[], messages=[], events=[], channels=[], projects=[], collaborations=[],
             settings=dict(accent="#3f9d6a", theme="system", interfaceScale=125, showToolActivity=True, showReasoningSummaries=True, sendWithEnter=False, sidebarView="standard"))
state_path = state_dir / "state.json"
state_path.write_text(json.dumps(state))
os.chmod(state_path, 0o600)
marker = "MONITTER_PROOF_" + uuid.uuid4().hex[:16]
request_id = "proof-" + uuid.uuid4().hex[:12]
inbox_request_id = "inbox-" + uuid.uuid4().hex[:12]
inbox_marker = f"INBOX_ACK {marker}"
prompt = f"""Perform a small Monitter integration proof using only Monitter's collaboration tools. Do not emit interim assistant messages; only the exact final answer.
Call list_agents with query 'protocol proof' and find the agent named 'Proof reviewer'.
Call delegate_task for that directory ID with title 'Protocol acknowledgement', request_id '{request_id}', and this exact brief:
'Call list_agents with query protocol proof and find Proof coordinator. Its current_task_id is the active recipient task. Call send_message once to that agent using task_id=current_task_id, request_id={inbox_request_id}, and exact message {inbox_marker}. Then reply exactly PEER_OK {marker}. Do not delegate, send any other message, or call tools again.'
Use wait_for_task for the delegation until its status is terminal. It may return early with incoming peer messages: remember their text and continue waiting if status is queued or running. Once terminal, use get_task_result for the same delegation. Do not create a second delegation or send a peer message yourself.
Only if the incoming message contains exactly {inbox_marker} and the completed result contains PEER_OK {marker}, reply exactly 'HANDOFF_OK {marker}'. Otherwise report the actual failure. Do not pretend a tool succeeded.
"""
followup = f"Verify the existing delegation from the previous turn with get_task_result and inspect list_messages. Do not create a delegation or send a message. If the completed result contains PEER_OK {marker} and the persisted inbox contains {inbox_marker}, reply exactly 'RESUME_OK {marker}'. Otherwise report the actual failure."
command = [str(binary), "--state-dir", str(state_dir), "--cwd", coordinator_cwd, "--provider", "codex", "--model", args.model,
           "--prompt", prompt, "--second-prompt", followup]
print(json.dumps({"proof": args.peer, "coordinator": args.coordinator, "artifact": str(run), "status": "running"}), flush=True)

def stop_owned_smoke(process):
    """Reap only the smoke binary that this script started; never use a host-wide kill."""
    if process.poll() is not None:
        return process.returncode
    process.send_signal(signal.SIGTERM)
    try:
        return process.wait(timeout=15)
    except subprocess.TimeoutExpired:
        process.kill()
        try:
            return process.wait(timeout=5)
        except subprocess.TimeoutExpired as error:
            raise RuntimeError("Owned Monitter smoke process did not exit after SIGTERM and SIGKILL.") from error

with (run / "process.stdout").open("w") as stdout, (run / "process.stderr").open("w") as stderr:
    process = subprocess.Popen(command, cwd=root, stdout=stdout, stderr=stderr)
    deadline = time.monotonic() + 420
    try:
        while process.poll() is None:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                stop_owned_smoke(process)
                raise TimeoutError("Monitter collaboration smoke exceeded its 420 second deadline.")
            try:
                process.wait(timeout=min(1, remaining))
            except subprocess.TimeoutExpired:
                pass
    except BaseException:
        stop_owned_smoke(process)
        raise
snapshot = json.loads(state_path.read_text())
deliveries = snapshot.get("collaborations", [])
delegations = [item for item in deliveries if item["kind"] == "delegation"]
peer_messages = [item for item in deliveries if item["kind"] == "message"]
assert process.returncode == 0, f"Native smoke failed; inspect {run}"
assert len(delegations) == 1, f"Expected one idempotent delegation, got {len(delegations)}"
delivery = delegations[0]
assert delivery["status"] == "completed" and f"PEER_OK {marker}" in (delivery["result"] or ""), "Peer did not return the real acknowledgement"
parent = next(t for t in snapshot["tasks"] if t["id"] == delivery["fromTaskId"])
child = next(t for t in snapshot["tasks"] if t["id"] == delivery["toTaskId"])
assert child["parentTaskId"] == parent["id"] and child["hostId"] == peer_host["id"], "Task lineage or host routing was wrong"
assert parent["nativeSessionId"] and child["nativeSessionId"] and parent["nativeSessionId"] != child["nativeSessionId"], "Both distinct native sessions must exist"
assert all(t["status"] == "completed" for t in snapshot["tasks"]), "Proof left an active or failed task"
assert len(snapshot["tasks"]) == 2, f"Expected coordinator and reviewer only, got {len(snapshot['tasks'])} tasks"
assert len(peer_messages) == 1, f"Expected one peer inbox message, got {len(peer_messages)}"
peer_message = peer_messages[0]
assert peer_message["fromTaskId"] == child["id"] and peer_message["toTaskId"] == parent["id"], "Peer message was not routed from reviewer to active coordinator"
assert peer_message["requestId"] == inbox_request_id and peer_message["text"] == inbox_marker, "Peer message markers were wrong"
assert peer_message["status"] == "completed" and peer_message["result"] == "Delivered to the recipient’s active turn via its Monitter inbox.", "Coordinator did not acknowledge the active-turn inbox delivery"
inbox_records = [m for m in snapshot["messages"] if m.get("collaborationId") == peer_message["id"]]
assert len(inbox_records) == 1 and inbox_records[0]["taskId"] == parent["id"] and inbox_records[0]["role"] == "user", "Inbox delivery was not durably persisted exactly once"
parent_outputs = [m["text"] for m in snapshot["messages"] if m["taskId"] == parent["id"] and m["role"] == "assistant"]
child_outputs = [m["text"] for m in snapshot["messages"] if m["taskId"] == child["id"] and m["role"] == "assistant"]
assert len(parent_outputs) == 2 and parent_outputs.count(f"HANDOFF_OK {marker}") == 1 and parent_outputs.count(f"RESUME_OK {marker}") == 1, "Coordinator did not complete exactly the handoff and resume turns"
assert child_outputs and child_outputs[-1] == f"PEER_OK {marker}" and child_outputs.count(f"PEER_OK {marker}") == 1, "Reviewer did not finish with exactly one acknowledgement"
assert sum(e["taskId"] == child["id"] and e["title"] == "turn.started" for e in snapshot["events"]) == 1, "Reviewer ran more than one native turn"
assert any(m["taskId"] == parent["id"] and m["role"] == "assistant" and f"RESUME_OK {marker}" in m["text"] for m in snapshot["messages"]), "Resumed coordinator did not consume the persisted peer result"
result = dict(ok=True, peer=args.peer, coordinator=args.coordinator, parentTaskId=parent["id"], childTaskId=child["id"], parentNativeSessionId=parent["nativeSessionId"], childNativeSessionId=child["nativeSessionId"], collaborationId=delivery["id"], inboxCollaborationId=peer_message["id"], marker=marker, inboxMarker=inbox_marker, artifact=str(run))
(run / "result.json").write_text(json.dumps(result, indent=2) + "\n")
print(json.dumps(result, indent=2), flush=True)
