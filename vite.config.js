import { defineConfig } from "vite";
import { sveltekit } from "@sveltejs/kit/vite";
import process from "node:process";
import { webDevPlugin, webDevProxy } from './scripts/vite-web-dev.mjs';
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(({ command, mode }) => {
  const webDev = command === 'serve' && mode === 'monitter-web';
  return {
  plugins: [...(webDev ? [webDevPlugin()] : []), sveltekit()],
  // Keep dependency transforms local when isolated worktrees reuse node_modules.
  cacheDir: ".svelte-kit/vite-cache",

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: webDev ? 18450 : 18420,
    strictPort: true,
    host: webDev ? '0.0.0.0' : host || "127.0.0.1",
    ...(webDev ? { cors: false, proxy: webDevProxy() } : {}),
    hmr: !webDev && host
      ? {
          protocol: "ws",
          host,
          port: 18421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
};
});
