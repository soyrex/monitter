# Monitter GPUI desktop experiment

The [UX style guide](UX_STYLE_GUIDE.md) defines the Tauri-inspired spacing, forms, menu/button/tab treatment, surfaces, typography, and review rules. Shared source tokens live in `src/design.rs`.

## Tern tile layout

Choose **Preferences → Appearance → Tile layout → Tern** for rounded,
separated tiles, compact pill tabs and a quiet accent focus outline, based on
[Stencil's Tern](https://stencil.so/tern). **Monitter** retains the original
contiguous dock and the selected Classic/Modern tab style. Light/dark palettes,
accent, fonts and interface scale remain independent.

The choice applies immediately to retained panes and persists in the private,
revision-guarded `native-ui.json` appearance store. Older stores default to
Monitter. Use a paired GUI/server build: an older server's strict preference
schema does not accept the new `tileTheme` field. Packaging does not install the
build or restart an existing server.

Native review used an isolated macOS preview: live switching in both directions,
rounded WKWebView corners, split tiles, pill tabs, focus, terminal input/output,
and saved selection after GUI restart. That initial review did not replace or
restart the installed app or existing backend. Screenshot:
[`tern-tiles-native.jpg`](../artifacts/gpui/visual/tern-tiles-native.jpg).
Chat density variants and Linux rendering were not exercised. Focused regression
checks cover legacy appearance defaults, strict theme serialization, corner
geometry and retained-pane layout restoration.

Tern's tile outlines and accent focus outlines now follow **Border and divider
opacity**. Navigation and usage/resources have separate rounded sidebar bubbles,
including the compact rail; their borders use the same control. Splitters retain
their grab bands but show only a short hover/drag grip, not window-spanning rules.
Preferences paint above tile chrome.

The updated macOS preview was visually checked with live border-control values
of 0%, 50% and 100%, retained terminal/browser tiles, both sidebar widths, layout
switching and the provider-usage dialog. The sidebar checks passed 38 tests and
the retained-pane layout checks passed two tests. Screenshot:
[`tern-borders-sidebar-native.jpg`](../artifacts/gpui/visual/tern-borders-sidebar-native.jpg).

This verified GUI build is installed at `/Applications/Monitter.app`; the previous
bundle is retained under
`/Applications/.monitter-desktop-backups/tern-borders-1791753636135/`.
The installed signature and executable checksum match the packaged build.
The existing GUI and backend processes were left running, and the backend bundle
was not replaced. Installation is not a hot reload: the interface changes appear
on the next GUI launch. Theme preference persistence still requires the paired
server upgrade described above.

## Linux build checkpoint (2026-10-09)

The source is available on `main`. For Omarchy / Arch Linux on
x86_64, see [LINUX.md](LINUX.md) for dependencies, build and launch commands.
Linux builds produce sibling GUI/backend executables; the backend runs headlessly
and continues after the GUI closes. macOS retains its app bundles and menu-bar server.

Current host verification: 352 GUI tests and five isolation tests passed, and
Svelte/TypeScript checking completed with zero errors (24 warnings). The latest
portable-startup changes also pass macOS Cargo checking. Linux compilation and a
live Omarchy run have not yet been verified. Embedded browser and native microphone
capture remain macOS-only; Hyprland wallpaper capture, captured in-app blur,
and macOS-native edge features are not implemented on Linux. The shared window
surface mapping requests native blur on every platform: AppKit vibrancy on macOS,
acrylic on supported Windows versions, and KWin blur on Wayland when the compositor
advertises that protocol. X11 and Wayland without that protocol retain alpha
transparency without a blur guarantee. Windows/Linux runtime blur remains
unverified. Older package/PID reports below are historical.

## Shared design language and chat refinement (2026-10-07, partial native review)

The latest optimized GUI package is `build-1791351504309`, installed/running as PID 56077. The centered sidebar wordmark now occupies about 70% of its prior rendered size: native ink bounds changed from 314x60 to 219x42 screenshot pixels. Header and control geometry stay unchanged. This size-only change was optimized-compiled and native-reviewed; the previous 14 library +229 desktop tests were not rerun. Strict signatures/hash match; server9031/binary, production75865,12tasks140messages and authoritative settings are preserved. Proof: `artifacts/gpui/release/logo70-native-final.json`.

The preceding shared-style GUI package is `build-1791331003284`, native-reviewed as GUI PID 13881 at that checkpoint. The UX style guide is backed by shared 5/10/15/20/30 design-pixel spacing, 5px themed control radii and 10px cards, applied to chats/composer, sidebar/footer, preference forms, and rich composer menus. Text fields have explicit accessible names and popup content respects the selected interface font. 14 library + 229 desktop tests pass; native chat/menu/preferences review and bounded phone screenshots confirm this first pass. Strict package/installed signatures and hashes match; retained server PID 9031/binary, production PID 75865 and task/message continuity remain protected. Every density/subpanel and full desktop parity remain open. Proof: `artifacts/gpui/release/style-guide-native-final.json`.

The preceding ring follow-up is optimized package `build-1791322208823`, installed as GUI PID 90192. Usage rings use an equal responsive diameter after reserving sidebar edges, inter-ring gaps and existing account controls, so all five providers stay in one row. Artwork scales with the rings; the clock remains 78 design pixels. Native screenshot review confirms the current-width row. Strict package/installed signatures and hashes match; server PID 9031, production PID 75865 and all 12 tasks/140 messages are preserved. No new tests were added for this geometry-only change; optimized compilation and native review passed. Proof: `artifacts/gpui/release/rings-native-final.json`.

The current source uses Tauri's conversation event whitelist before building the retained process tree. Usage/context accounting does not create visible steps or inflate tool counts. Collapsed turns show a tool count and a bounded readable preview of the latest tool; expanding them reveals retained semantic branches and per-event detail disclosures. Saved-rule approval cards and approved-by-rule history are hidden from the conversation, while pending approval requests and explicit approval management remain available.

This wave also removes recursive split-frame fills, restores the full empty-tab drop target, preserves active sidebar drag width before release normalization, and gives idle task rows transparent backgrounds with fading hover. Sparkles reverse a 500 ms visibility fade without restarting their exit phase. Usage rings are 72 design pixels and the clock is 78. Native modals capture one bounded blurred GPUI scene and apply one-quarter of the configured window transparency to their panel fill. The tab and header now share task/window-scoped world coordinates and a phase clock, rather than independently positioning their stars. All 228 desktop tests pass. Native inspection of package `build-1791320708880` confirms hidden accounting/saved-rule clutter, readable tool-count summaries, nested real tool disclosures, larger rings/clock, and the resource modal's blurred workspace backdrop. The final optimized package `build-1791321164222` is installed and running as GUI PID 87820. Native review confirms the corrected narrow branch-label/timestamp layout and accessible branch controls. Server PID 9031 and its instance/binary remain unchanged, with all 12 tasks and 140 messages preserved. Strict signatures and installed/package hashes match. Proof: `artifacts/gpui/release/curated-native-final.json`. No task was running for a live star-continuation check. Tests, package and native evidence are tracked separately; this does not establish full parity.

## Preferences, navigation and sidebar cohort (2026-10-06, partial native review)

At that earlier checkpoint, the optimized GUI package was `build-1791311065568`, installed as PID 48970. Fourteen library and 201 desktop tests pass, strict package/installed signatures match, and server PID 9031 retains the same instance and every prior task/message. Native review confirms preferences geometry/category scrolling, terminal palette controls, tab icons, compact rail, scoped tab creation/closing/movement/cycling, and menus aligned above their buttons. Double-clicking empty Modern strip space zooms the native window; pointer move distance remains unproven by cropped captures. Full blur, divider gesture and settings/remote parity remain open. Proof: `artifacts/gpui/release/native-ui-after.json`.

The current source adds a retained preferences overlay sized to 90% of the native viewport. Opening it captures the completed GPUI scene once into memory, downscales it to at most 960 pixels on its longest side, and blurs it on a worker. The retained backdrop replaces sharp underlying surfaces until dismissal. Compact pane-local details use the same mechanism with an owning-pane crop. This excludes native browser/webview pixels and freezes the backdrop while the overlay is open; it does not request Screen Recording access or continuously capture the desktop. Settings reuse the existing authority for appearance, fonts, conversation options, agents/hosts, LAN, MCP, skills, secrets and mail. The new extension/secrets commands are authenticated owner-local IPC only and are not added to LAN/mobile permissions. An older retained server shows a capability gate rather than pretending these commands work. The ten terminal ANSI palettes are now a GUI-local global setting, observed by every retained terminal pane. Remote phone pairing remains a separate gap.

The next visual cohort has 214 passing desktop tests. It uses the original illustrated Tauri wordmark and mark, full empty-tab-strip window dragging only when no tab drag is active, uniform window alpha for background surface fills, and a usage/resources-only full-width footer. A repeated retained-pane add now selects the existing leaf instead of creating a duplicate; this does not establish the original cause of every invalid layout. Running-task Sparkles are retained child entities in panes, tabs and task rows, with one 30-fps clock per field, static reduced-motion rendering, source stagger/geometry and readable accent. Stopped tasks release their cached fields. Motion preference changes update existing fields immediately; System/Subtle currently sample the macOS setting at GUI startup. Alternative Grid/Matrix pane effects remain unimplemented. Native verification and package activation are recorded separately below.

The following source pass applies the exact same window alpha to base, card, input, menu and widget backgrounds, preserving their colors and foreground opacity. Intentionally clear spacers remain clear. Modern tabs paint one fill per region; the selected tab and the pane header share the paper color without the generic eight-pixel inset or a full-strip border below the selected tab. Existing inactive-pane dimming also applies to its tab. Descending running-task sparkles extend into the header and fade toward its lower edge.

Chat headers now place the existing scoped dock menu (Zoom and Close) beside the pane-local sidebar toggle. They no longer open the former chat-settings pane or contain a reload icon. Reload and older-history controls live in the first loaded row and appear only after the user scrolls to that history boundary away from tail-follow. Dock menu state is read at opening time so retained rendering cannot leave its zoom label stale. This source pass has 216 passing desktop tests; its final package/native review is recorded separately.

The sidebar resizes to a compact 80-point rail (independent of interface scale) and keeps the expanded width. Modern tabs show type icons; their unused area starts native window dragging without repurposing tab/control hitboxes. Composer Access and Model controls use menus above the triggering control, aligned left and right respectively. Usage rings embed the existing real provider SVGs. Transcript refresh respects GPUI's explicit tail-follow state so an unmeasured virtual row cannot snap a reader back to the bottom.

Standard workspace shortcuts follow Tauri's modified-arrow priority even while the composer is focused; ordinary selection keys remain native. The Vim command bar and dock operations are source implemented. Both shortcut dispatchers suppress hidden-workspace mutations while preferences, palettes, the Vim command bar or a draft view are open, and while a focused chat composer has actual IME marked text. The marked-text fixture and native preferences Cmd+W check pass. Complete multiple-tab auto-hide behavior, broad native interaction acceptance and full-pane working animation designs remain incomplete. System/Subtle/On/Off now apply GPUI reduced-motion policy with private GUI-local persistence. Compilation/tests/package/native evidence are recorded separately; this source description is not acceptance of full parity.

## Installed preview and responsiveness investigation (2026-10-06)

The installed optimized GUI comes from `artifacts/gpui/release/build-1791304708137`, running as PID 23873 with the retained experimental server PID 9031 / instance `80921ad9-7a28-47c5-b740-92a76a7eb49d`. Its installed executable hash matches the immutable package and strict ad-hoc verification passes. The server executable was not replaced or restarted. Native inspection after activating the window shows the two split chats, message actions, context/access/model controls and the user's saved draft; every task and message ID recorded before the update remains present. `artifacts/gpui/release/optimized-install-20261006.json` records this bounded proof. The phone screenshot is a 162,465-byte JPEG with roughly 218 KiB inline payload. Production remains separate.

This cohort avoids invalidating the whole Desktop for each composer draft keystroke and retains one cancellable goal poll per task activity view, without notifying unchanged activity projections. Native pane content uses GPUI's retained cache when accessibility is inactive; while an accessibility tree is requested, it rebuilds content because this pinned GPUI version does not replay AccessKit nodes from cached prepaint. The visibility canvas stays outside the cache. A native-widget regression proves an unrelated parent redraw reuses content, a content notification invalidates it, and visibility is recorded on the replayed frame. All 14 library and 175 desktop tests pass. The ordinary optimized release package is installed; measured typing latency and matched workload performance are not accepted from tests or native observations alone.

Process measurements distinguish the separate GUI from the backend's inclusive process-tree counter. Most measured backend memory belongs to the Node/MCP helpers of two resident Codex sessions, while the GUI itself measured about 100 MiB RSS. The counter also includes exited processes' last resident sizes, so it is not an exact live-memory attribution. `artifacts/gpui/debug/memory-accounting-20261006.json` records the bounded samples. No provider session or helper was stopped to lower this number.

## Historical packaged preview and source review (2026-10-05)

The last packaged preview is `artifacts/gpui/debug/build-1791049707621/Monitter GPUI.app`. It was reopened without compiling on 2026-10-05 as GUI PID 63091; because the former experimental backend was no longer running, it launched the sibling server as PID 63098 / instance `b9e39648-cd3a-44e8-b963-409f752c0bad`. The restored snapshot has eight tasks and 54 messages, with no running tasks. This is a fresh server instance, not new proof of uninterrupted provider continuity. Both packaged apps pass strict ad-hoc signatures; the package corresponds to the earlier 166-test source. Production PID 75865 remains untouched.

Newer, unpackaged source adds message Copy / Reply / Fork controls hidden at idle and revealed on hover or keyboard focus. Reply saves visible text as a scoped Markdown attachment without sending. Fork captures a real message cutoff, reads the real target model catalog and invokes the existing backend fork command; it opens the result only after the canonical snapshot contains the returned task. The preceding source cohort passed 167 GUI tests, including bounded Markdown reply attachment coverage. Subsequent source fixes re-resolve Reply text at click time, guard retained history row bounds, match Tauri metadata ordering/fonts and read computer activity from task-scoped flat Snapshot messages. These follow-ups parse successfully but have not been compiled or tested. Native actions have not been exercised. Packaging is held below the unchanged 6 GiB warm-build reserve (1.2 GiB observed) after a concurrent iOS/Xcode build; its caches were untouched. See `artifacts/gpui/debug/message-actions-followup-source-review.json` and the earlier `message-actions-source-review.json`.

The newest native renderer export shows a real persisted active Codex goal in a compact themed dock above the composer, including its objective, status and actual token count. The goal disclosure and clear control have not been exercised. The permanent “Load earlier” / “Jump to live” toolbar is gone; history paging now follows native user scrolling, with a guarded upward-gesture fallback for short pages and live updates released when returning to the bottom. The existing pane-local Details work-plan renderer remains; its stale duplicate above the transcript was removed and earlier plans now start collapsed. Native inspection recovered on 2026-10-05. A fresh OS screenshot confirms the restored three-chat workspace and absent permanent history toolbar; goal disclosure/clear and history paging remain untested in that session. See `artifacts/gpui/debug/goal-chat-visual-review.json`.

The sidebar resource footer now fills the available sidebar content width without a bordered rounded card. CPU/RAM history is painted synchronously with native canvas paths instead of creating fresh asynchronously decoded image assets on every redraw. Native screenshots show changing clock hands and metrics with visible traces. This is sampled visual evidence, not sustained flicker acceptance. On 2026-10-05 a native AX click opened the resource dialog and Close returned to chats. Its long root command exposed a width defect that hid the metric columns; a subsequent uncompiled source correction bounds the row/button/label width chain and aligns fixed metric columns. Native inspection failed in the earlier cohort but recovered on 2026-10-05; sustained flicker and the corrected dialog layout remain open.

Pane resizing now ends on captured release, recovers a missed release on the next no-button move, and cancels on deactivation. Listener order precedes child content and stale GPUI drag payloads are cleared; Terminal no longer consumes foreign mouse releases. Three public-widget behavioral regressions pass, including swallowed release and a second independent drag after recovery. Pill handles were removed; engagement paints the full divider Monitter blue. Idle dividers were inspected, but native blue-hover/release acceptance remains open.

Terminal panes retain individual session identity and ownership. Only the owned native-test PTY was closed; the unrelated user's terminal remains. Empty-pane cards and individual Modern browser tabs retain their earlier bounded evidence. New Chat still uses a global retained form. Alex has deferred pane-local draft integration behind visual matching; the GUI-only draft-store module remains unwired. Native window bounds are not yet persisted across GUI restart.

Earlier bounded proofs remain valid for their named cohorts: a real read-only GPT-6-Luna turn survived forced GUI exit/reopen with stable server, session hash and backend PIDs, and its exact reply was stored and rendered. Acknowledged text was cleared from saved drafts and stayed blank across forced restart with a sole GUI. An unsent harmless text attachment's metadata/card also survived restart and explicit removal. These do not establish all-provider continuity, goal auto-continuation, upload/send or voice acceptance. Evidence is in `artifacts/gpui/debug/traced-provider-continuity.json`, `draft-ack-native-review.json`, `attachment-draft-continuity.json`, `browser-dock-native-review.json`, and `empty-landing-native-review.json`. Complete pixels, workflows, window blur and matched release performance remain open. Production Monitter and mobile/shared clients are untouched.


This is a native GPUI desktop client backed by Monitter's existing Rust service. It remains an experimental partial implementation; the target is full desktop parity. The embedded browser uses a native macOS WKWebView.

The backend uses a separate experimental profile at `~/Library/Application Support/com.monitter.gpui.experimental`. It does not open or copy the installed Tauri profile. Existing CLI authentication is used only when a message is explicitly sent. Creating an agent stores configuration; it does not establish that the harness is installed or authenticated.

The new architecture places the existing Rust Service in `Monitter GPUI Server.app`, a separate menu-bar process. The workspace GUI connects through private same-user IPC and cannot shut down the server when it closes. Rebuilding/restarting the GUI preserves a compatible running backend; changing backend code requires an explicit server restart. The menu offers Open Monitter and Quit Server. Quit Server stops its runs. Packaging writes immutable sibling GUI/server bundles and does not replace either running executable. Lifecycle tests and live process-continuity evidence are recorded separately from feature-parity claims.

## Historical checkpoint

That earlier preview was `build-1791021493945` (GUI PID 87880) against retained server PID 68145. It has 125 passing GUI tests and strict signatures for both apps. Native window inspection verifies modern tabs, pane-local sidebars, composer Git/context/access/model controls, slash-menu keyboard activation, native project-picker cancellation, and accessibility controls surviving tab reactivation. Mail default-reader improvements are under review. Provider reply continuity, complete workflows, exact pixels and matched release performance remain open. The sections below contain historical checkpoints; use `parity.json` and the named proof artifacts for the latest per-feature boundary.

## Partial native source

- The main shell routes chats, archives, agents, projects, browser, settings, mail, channels, terminal, usage, schedules, and selected-task subagents. It listens for committed Service changes and applies typed revisioned Rust deltas to a root-owned cache. Exact revision checks reject stale pages; resets and explicit removals invalidate retained history. A shared compatibility projection still feeds existing native panes.
- Chat source includes per-task text drafts saved and restored through native preferences, a variable-height virtualized chronological message/activity timeline, selectable Markdown, paginated history, reader anchors, and held live updates while reading older messages. Activity rows retain event identity and load full detail only on explicit expansion. Message sends use the shared `DesktopService` and its native app-server continuation path; GPUI does not implement a duplicate provider continuation. Approval/input cards and queued-message editing/removal use existing backend validation. Grouped process/reasoning presentation, thinking elapsed status, and run-summary fidelity remain incomplete; per-assistant response metadata now has a native disclosure based only on stored attribution fields.
- Compact Goal HUD and native scroll history are packaged and renderer-reviewed as described above; live interaction remains unverified. Hover message actions remain incomplete.
- Native panes now cover channel membership/transcripts/sending; agent and host editing; project/board editing and notes; selected-task subagent status/transcripts/actions; task-scoped read-only Git status and selectable diffs; work-plan summaries; terminal sessions; schedules; and provider/context usage plus process metrics. These are partial source implementations; combined compilation now passes, while feature-specific interaction and parity verification is incomplete.
- Mail source supports Gmail setup/connect status, account-agent assignment, inbox refresh, eligible-card filtering, detail, and summary requests. An explicit rich-reader action opens sanitized HTML in an ephemeral WKWebView with scripts, navigation, and remote resources blocked; plaintext remains available. Quote styling and bounded embedded raster images are supported. Attachments and runtime network/lifecycle verification remain pending.
- The native browser source supports multiple retained tabs, add/select/close, URL navigation, back/forward/reload, and keyboard tab/address controls. Extension workflows, a visit-history surface/persistence, and browser split visibility, extension and history workflows remain pending.
- `docking.rs` now hosts retained chat entities and auxiliary panes in native tabs/splits. Chat panes share snapshot data but own their drafts, focus, history, and busy state. Task text drafts and last-route preference save/restore are implemented in source; settings docking, sidebar/workspace selection persistence, forced/platform-quit persistence, and interactive lifecycle verification remain pending. Native browser visibility follows pane lifecycle. The custom header uses transparent native chrome with integrated traffic lights and a separate drag region. All instantiated auxiliary panes refresh from committed state even when another route is selected. Saved layout restoration now constructs real retained panes without starting agents or opening PTYs. Layout saves are debounced and revision guarded; normal window close flushes saves before cleanup.

## Parity and verification boundary

The full desktop remains incomplete. Rich composition and document previews, attachments and voice, full agent/provider/account workflows, remaining settings and hotkeys, browser extension/auth workflows, complete mail rendering and attachments, collaboration workflows, complete project and workspace behavior, runtime delta and deletion interaction verification, shared remote-control surfaces, and comprehensive lifecycle/visual/accessibility verification remain outstanding. `parity.json` tracks the source-level feature gaps; `implemented_unverified` means source behavior has focused tests where stated, but has not passed complete GUI acceptance.

An earlier staged package, `artifacts/gpui/debug/build-1790942786909/Monitter GPUI.app`, crashed as PID 38404 with SIGILL; its cause is unknown. The visual cohort test log `/tmp/monitter-gpui-visual-cohort-tests.log` reports 14 library and 53 binary tests passed, 0 failures, plus 0 doc tests. It includes form-input validation, image-preview bounds, and response-metadata formatter tests. Cargo emitted a rust-objcopy warning because its bundled libLLVM could not load; tests still completed successfully. Compact sidebar/toolbar changes made after the tested/package cohort are not covered by that log.

At that historical checkpoint, the native computer-use pipe failed before app inventory. Later native window inspections are documented above; this historical renderer export does not represent the current package. A GPUI renderer-frame PNG is available at `artifacts/gpui/visual/gpui-frame-1790947603918262000.png` (SHA-256 `1580092487473e96277c72e3c8b82a86dba6f2bd623f3366eb19bc9efac63ecf`). It shows the GPUI-rendered scene and is useful for visual review, but excludes macOS traffic lights/titlebar compositing and the native WKWebView child; it does not prove interactive behavior or full-window appearance. The frame shows typography/density and pane overflow that still need visual tuning. Compact sidebar/toolbar source changes followed the captured package and are not represented in this frame. No matched responsiveness/resource comparison has been measured. Mobile/shared compatibility has not been regression-tested at runtime.

## Running

From the repository root on macOS:

```sh
npm run dev:gpui
```

`dev:gpui` builds the sibling apps and launches the GUI; it connects to a running server or starts the packaged server if missing. It never kills an existing preview. The GPUI commands use the fork-local Rust toolchain when available, otherwise they require Rust 1.95 or newer. The helper changes `PATH` only for its Cargo child process and preserves `CARGO_HOME`.

Package a separate ad-hoc signed app bundle with:

```sh
npm run build:gpui
```

New bundles are written under `artifacts/gpui/debug/build-<timestamp>/Monitter GPUI.app`; `latest.json` in that profile directory records the latest path. The initial preview remains at `artifacts/gpui/debug/Monitter GPUI.app`. Each package preserves previous running bundles. `--release` selects the release profile. Packaging itself does not install or launch the app. The existing `build:mac` command continues to build the Tauri app.

Native commands share `.cargo-target/gpui` to avoid duplicate caches. Run `node scripts/gpui-disk.mjs` for an accounting checkpoint. Packaging records before/after disk usage and stops below 10 GiB available; cleanup is limited to idle, regenerable fork-owned caches, preserving app bundles, profiles and backups.

Bundled IBM Plex Sans and Mono regular, medium, and semibold faces are sourced from a pinned official IBM commit under `assets/fonts/`. Their OFL notices accompany packaged builds. Font registration is application-local; no system fonts are installed.

## Resource observations

Use `node scripts/gpui-process-sample.mjs PID [count] [intervalMs]` for read-only macOS process samples. It verifies the executable belongs to this fork and does not record invocation arguments, launch, signal, or interact with the app.

On 2026-10-02 at 11:43 UTC, three one-second samples of debug preview PID 23899 recorded 96.3 MiB root-process RSS and 1082.7 MiB summed RSS across 30 process-tree members. The root's OS-reported CPU moving average ranged 1.1–1.8%. The running workload was not controlled or classified as idle; these observations include descendant processes and cannot establish UI efficiency. RSS can double-count shared pages, XPC WebKit/GPU services may be outside the tree, and no frame-latency or equivalent Tauri workload was measured. Raw samples are in ignored `artifacts/gpui/measurements/process-23899-20261002T114311Z.json`. A release-build comparison under matched workload remains required.

Package `artifacts/gpui/debug/build-1790947177336/Monitter GPUI.app` was launched as PID 62670 for the renderer-frame capture above. The image is a renderer output, not an OS desktop capture; macOS traffic lights and the embedded native WKWebView are absent. Compact sidebar/toolbar source changes followed this package, and a newer build was underway when these notes were updated. Production repair `fd9869e` was installed with deferred restart; the running production PID 77732 remained unchanged. No production runtime outcome is claimed. Full feature parity and GUI interaction remain unverified.

## Budget checkpoint, 2026-10-02

The 5 million token goal exhausted its budget with full desktop parity incomplete. The compact styling cohort is now packaged at `artifacts/gpui/debug/build-1790948559499/Monitter GPUI.app`; exact executable observed running as PID 65424, strict ad-hoc signature verified, SHA-256 `95478637910d356555e72708ce84a3e1e6b5499892f276b5eb8acdf58993fc01`. The prior idle preview PID 62670 was replaced; production PID 77732 was not stopped. Latest source tests passed: 14 library + 53 binary.

Renderer export `artifacts/gpui/visual/gpui-frame-1790948612233197000.png` was inspected: sidebar controls fit two compact rows and the formatting actions fit one row. Narrow chat headers remain cramped; this is partial scene verification, excluding native macOS chrome and WKWebView children, not full interaction acceptance. The LAN-focused test log is `/tmp/monitter-gpui-lan-focused-tests.log`; its result must be collected separately. Approximately 14.4 GiB free and 5.1 GiB fork build cache; no cleanup needed or performed. Remaining work includes semantic transcript grouping, voice integration, relay/share support, workspace/narrow-pane polish, complete browser/mail/attachment behavior, interaction/accessibility acceptance and matched release performance measurement. The voice prototype remains unreferenced and uncompiled.

LAN checkpoint follow-up: all six socket-free focused filters completed, 9 tests passed and none failed. This does not verify a live LAN listener or unchanged iOS port compatibility.

## Separate server preview evidence (2026-10-02)

The split package `artifacts/gpui/debug/build-1790980712049` contains independently signed GUI and server apps. GUI PID 11072 reconnects to the existing server PID 73266, instance `132e8ee7-1703-4fed-9af2-dbfc3e162454`, from the prior immutable package. Production PID 75865 remains separate; the server was not replaced during GUI rebuilds.

`artifacts/gpui/server-continuity.json` records a real experimental PTY heartbeat advancing while the GUI was absent and reopened; that proof terminal was explicitly closed. A separate bounded real Codex smoke retained its native session and backend across GUI restart but produced no assistant reply, then was canceled and recoverably archived. `artifacts/gpui/agent-run-continuity.json` explicitly records `providerContinuityPassed: false`. The native menu click behavior remains unverified. Focused IPC tests passed 18/18 and GUI tests passed 106/106.

Fresh native screenshot inspection of this unique package confirms modern label sizing, the companion Active/Recent empty sections, composer controls and readable backend-resource footer. Comparison preferences now use Tauri’s 125 percent scale; the prior 150 percent preferences were privately backed up. Modern tab close geometry follows Tauri's design dimensions, but exact hitbox interaction and keyboard-focus visibility parity remain unverified. The compact Git strip is joined to the composer, with a bottom-edge context meter and mono footer labels. An explicit native custom accent matches the Tauri reference green; the previous preferences were backed up privately. Native inspection caught a duplicate button-hover rendering panic in an earlier package; the duplicate style was removed and this package opened successfully. Narrow panes still clip some labels. Full pixel and behavior parity remain incomplete.

Build-space reserves are 10 GiB for cold/release builds and 6 GiB for a warm debug rebuild with both existing executables. The latest package left 10.8 GiB free with a 6.0 GiB shared cache. Accounting remains in `artifacts/gpui/disk-usage.jsonl`; active server packages and production caches are retained.

Secondary sidebars now belong to each retained chat pane, including visibility, task context and resize state. Native two-pane inspection verified independent closing, and the original saved layout was restored afterward. The four task-detail tabs are now native-inspected: Timeline loads actual events, Right-arrow selects Approvals and shows keyboard focus, and Details/Subagents selection remains local. The task header spans conversation and sidebar. Narrow conversation message wrapping is fixed and visually verified. Modern single-tab strips, independent selected detail tabs and independent hiding were verified in a temporary split layout; the original layout was restored afterward. Compact pane overlay is the next implementation.


Latest pane-local inspection (package `build-1790982309109`, GUI PID 20069): 114 GUI tests pass and packaging strictly signs both apps. Native AX/screenshots verify compact overlay beneath the owning pane header, backdrop and Escape dismissal, retained detail selection on reopen, dark tint, Timeline events, and a wide fullscreen split. Exiting fullscreen restores the regular window and automatically hides compact details. The persistent server remains PID 73266; production PID 75865 stays untouched. Preview appearance uses current native Tauri saved left width 354.979187 design px, scale 125%, and comparison-only `MONITTER_GPUI_REFERENCE_DETAIL_WIDTH=416`. Default detail width remains 292; per-pane detail width/tab/visibility are not yet persisted across GUI restart. Backdrop blur, modern close focus, full process inventory, and comprehensive pixel/performance/provider continuity proof remain incomplete.

Latest verified package `build-1791017938986`, GUI PID 70123, retained server PID 68145 (instance `783b18fd-578d-4e8a-9dc6-ba02ecb4183c`): 116 GUI and 5 native preference tests pass. Per-task secondary-sidebar visibility preference, selected tab and width now persist in the private revision-guarded native preferences. A GUI-only restart restored Timeline and 416 design px without the reference-width environment override; compact startup still automatically hides the overlay. Native inspection caught tab clicks falling through to the overlay backdrop; the rail now occludes underlying mouse hitboxes, and Details selection stays open. Details shows the scoped real tracked-process empty state. Modern close controls become visible on keyboard focus; Enter closed only an unsent temporary tab and the original three-tab layout was restored. Sidebar CPU/RAM tooltips identify backend and children, excluding the separate GUI. Production PID 75865 remains unchanged. Full pixel parity, blur, idle close pointer-event fidelity and provider reply continuity are still incomplete.

Details cohort `build-1791018938513`, GUI PID 75264, retains server PID 68145/instance783b18fd and production75865. 119 GUI tests pass. Native inspection verifies real branch/staged/changed/untracked Git summary and review navigation, parent-linked unsent delegation draft, no new task/run from opening it, and Back to workspace. Work-plan segmented progress/expand controls and related-task navigation are source-tested; this experimental snapshot has no actual work plans or collaborations, so those runtime cases remain unproven. Git-review typography and slash-palette integration remain underway.


Native command cohort `build-1791021493945`, GUI PID 87880, retains server PID 68145 and production PID 75865. All 125 GUI tests pass and both bundles strictly sign. Native inspection verifies Down selects Settings and Enter opens preferences, a docked slash palette above the composer, `/project` opening a native dialog with the actual empty-project state and Escape cancellation, and chat accessibility controls surviving tab reactivation without an extra child click. The original three-tab layout and empty composer were restored. The server still has 7 tasks, 34 messages and no running tasks; no project assignment, provider command or mail action was performed. The Cancel accessibility label and default rich mail reader are follow-up source work. Removing the cached active-pane wrapper fixes accessibility replay in this pinned GPUI version, but its performance cost is not yet accepted. Debug samples with renderer export enabled do not establish native responsiveness or resource savings. Four obsolete idle app bundles were removed after process-path checks, reclaiming about 542 MiB; active server/GUI/proof packages and shared cache remain.


Native review checkpoint `build-1791023498148` (GUI PID 98838, retained server PID 68145): both bundle signatures pass strict verification and 129 GUI tests passed before packaging. The native composer exposes the compact microphone action; it was not clicked, so permission/capture/transcription are untested. The retained sidebar resource widget opens the real backend process group. Modern close buttons stay hidden during idle AX clicks, appear after real pointer movement, and direct clicking closes only the disposable unsent tab. Cmd+T/W also passed; the original three tabs and selected chat were restored. The native click adapter alone did not deliver hover, so its initial no-X observation was superseded by actual pointer movement. Mail's truthful no-assigned-account state was inspected without refresh or opening messages; its oversized inherited labels are under a separate styling correction. Debug idle sample `idle-retained-sidebar-3498148.json` excludes the separate server and is not a release performance comparison. Attachment draft integration is newer source, with 131 GUI and six focused native preference tests passing; it is not yet packaged or proven across restart. Production PID 75865 is unchanged. Complete pixel and successful provider-reply continuity remain open.

## Phone screenshot delivery

Capture native UI through CUA with `emit:false`, save the returned bytes, and process that existing image with `python3 scripts/gpui-screenshot-share.py SOURCE OUTPUT.jpg`. Never emit the full-resolution PNG directly. The script preserves the original, resizes and JPEG-encodes the share copy, and checks both a 300 KiB image limit and a 450 KiB base64-plus-headroom limit below the transport ceiling. Optional `--crop LEFT TOP RIGHT BOTTOM` produces a focused view without changing the app. Metadata beside each JPEG records source/output dimensions and both byte counts. For delivery, read the encoded JPEG in CUA and call `nodeRepl.emitImage({bytes, mimeType:'image/jpeg'})`, one screenshot per tool result. Local-file Markdown links do not embed in the phone app; native MCP image result blocks enter Monitter's generated-image attachment path. Both resent preview images were verified as stored JPEG attachments with inline data URLs (41,383 and 234,975 bytes). The 2026-10-05 native preview share is 176,214 image bytes / 235,976 estimated inline bytes at `artifacts/gpui/visual/share/native-preview-20261005.jpg`. Phone-side display confirmation remains separate.

The subsequent native reasoning disclosure review exposed structured empty reasoning rendered as raw JSON. The source now follows Tauri summary extraction, renders blank reasoning as Thinking, retains meaningful Markdown summaries and bounds activity labels in narrow panes. Both regression cases and icon coverage are present; only Rust syntax parsing has run for these follow-ups. No new build was started while a competing Rust build was active and free disk was approximately 0.36 GiB. Evidence: `artifacts/gpui/debug/reasoning-and-image-source-review-20261005.json`. The bundled phone owner workspace uses TaskTranscript/AttachmentList; legacy controller preview omissions do not establish an owner-mode defect. Two compact JPEGs were resent through direct native MCP image blocks; phone display remains unconfirmed.

### Streaming reply display

Preferences → Conversation → Streaming replies selects **As received** (default)
or **Typing**, with Slow, Normal, and Fast speeds. Typing reveals one Unicode
character at a time with a cursor; it drains an already-received reply before
switching to its final Markdown. **Show full reply** skips a completed reply's
remaining reveal. Loaded history, interrupted responses and hidden panes reveal
immediately. This is GUI-only presentation metadata in
`streaming-display.v1.json`; it never delays execution, alters the saved message,
or changes mobile/shared rendering. Each live message owns its timer and only
remeasures its own virtual transcript row.

### Existing sessions in a new tab

The new-tab landing screen keeps its New chat, Terminal and Browser actions,
then offers **Open an existing chat** underneath. The list groups running chats
first and other unarchived chats by newest activity. Search covers all visible,
unarchived sessions across projects and workspaces using title, agent, project,
model and working-folder metadata. Results are virtualized, with no recent-list
cap before filtering. Arrow keys select a result and Enter opens it in the
landing tab's own pane group, reusing an already-open chat rather than creating
another conversation or restarting its agent.
