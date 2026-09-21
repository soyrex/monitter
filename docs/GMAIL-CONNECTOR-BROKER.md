# Shared Gmail connector broker

## Idea

Allow a user to connect one or more Gmail accounts to Monitter once, then make
bounded Gmail capabilities available to different model harnesses through
Monitter's MCP broker. Harnesses receive a temporary capability, never a Google
access token or refresh token.

This would replace the current mail-triage dependency on each individual
harness having its own Gmail connector. It would also let the native inbox open
a message directly, without spending another model turn merely to retrieve the
body.

```text
Google OAuth (once per account)
          |
          v
Monitter Gmail broker
  |- refresh token in macOS Keychain
  |- short-lived access token in memory
  |- native inbox reads (no model turn)
  |- bounded snippets sent to Jev for classification
  `- task-scoped MCP capabilities for Codex, Claude, ACP, and other harnesses
```

## Proposed boundary

Monitter should own authentication and API access, but should not pass Google
credentials into a harness, prompt, workspace, environment variable, or
transcript. Instead, its existing local MCP broker should expose a small
read-only tool surface, initially:

- `gmail.accounts.list`
- `gmail.search`
- `gmail.message.get`
- `gmail.thread.get`

Every call should require a short-lived Monitter grant bound to the task,
Gmail account, permitted tools, and optionally an exact set of message IDs.
Model choice and Gmail authority remain separate: routing a task to a stronger
model must never broaden its mailbox access.

The native inbox can use the same internal adapter directly. Jev should receive
only the bounded headers and snippets required for batched classification;
provider message IDs, OAuth credentials, attachments, and full message bodies
should remain outside Jev.

## Narrow MVP

1. Add **Connect Gmail** in Monitter using Google's installed-app OAuth flow
   with Authorization Code and PKCE in the system browser.
2. Store each account's refresh token in macOS Keychain. Keep access tokens in
   process memory and refresh them through the broker.
3. Implement the four read-only tools above with strict response-size limits,
   account attribution, timeouts, and audit events that exclude message bodies.
4. Issue task-scoped MCP grants to supported harnesses. Revoke those grants when
   a task ends, an account disconnects, or their short expiry is reached.
5. Change mail-card clicks to read the selected message through the broker,
   avoiding an extra LLM turn. Continue keeping full bodies in bounded,
   process-local cache only.
6. Feed normalized search results into the existing one-call-per-batch Jev
   classification path.

The first milestone must not expose compose, reply, send, forward, label,
archive, delete, attachment download, or arbitrary Gmail API requests. Any
future mutation would need a separate user-visible permission and confirmation
design.

## Safety and privacy requirements

- Treat all email content as untrusted data, never as agent instructions.
- Default to read-only grants with explicit account and task scope.
- Never serialize Google tokens or return them over Tauri, MCP, LAN, or ACP.
- Prevent confused-deputy access by binding every request to the selected
  account and the owning task.
- Log account, tool, task, result status, latency, and byte counts, but not
  credentials or full message bodies.
- Rate-limit calls and cap searches, messages, threads, MIME payloads, and
  attachment metadata.
- Disconnecting an account must remove its refresh token and invalidate every
  outstanding grant.
- Shared visitors must never receive Gmail tools or full message content.
- Keep human approval mandatory before adding any consequential mailbox action.

## Implementation options

### Strategic: Monitter-owned Gmail connection

This is the provider-agnostic solution. Every harness talks to the same
Monitter broker, users connect each account once, and the native UI can retrieve
mail without involving a model. It requires Monitter to register and maintain a
Google OAuth application.

Gmail body access requires a scope such as `gmail.readonly`; `gmail.metadata`
does not include bodies. Gmail read scopes are classified by Google as
restricted, so public distribution may require OAuth verification and a
security assessment. A local/private proof of concept is considerably simpler,
but must preserve the same credential boundary.

### Short-term: Codex connector direct call

Codex app-server exposes an `mcpServer/tool/call` operation. A focused proof of
concept could test whether Monitter can invoke the existing Codex Gmail tool
directly, using the thread's connector session but without starting a model
turn. This could remove the current click-to-read token cost quickly, but it is
Codex-specific and does not provide shared authentication to other harnesses.
The connector's OAuth token must still remain private to Codex.

## Evaluation

Compare the existing harness-owned path with the broker path for the same mail
tasks. Record connection prompts, successful reads, model turns and tokens,
end-to-end latency, Gmail API calls, Jev batch calls, access denials, and user
corrections. The initial success criterion is that multiple harness types can
read only the mail explicitly granted to their tasks, while native message
opening becomes faster and consumes no model tokens.

## References

- [Google OAuth for installed apps](https://developers.google.com/identity/protocols/oauth2/native-app)
- [Gmail message retrieval](https://developers.google.com/workspace/gmail/api/reference/rest/v1/users.messages/get)
- [Gmail OAuth scopes](https://developers.google.com/workspace/gmail/api/auth/scopes)
- [OpenAI Codex app-server](https://developers.openai.com/codex/app-server/)
- [Current Monitter mail-triage MVP](./MAIL-TRIAGE.md)
