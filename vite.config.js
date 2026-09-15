import { defineConfig } from "vite";
import { sveltekit } from "@sveltejs/kit/vite";
import process from "node:process";
import { webDevPlugin, webDevProxy } from './scripts/vite-web-dev.mjs';
const host = process.env.TAURI_DEV_HOST;

export const hotUiMarkerPlugin = (markerPath = '/__monitter_dev__') => ({
  name: 'monitter-hot-ui-marker',
  /** @param {import('vite').ViteDevServer} server */
  configureServer(server) {
    server.middlewares.use((request, response, next) => {
      if (request.url?.split('?', 1)[0] !== markerPath) return next();
      response.statusCode = 200;
      response.setHeader('Content-Type', 'application/json');
      response.setHeader('Cache-Control', 'no-store');
      response.end(JSON.stringify({ app: 'monitter', hotUiProtocol: 1 }));
    });
  },
});

// https://vite.dev/config/
export default defineConfig(({ command, mode }) => {
  const appUi = command === 'serve' && mode === 'monitter-app-ui';
  const webDev = command === 'serve' && (mode === 'monitter-web' || appUi);
  return {
  plugins: [...(webDev ? [webDevPlugin()] : []), ...(appUi ? [hotUiMarkerPlugin('/monitter-app-ui/__monitter_dev__')] : []), sveltekit()],
  // Keep dependency transforms local when isolated worktrees reuse node_modules.
  cacheDir: ".svelte-kit/vite-cache",

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: appUi ? 18420 : webDev ? 18450 : 18420,
    strictPort: true,
    host: appUi ? '127.0.0.1' : webDev ? '0.0.0.0' : host || "127.0.0.1",
    ...(webDev ? { cors: false, proxy: webDevProxy({ developerBridge: appUi }) } : {}),
    hmr: !webDev && host
      ? {
          protocol: "ws",
          host,
          port: 18421,
        }
      : undefined,
    watch: {
      // Opt in for isolated worktrees where native filesystem events are missed.
      ...(process.env.MONITTER_DEV_POLL === '1' ? { usePolling: true, interval: 300 } : {}),
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**", "**/.cargo-target/**", "**/.slim/**"],
    },
  },
};
});
