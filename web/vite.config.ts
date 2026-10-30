import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

const hided = process.env.HIDED_ORIGIN ?? "http://127.0.0.1:9876";

export default defineConfig({
  plugins: [react(), tailwindcss()],
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
    // Throwaway diagnosis for with-hide/hide-private issue 354, never merged.
    setupFiles: process.env.DIAG_VARIANT === "perfile" ? ["./test/diag-intl-timing.setup.ts", "./test/intl.setup.ts"] : process.env.DIAG_VARIANT ? ["./test/diag-intl-timing.setup.ts"] : [],
    globalSetup: process.env.DIAG_VARIANT === "global" ? ["./test/diag-intl.global-setup.ts"] : [],
    include: ["src/**/*.test.{ts,tsx}", "e2e/windows-sockets.unit.ts", "e2e/platform-program.unit.ts", "e2e/bounded-child.unit.ts"],
  },
});
