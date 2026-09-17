# Agent discovery, messaging and delegation

Implemented 2026-09-10. The protocol is part of Monitter's native service. See
[CONTRACT.md](CONTRACT.md#cross-agent-collaboration) for its data and execution contract and
[VALIDATION.md](VALIDATION.md) for evidence and current harness limits.

## Using it

Open **monitter → Agent directory** (also available in Cmd-P). Edit an agent's collaboration profile
to describe its expertise, responsibilities and skills. The availability switch controls whether
other agents can discover and involve it. Existing agents migrate with collaboration enabled.

Ask a participating agent to discover a suitable peer and delegate a specific brief. Its native
harness receives Monitter's callable MCP tools and concise usage instructions for that turn.
Monitter creates the recipient's linked chat, runs its configured harness and returns its real result.
Both chats show the relationship, delivery status and errors in run detail. Agents can also exchange
messages through linked chats. A delivered message and a completed delegated task are distinct states.

The source chat's project follows the delegation; the recipient uses the project's folder for its
own host, falling back to the saved agent folder. Different host folders are not assumed to contain
the same checkout. Agents' declared skills do not grant new permissions.

## Callable tools

- `list_agents(query?)` discovers published saved profiles.
- `delegate_task(to_agent_id, title, message, request_id)` queues a linked task.
- `send_message(to_agent_id, message, request_id, task_id?)` queues peer context.
- `get_task_result(collaboration_id)` and `wait_for_task(collaboration_id, timeout_seconds?)`
  return actual status/result/error and deliver pending incoming messages to the current turn.
- `list_messages()` reads and acknowledges incoming peer-message delivery.
- `cancel_delegation(collaboration_id)` cancels an owned delegation.

Caller identity comes from a short-lived per-turn grant. The sender cannot supply another caller ID,
read arbitrary transcripts, target an unrelated existing chat or cancel someone else's unrelated task.
Retries reuse a stable request ID. Requests, results and lineage are private, durable local state.

## Harness integration

| Harness | Implementation | Validation scope |
| --- | --- | --- |
| Codex | HTTP MCP URL, environment bearer token, exact Monitter tool list and server-scoped tool approval; normal host auth/config retained. | Historical relay proofs and current HTTP validation are distinguished in VALIDATION.md. |
| Claude | HTTP MCP config with an environment-expanded Authorization header and exact Monitter tool allowlist. | Local adapter; SSH Claude remains unsupported by its interactive transport. |
| OpenCode | HTTP `mcp.monitter` config merged with the host's existing inline settings; OAuth disabled for the private bearer grant. | Version 1.18 configuration; local and remote merge checks. |
| ACP | HTTP MCP session definition with private Authorization header; HTTP capability required. | Local and tunneled endpoint, unsupported-capability and recovery checks. |
| Hermes | Receives delegated work through the existing gateway. | Gateway offline checks; callable outbound collaboration awaits an ephemeral ACP integration. |

Primary integration references: [Codex MCP](https://learn.chatgpt.com/docs/extend/mcp?surface=cli),
[Claude MCP](https://code.claude.com/docs/en/mcp), and
[OpenCode MCP](https://opencode.ai/v2/docs/mcp-servers).
Installed CLI schemas take precedence over newer documentation; OpenCode 1.18 uses `mcp.monitter`.

## Boundaries and recovery

Registration is per invocation. Monitter does not rewrite AGENTS.md, CLAUDE.md, global skills,
authentication or the user's other MCP servers. Independently running Desktop/TUI processes do not
receive these tools retroactively. Existing idle native sessions receive them when resumed by Monitter.

The Rust HTTP MCP broker and SSH reverse forwards bind to loopback only. Grants stay in memory,
child process environments and private session frames, never arguments or saved state. No MCP helper
is installed remotely. The owned tunnel is stopped when its harness process is released. Resident
harnesses retain their MCP configuration between turns, while idle tasks cannot execute tools.
There is no public relay or persistent remote daemon.

Delegation depth, cycles, per-task/root request counts and concurrent runs are bounded. A peer brief
is context, not new user authorization. Native permissions and purchase/publishing boundaries remain.
Token or monetary ceilings are not claimed where the harness cannot enforce them.

Unstarted deliveries survive restart. In-flight work becomes interrupted with explicit uncertainty;
it is not silently replayed. Parent cancellation cancels its collaboration descendants. Results and
messages remain available after resume. An active agent reads incoming messages through its inbox
or while waiting for results; acknowledgement means delivery, not completion of the peer's request.
