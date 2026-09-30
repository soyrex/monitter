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
rectangles. Window size is part of that cache key because native child views can
resize proportionally before the renderer corrects their position. Failed writes
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
