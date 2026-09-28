# Mail MCP

Monitter's built-in Mail integration connects Gmail accounts to selected local
agents. It is a read-only MCP service: agents can search messages and read
message or thread contents, but cannot send, delete, label, archive, or otherwise
change mail. Gmail is the first provider; the account and provider boundary is
intended to allow future adapters without changing the agent-facing tool model.

## Connect Gmail

Open Settings → MCP & Plugins → Plugins → Mail. Create an OAuth client for the
Monitter desktop app and choose the **Desktop app** application type, following
[Google's native app setup guide](https://developers.google.com/identity/protocols/oauth2/native-app).
Enter the Google OAuth client ID and, when required, its client secret under
Google app setup.
The client secret is write-only in Monitter and stored in the macOS Keychain.
Monitter opens Google sign-in in the system browser. Refresh tokens are stored
in the Keychain; account metadata and per-account agent assignments stay in
Monitter's private local state. Disconnecting an account removes its local
credentials and grants.

For initial Google setup:

1. Enable the Gmail API in the Google Cloud project that owns the OAuth client.
2. Configure the OAuth consent screen. In testing mode, add the Gmail account as a test user.
3. Create an OAuth client of type **Desktop app**, then enter its client ID and optional client secret here.
4. Select **Connect Gmail**, approve read-only mail access in the system browser, then select the agents allowed to use that account.

See [Google's installed-app OAuth guide](https://developers.google.com/identity/protocols/oauth2/native-app)
and [Gmail authorization scopes](https://developers.google.com/workspace/gmail/api/auth/scopes).
Disconnect removes Monitter's local tokens and grants; it does not remove the
Google app's consent from your Google account. Google consent can separately be
removed through your account's connected-app settings.

Google OAuth configuration is supplied by the user; Monitter does not ship a
shared OAuth client credential. Public Gmail distribution requires Google
OAuth app verification and consent-screen configuration by the app publisher.

Each connected account has an explicit list of local agents that may use it.
Only local Codex, Claude, and supported ACP agents are eligible; internal and
AGY headless agents are excluded. Account grants are checked at request time:
adding an account to an agent that already has Mail tools takes effect
immediately; a newly assigned agent may need a new harness session to receive
the tools. Removing an assignment revokes access immediately. Accounts with no
assigned agents are not exposed to any agent.

Mail connection and credential controls are available only in the native
desktop UI. They are not available through LAN or shared browser clients.
Mail fetched by an agent becomes part of that agent's conversation context.
Use Mail with agents whose conversation content you are comfortable exposing to
that agent's configured model and provider.

## MCP tools

All tools are scoped to the connected accounts assigned to the calling agent.
The caller cannot request an account outside that grant. Tool results contain
read-only Gmail data.

### `mail_accounts {}`

Lists the Gmail accounts assigned to this agent. The response contains account
IDs, email addresses, and labels; it does not expose OAuth credentials.

### `mail_search {account_id, query, limit?, page_token?, label_ids?, include_spam_trash?}`

Searches Gmail using its standard `q` search syntax. `account_id` is required;
`query` is optional and follows Gmail syntax (for example
`in:inbox newer_than:7d`). `limit` defaults to 20 and is capped at 50. `page_token` requests
the next page. `label_ids` narrows results to Gmail labels, and
`include_spam_trash` defaults to false. Results return message and thread IDs,
sender/recipient metadata, subject, received time, and a bounded snippet; use
`mail_read` to retrieve body text.

### `mail_read {account_id, message_ids, max_body_chars?}`

Reads 1–20 messages by Gmail message ID from an assigned account.
`max_body_chars` defaults to 20,000 and is capped at 50,000 characters per
message. The result contains normalized message metadata and bounded plain-text
bodies. Results explicitly report `bodyTruncated` and `bodyUnavailable` when relevant.

### `mail_thread {account_id, thread_id, max_messages?, max_body_chars?}`

Reads a Gmail thread from an assigned account. `max_messages` defaults to 20
and is capped at 50. `max_body_chars` defaults to 20,000 and is capped at
50,000 characters per message. `threadTruncated` reports omitted messages. The adapter fetches thread IDs first, then only the selected message bodies.

## Frontend command contract

The desktop bridge uses these native-only commands. Configuration mutations
return the updated `MailConfig`; OAuth access and refresh tokens are never
returned to the UI.

- `mail_get_config {}` → `MailConfig`
- `mail_configure_gmail {revision, clientId, clientSecret?}` → `MailConfig`
- `mail_start_connect {}` → `{attemptId, authorizationUrl, expiresAt}`
- `mail_connect_status {attemptId}` → `{status, error, account}`
- `mail_cancel_connect {attemptId}` → `void`
- `mail_set_account_agents {revision, accountId, agentIds}` → `MailConfig`
- `mail_disconnect {revision, accountId}` → `MailConfig`

The renderer clears the secret field after every submission. OAuth attempt IDs
are short-lived and status polling stops on completion, cancellation, timeout,
or component teardown.
