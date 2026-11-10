import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// The parts the user docs frame, built apart from the shell so the daemon never serves them and the dev-only
// gallery stays dev-only. Output: `docs-embed-dist/`, inside this worktree and ignored.
export default defineConfig({
  plugins: [react(), tailwindcss()],
  publicDir: false,
  base: "./",
  build: { outDir: "docs-embed-dist", emptyOutDir: true, rollupOptions: { input: "docs-embed.html" } },
});
