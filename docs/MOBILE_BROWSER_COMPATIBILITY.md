# Svelte/mobile browser compatibility

The production SvelteKit bundle uses `@vitejs/plugin-legacy` 8 with Vite 8.
`modernPolyfills: true` detects API usage across the client chunks and includes
the required core-js polyfills. For example, the owner workspace uses
`Array.prototype.toSorted`, which is newer than some of Vite's modern targets.

`renderLegacyChunks: false` is intentional: SvelteKit owns the client manifest
and HTML bootstrap, so Vite's normal SystemJS/nomodule injection does not work
as it does in a plain Vite app. Svelte 5 also requires native modern browser
features such as Proxy. This configuration does not establish support for IE
or other browsers outside Svelte's requirements.

`scripts/sveltekit-legacy-polyfills.mjs` wraps the existing static adapter and
makes the emitted HTML await the generated polyfill module before importing
either SvelteKit client entry. It preserves the configured base path, does not
rewrite hashed JavaScript, and fails the build if Kit's bootstrap changes to an
unrecognised shape. It is paired with the default `build` output directory and
the current adapter-static configuration, without precompression. Development
still uses Vite's native module path without production polyfills.

Verify with `npm run check` and `node node_modules/vite/bin/vite.js build`.
The direct Vite command avoids the separate LAN publication step in
`npm run build`. Serve `build/` with a static SPA fallback, then run:

```sh
MONITTER_TEST_URL=http://127.0.0.1:18451/ \
MONITTER_TEST_PRODUCTION=1 MONITTER_TEST_MISSING_APIS=1 \
MONITTER_TEST_ENGINE=webkit node scripts/mobile-interface-parity-ui-test.mjs
```

Repeat with `MONITTER_TEST_ENGINE=chromium`. The regression removes `toSorted`
before navigation, delays the polyfill response, verifies the app does not
start early, and then exercises the actual activity, Watch and Ideas flows.
This proves polyfill delivery and startup ordering in those test engines;
actual older iOS/Android devices still require their own acceptance checks.
