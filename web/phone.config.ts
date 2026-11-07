import { readFileSync } from "node:fs";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { PHONE_PROTOCOL } from "./src/mobile/phoneVersion";

// Public build contract: the only relay override, loopback allowed for isolated tests.
const relay = new URL(process.env.VITE_HIDE_RELAY_URL || "wss://relay.withhide.dev/ws");
if (relay.username || relay.password || relay.search || relay.hash ||
    (relay.protocol !== "wss:" && !(relay.protocol === "ws:" && ["127.0.0.1", "localhost", "[::1]"].includes(relay.hostname)))) {
  throw new Error("VITE_HIDE_RELAY_URL must be a credential-free WSS URL or loopback WS URL");
}
const csp = `default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self'; connect-src 'self' ${relay.origin}; worker-src 'self'; manifest-src 'self'; media-src 'self' blob:; frame-src 'self'; base-uri 'none'; form-action 'none'; object-src 'none'`;

export default defineConfig(({ mode }) => {
  const root = mode === "loader";
  return {
    plugins: [react(), tailwindcss(), {
      name: "phone-security",
      transformIndexHtml(html) {
        return html.replace("<head>", `<head><meta http-equiv="Content-Security-Policy" content="${csp}">`);
      },
      // The frame an HTML file is drawn in sits beside the bundle's page, which loads it by the relative address
      // `look-frame.html`, as the desktop's own build serves it beside `/m/`. It keeps its own content policy.
      generateBundle() {
        if (root) return;
        this.emitFile({ type: "asset", fileName: "look-frame.html", source: readFileSync(new URL("./public/m/look-frame.html", import.meta.url)) });
      },
    }],
    publicDir: false,
    base: root ? "/" : `/p/${PHONE_PROTOCOL}/`,
    build: {
      outDir: root ? "phone-dist" : `phone-dist/p/${PHONE_PROTOCOL}`,
      emptyOutDir: true,
      rollupOptions: { input: root ? "phone.html" : "phone-bundle.html" },
    },
  };
});
