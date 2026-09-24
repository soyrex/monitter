# Native browser tabs — implementation and acceptance

Monitter remains a Tauri shell. Browser panes use the host OS web engine rather
than bundling Chromium: WKWebView on macOS, WebView2 on Windows, and WebKitGTK on
Linux. The frontend owns tab placement and inert restore hints; the native
backend owns live webviews, profiles, permission decisions, and extension state.

## V1 boundary

- Bitwarden actually working is the acceptance criterion. The extension package
  format is not a requirement. Load an unpacked extension folder in v1; direct
  Web Store installation, updates, and broader marketplace UX follow later.
- HTTP Basic Auth must present a native username/password prompt. Credentials
  must not enter frontend state, workspace persistence, command traces, logs,
  or LAN/shared clients. Cancel and repeated failures must be bounded.
- Browser tabs share one persistent, app-private browser profile but not the
  Monitter shell's profile. WebKit may still use several content processes;
  sharing is not a fixed memory cap. Users can unload a tab to release its view
  while retaining its URL/title.
- A restored workspace is inert until the user explicitly reloads a tab. URLs
  and titles are private restore metadata; they may themselves contain secrets.
- External pages have no Monitter Tauri IPC capability. Browser commands are
  local-owner-only, and only `about:blank`, HTTP, and HTTPS navigation is allowed.

## Platform path

| Platform | Native engine | Browser profile / extensions | V1 status |
| --- | --- | --- | --- |
| macOS | WKWebView | Shared isolated WebKit data store; WebExtension controller on macOS 15.4+ | In progress; Bitwarden and Basic Auth unverified |
| Windows | WebView2 | Shared profile; unpacked extension API needs host UI and testing | Future validation |
| Linux | WebKitGTK | Shared website data; browser WebExtensions API is initial | Future validation |

Avoid introducing a browser bundle solely for API parity. If Bitwarden fails on
the native host and a larger runtime becomes necessary, measure size and memory
impact and bring that tradeoff to Alex before adding it.

## Acceptance checks

- [x] Pane/tab metadata, explicit unload, inert restore, scoped native command
      contract, and capability exclusion of external webviews are implemented.
- [x] Focused TypeScript/Svelte and Rust contract checks pass.
- [ ] Isolated native app smoke: create, navigate, resize, switch panes and
      workspaces, unload, restore, and close without orphan views.
- [ ] Basic Auth: local 401 fixture prompts, valid credentials load, cancel
      rejects, and no credential appears in state or logs.
- [ ] Unpacked Bitwarden loads with a visible toolbar popup, signs in, unlocks,
      autofills a local test form, and retains its extension data after restart.
      The user enters any vault credentials; the test never asks the agent to
      read or store them.
- [ ] Windows and Linux native behavior and memory scaling are measured before
      claiming those platforms work.

Passing an extension manifest load or showing its popup is not enough to mark
Bitwarden complete. Use `scripts/browser-basic-auth-fixture.mjs` and
`scripts/browser-native-smoke-fixture.mjs` for bounded local checks.
