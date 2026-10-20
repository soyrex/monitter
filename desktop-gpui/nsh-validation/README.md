# NSH embedding validation

An isolated native host using Monitter's GPUI Kit **0.7.0**, its vendored and patched GPUI Fast core, and NSH `AppView`. It does not connect to Monitter's backend, providers, profiles or sessions.

NSH source is pinned to `c1511fa4dda6ce2ed04e8f47ed087a1e7dee876b` in the ignored `artifacts/nsh-validation/upstream` checkout. To reproduce, clone `https://github.com/longbridge/nsh.git` there and check out that exact commit before resolving/building this manifest. The lockfile pins the resolved Rust dependencies.

The validation-only `compat/` packages expose NSH's published `gpui-fast` / `gpui-fast-platform` 0.1.9 names by reexporting the exact same patched core used through Monitter's existing `gpui-pre` wrappers. These are adapters, not second rendering engines. NSH needs two clipping-mask constructor substitutions because Monitter extends `ContentMask`; apply `nsh-content-mask.patch` in the pinned checkout before building. `artifacts/nsh-validation/core-identity.json` records the resolved package paths.

The host mounts two independent NSH engines alongside GPUI Kit controls. Local TSX is compiled to a source module by `nsh-bundler`. Each bundle gets an explicit empty capability declaration and grant, a separate app ID and owned local data directory. Embedded limits are lowered to 32 MiB JS heap, 100 ms call budget, 3 s load budget and 1,000 nodes. The optional NSH webview/dev/video features are disabled. Native components remain trusted host code.

Build (after registering and claiming the shared build resource):

```sh
CARGO_TARGET_DIR=/Users/alex/code/monitter-gpui/.cargo-target/gpui node /Users/alex/code/monitter-gpui/scripts/gpui-toolchain.mjs cargo build --manifest-path /Users/alex/code/monitter-gpui-fast-poc/desktop-gpui/nsh-validation/Cargo.toml --locked
```

Runtime evidence is written to `artifacts/nsh-validation/runtime.jsonl`. This host is an architectural/interaction probe, not a production NSH feature, benchmark, or security certification. The decision and observed runtime checks belong in `docs/gpui-fast-poc/nsh-validation.md`.
