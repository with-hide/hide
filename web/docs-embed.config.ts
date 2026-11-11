import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

const relayStub = fileURLToPath(new URL("./src/docs/relayStub.ts", import.meta.url));

// The parts the user docs frame, built apart from the shell so the daemon never serves them and the dev-only
// gallery stays dev-only. Output: `docs-embed-dist/`, inside this worktree and ignored.
// The relay protocol's WASM package is built only by cargo, and the phone part never opens a relay, so its
// imports resolve to `src/docs/relayStub.ts`, which fails loudly if anything calls it.
export default defineConfig({
  plugins: [react(), tailwindcss()],
  publicDir: false,
  base: "./",
  resolve: { alias: [{ find: /^.*hide-relay-protocol\/(web|pkg\/relay_bg\.wasm\?url)$/, replacement: relayStub }] },
  build: { outDir: "docs-embed-dist", emptyOutDir: true, rollupOptions: { input: "docs-embed.html" } },
});
