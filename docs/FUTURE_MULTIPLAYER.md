# Multiplayer channels (after v1)

User requirement: pair with a colleague or friend, join a group channel, and share selected agents.
Explicitly not part of v1. No accounts, invitations, network channel service or remote collaboration
listener will be shipped in the first local macOS build.

Preserve these distinctions when designing the later protocol:

- A person owns an agent; an agent uses a harness on a host. These are separate identities.
- A channel has human and agent membership, with an explicit policy for who can invoke each agent.
- Sharing an agent grants a controlled capability to ask it to work. It never shares SSH keys,
  provider login tokens, filesystem access or arbitrary terminal access with other channel members.
- The host retains control over tool approvals, workspace scope and whether that agent is available.
- Channel events need stable IDs, ordering, deduplication, reconnect cursors and attribution.
- A Mac, Linux host or other always-on host may own execution. A phone is another client.
- Removing a participant or withdrawing an agent must revoke future invocations.

V1 uses stable channel, agent, task and message IDs and explicit recipient routing. Its local data
model is a starting point, not a promise that transport, authentication or authorization already exists.

## Later iOS remote control

An iOS client should connect to the desktop execution service and subscribe to the same task events.
The Mac remains responsible for local CLI processes, SSH connections and credentials. A phone should
be able to read tasks, send messages and stop its authorized runs without installing a harness itself.
Pairing, authentication, reconnect history and scoped command permissions must precede a network
listener. The current Tauri command boundary keeps UI operations separate from execution, but is
not yet a remotely accessible API. A disconnected phone must not implicitly cancel an agent run;
desktop exit and host loss need explicit, separately reported lifecycle behavior.
