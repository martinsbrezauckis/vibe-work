/**
 * Vite + Vitest configuration for the Vibe Work UI.
 *
 * Role: bundles `src/` into `dist/` (which Tauri loads, see app/tauri.conf.json) and configures the
 * public UI test runner. `devBridge` (serve only) proxies `/__vw/*` to the Debug API for browser
 * development (see vite.dev-bridge.ts). `xtermCsp` rewrites xterm so it needs no inline styles
 * (see vite.xterm-csp.ts). Dependencies: @vitejs/plugin-react. Callers: `pnpm dev|build|test` in ui/.
 *
 * The dev server uses a fixed port because tauri.conf.json points `devUrl` at it; `strictPort`
 * makes a clash fail loudly instead of silently moving to another port.
 */
import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

import { devBridge } from "./vite.dev-bridge.js";
import { xtermCsp } from "./vite.xterm-csp.js";

export default defineConfig({
  plugins: [react(), devBridge(), xtermCsp()],
  // xterm is rewritten by xtermCsp (no inline styles under the CSP); pre-bundling would skip the
  // rewrite in dev, so dev and production run the same xterm code.
  optimizeDeps: { exclude: ["@xterm/xterm"] },
  clearScreen: false,
  server: { host: "127.0.0.1", port: 1420, strictPort: true },
  build: {
    target: "es2023",
    sourcemap: false,
    // Never inline the dictation audio worklet as a `data:` URL (Vite inlines small assets): the CSP's
    // `script-src 'self'` refuses a worklet module from `data:`.
    assetsInlineLimit: (file) => (file.endsWith("pcm-worklet.js") ? false : undefined),
  },
  // Public smoke and component tests share the synthetic browser setup.
  test: {
    globals: true,
    environment: "jsdom",
    setupFiles: ["../tests/public/ui/setup.ts"],
    testTimeout: 60_000,
    hookTimeout: 60_000,
    // Keep specs at this level; the dependency link below it must never be collected as tests.
    include: ["../tests/public/ui/*.test.{ts,tsx}"],
  },
});
