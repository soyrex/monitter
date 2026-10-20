# NSH native embedding validation

Validated on Apple Silicon macOS, 2026-10-20, in the isolated Fast worktree. This establishes native NSH embedding compatibility with Monitter's patched Fast core and GPUI Kit 0.7.0. It does not add an NSH pane to the product or establish a performance advantage.

## Reproducible source

- NSH: `longbridge/nsh` commit `c1511fa4dda6ce2ed04e8f47ed087a1e7dee876b` (0.1.1).
- Fast: existing vendored v0.1.9 core, including Monitter's patches.
- Current canonical main `fc864f0` merged into Fast by `d406371a612589e3d4162f43bf9e9b8842e2b5d3`.
- Harness: `desktop-gpui/nsh-validation/`, with lockfile and reproduction instructions.

The validation-only published-name adapters reexport the same actual `gpui` 0.2.2 core as Monitter's `gpui-pre` wrappers. Cargo metadata records one actual core. NSH's two `ContentMask` struct literals require `ContentMask::new(...)` to supply neutral defaults for Monitter's extra mask/fade fields; the minimal patch is retained beside the harness. NSH webview, dev and video features are disabled, so this does not validate those features or migrate Monitter's Wry dependency.

## Observed native behavior

The compiled host mounts two independently loaded NSH `AppView` entities beside GPUI Kit controls, with local TSX compiled by NSH's bundler and a trusted Rust component registered inside each surface.

| Check | Observed result |
| --- | --- |
| Native mounting | Both NSH surfaces render text, press handlers and editable inputs within GPUI Kit Root. |
| Input and isolation | Left count and `left native` input changed independently from right count and `right independent` input; both echoes updated. |
| Rust component bridge | Actual Rust component received each pane's count, input and heartbeat props. |
| Host controls | GPUI Kit host counter changed while NSH state remained present. |
| Empty capability policy | Both engines explicitly rejected the localhost fetch as **not granted**. This was a policy error, not merely a refused TCP connection. |
| Stop | Left native component dropped and left heartbeat stopped; right continued. |
| Reload | Left state reset to count 0 and default input while right retained count 9 and `right independent`. |
| Removal | Left tree disappeared and Rust component dropped. After the removal grace period, 25 observed heartbeat updates came from right and zero from left. |
| Remount | Restoring left rendered fresh state; increment worked (count 1) while right retained its state. |

Each load explicitly declares and grants an empty capability set. Engines use separate app IDs and owned artifact data directories, with 32 MiB JS heap, 100 ms call budget, 3 s load budget and 1,000-node limit. This probe does not certify the complete sandbox or test every limit. Native components remain trusted host code.

The final fixture build exited 0 in 12.77 seconds (build run `c8d72f0a-586f-49de-afbf-4d4621c60c72`). Native verification used the exact owned `Monitter NSH Validation.app` executable, PID 65197. No NSH error callbacks were recorded in that run. CUA observations sometimes lagged asynchronous loads; refreshing the app binding showed the completed remount and allowed another successful increment.

Local evidence under `artifacts/nsh-validation/`: `core-identity.json`, `runtime.jsonl`, `native-ui.png`, `native-ui-ax.txt`, build receipts and logs. These are retained artifacts rather than checked-in generated output. Earlier preflight launches are separated in `runtime-preflight.jsonl` and are not the final evidence run.

## Decision boundary

NSH's native AppView path is compatible in this pinned, minimally patched configuration. The remaining product decision is how to expose it through Monitter's authoritative backend and permissions; the harness makes no provider calls and touches no production profiles or sessions. The historical benchmark limitations in `results.md` remain unchanged.
