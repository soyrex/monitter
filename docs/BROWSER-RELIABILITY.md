# Browser reliability

The browser address is an editing draft, separate from the live page URL. Native
state polling cannot replace a draft while the user edits it, including after
blur. Enter submits it; Escape restores the latest page URL. Selecting another
tab starts with that tab's URL.

Native browser geometry is measured after DOM updates and frame-coalesced.
ResizeObserver watches the viewport and its ancestors, so resizing or closing
neighboring panes can trigger a position correction even if the viewport's own
size stays fixed. Reparenting refreshes the observed ancestor chain. PaneGrid announces its Web Animations geometry changes;
measurement follows ancestor animations through their final frame. Unlike CSS
transitions, these animations do not emit transitionend. Measurement stops when
motion ends; unchanged idle bounds generate no further reports.

Layout revisions update the observer rather than disposing it, avoiding a hide
and re-show on every splitter update. The existing serialized layout queue keeps
only the latest queued bounds per tab and now suppresses already-applied native
rectangles. Window size is part of that cache key so even fixed DOM rectangles are revalidated after host-window changes. Failed writes
remain retryable. Native position and size use one Tauri set_bounds dispatch.

Native state reads share one pending request per tab, discard responses after
retirement or a newer address change, preserve a requested address while navigation is pending, and avoid
replacing unchanged metadata or history objects.

## Focused verification

Run from the feature worktree:

```sh
node scripts/browser-state-flow-test.mjs
node scripts/browser-geometry-test.mjs
node scripts/browser-pane-geometry-test.mjs
node scripts/browser-tab-address-runtime-test.mjs
node scripts/browser-tab-ui-contract-test.mjs
npm run check
npx vite build
cargo test --manifest-path src-tauri/Cargo.toml --lib native_browser --jobs 2
```

The state-flow test executes the production functions with delayed native bridge
fixtures. Geometry tests cover position-only sibling resize/close, WAAPI motion,
real PaneGrid topology changes, window resize, idle deduplication, hiding, and
observer disposal in Chromium and WebKit. Address tests exercise polling,
selection/focus, draft blur, submit, continued editing, Escape, and tab changes.
These browser fixtures do not claim an installed WKWebView end-to-end test; a
native app rebuild is required to activate the Rust change.

Validated on macOS: Chromium/WebKit address, geometry and real-pane regressions;
state-flow and browser/command contracts; Svelte check (0 errors, 25 existing
warnings); production Vite build; and 5 focused native-browser Rust tests. The
native build used two jobs and a disposable APFS clone of the existing cache.
The task's Rust target, web build and Svelte/Vite cache outputs were removed after
validation. Main and the installed app were left unchanged.


## Native resize placeholder

The root surface captures pane and sidebar splitter pointer-downs before their
resize handlers run. During the drag, all native browser pages are hidden and
their shell viewports display the URL followed by “— browser”. Native bounds
continue updating while hidden. After pointer-up, cancellation, lost capture or
blur, settled animation frames precede the final bounds report and reveal.
Keyboard splitter changes and window resize bursts use the same guard with a
120 ms idle delay. Starting another resize cancels an outstanding reveal. Pane
resize capture now also releases on Escape, blur and component disposal.

`node scripts/browser-native-resize-smoke.mjs` builds a labelled local test page
and standalone shell fixture, then prints a side-by-side debug bundle command.
Keep the fixture server running while building and testing. Launch that bundle
with `MONITTER_BROWSER_SMOKE_DISABLE_LAN=1` to avoid the installed desktop's LAN
port. Its separate app identifier and browser profile do not replace the
installed app or its task data. The “Hold resize placeholder”, “Resize hidden
pane” and “Finish resize” controls expose each real WKWebView visibility phase
for visual verification; ordinary divider dragging tests native hit-testing too.

Follow-up validated in a separately identified macOS Tauri debug bundle at 125%
interface zoom: native WKWebView disappearance, hidden pane-bound changes and
reappearance were observed through accessibility and screenshots; an actual
splitter drag changed the ratio from 40% to 64% and revealed the page aligned
with the final pane. Chromium/WebKit regressions, Svelte check (0 errors, 25
baseline warnings) and the production UI build also passed. This verifies the
isolated fixture, not activation in the installed desktop.

## Live workspace pages and overlay ownership

Workspace switches park native pages with `browser_set_layout(..., false)`;
they never close or navigate them. A process-local registry reconnects restored
pane metadata to the same native child. Page DOM, input, scroll, history and JS
state remain native; an app restart or explicit unload still ends that page.
Layout bounds, revisions, deduplication and the write queue are shared across
panes. Viewport leases suppress stale teardown from a prior pane, and orphan
cleanup can hide a page even after its old tab metadata has moved. Browser moves
wait for pending creation/navigation to avoid closing a destination's child.
Mounted dialogs suspend overlays until exit completes, including usage and
process metrics. Terminal/Browser choices replace only a blank new-tab screen.

Focused checks: `browser-workspace-state-test.mjs`, `browser-state-flow-test.mjs`,
`browser-tab-ui-contract-test.mjs`, Chromium/WebKit pane geometry, address and
`browser-modal-visibility-test.mjs`. The existing modal-active-highlight test
fails its initial active-pane pseudo-element assertion on unchanged main too
(expected empty content, observed none); that unrelated styling test is not
claimed as passing. These follow-up checks exercise production functions and
web fixtures, not activation of this branch in the installed desktop.
