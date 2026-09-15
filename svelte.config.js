// Tauri doesn't have a Node.js server to do proper SSR
// so we use adapter-static with a fallback to index.html to put the site in SPA mode
// See: https://svelte.dev/docs/kit/single-page-apps
// See: https://v2.tauri.app/start/frontend/sveltekit/ for more info
import adapter from "@sveltejs/adapter-static";
import { vitePreprocess } from "@sveltejs/vite-plugin-svelte";

/** @type {import('@sveltejs/kit').Config} */
const config = {
  preprocess: vitePreprocess(),
  kit: {
    // The remote developer bridge is intentionally mounted below a dedicated
    // path so its narrowly scoped Tauri capability cannot match other pages.
    paths: {
      base: process.env.MONITTER_APP_UI === '1' ? '/monitter-app-ui' : '',
    },
    adapter: adapter({
      fallback: "index.html",
    }),
  },
};

export default config;
