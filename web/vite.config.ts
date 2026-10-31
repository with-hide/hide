import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import type { Plugin } from "vite";
import { defineConfig } from "vitest/config";
import { WEBGL_ADDON_MODULE, fixWebglAtlasSlots } from "./src/webglAtlasSlots";

const hided = process.env.HIDED_ORIGIN ?? "http://127.0.0.1:9876";

// Rewrites the WebGL addon as it is bundled; src/webglAtlasSlots.ts says why.
const webglAtlasSlots: Plugin = {
  name: "hide-webgl-atlas-slots",
  enforce: "pre",
  transform(code, id) {
    return WEBGL_ADDON_MODULE.test(id.split("?")[0]!) ? { code: fixWebglAtlasSlots(code), map: null } : null;
  },
};

export default defineConfig({
  plugins: [webglAtlasSlots, react(), tailwindcss()],
  // The dev server's prebundle would hand the addon to esbuild past the
  // plugin above; the addon is one self-contained module, so it needs none.
  optimizeDeps: { exclude: ["@xterm/addon-webgl"] },
  build: {
    rollupOptions: {
      // The desktop shell and the phone app (PRD mobile-companion D-16): hided
      // serves `mobile.html` at `/m/` and both share `/assets`.
      input: { main: "index.html", mobile: "mobile.html" },
    },
  },
  server: {
    host: "127.0.0.1",
    port: 5173,
    proxy: {
      "/ws": { target: hided, ws: true },
      "/health": hided,
    },
  },
  test: {
    environment: "node",
    reporters: ["default", "./e2e/unit-failure-reporter.ts"],
    globalSetup: ["./e2e/unit-node-binary.ts"],
    include: ["src/**/*.test.{ts,tsx}", "e2e/windows-sockets.unit.ts", "e2e/platform-program.unit.ts", "e2e/bounded-child.unit.ts"],
  },
});
