Vendored from the published gpui-wry 0.7.0 crate:
https://static.crates.io/crates/gpui-wry/gpui-wry-0.7.0.crate

Original repository: https://github.com/longbridge/gpui-kit
Published archive revision: 0c830f4d257e69fdd17200650533ab4ca9a40cc0
Original repository path: crates/webview
Original license: Apache-2.0. LICENSE-APACHE is copied from that revision.

The wrapper's src/lib.rs and README.md are unchanged from the archive.
Cargo.toml is reduced to the package/library dependencies used locally and
changes its `wry` dependency from `lb-wry 0.53.3` to upstream `wry =0.55.1`.
This matches Monitter's Tauri runtime and avoids incompatible Linux GTK link
packages during dependency resolution, including on macOS. Publishing is disabled.

The wrapper methods and BrowserPane builder calls were checked against the
locally installed wry 0.55.1 source. Compilation and native lifetime/focus,
geometry and navigation behavior require separate validation; this adaptation
is not runtime proof. Keep Apache attribution when redistributing.
