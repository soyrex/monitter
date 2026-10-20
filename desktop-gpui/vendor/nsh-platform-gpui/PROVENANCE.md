Vendored from https://github.com/longbridge/nsh at `c1511fa4dda6ce2ed04e8f47ed087a1e7dee876b`.
Only `crates/gpui/nsh-platform-gpui` is vendored; NSH core, engine, base, and bundler resolve from that exact official revision. Apache-2.0; see LICENSE-APACHE.

Local changes: normalize inherited Cargo metadata/dependencies; use ContentMask::new in native.rs and render.rs for Monitter's patched GPUI. Product enables js only, with development, webview and video disabled.
