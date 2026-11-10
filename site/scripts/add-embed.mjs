// Puts the live parts the docs frame (the web shell's own components, built by `web/docs-embed.config.ts`)
// into the built site at `/embed/`, so a docs page frames `/embed/docs-embed.html?part=<id>`.
// Run after `astro build`; it fails when the parts were not built, rather than ship pages whose pictures 404.
import { cpSync, existsSync, rmSync } from "node:fs";
import { fileURLToPath } from "node:url";

const parts = fileURLToPath(new URL("../../web/docs-embed-dist", import.meta.url));
const target = fileURLToPath(new URL("../dist/embed", import.meta.url));
if (!existsSync(`${parts}/docs-embed.html`)) {
  throw new Error(`${parts}/docs-embed.html is missing: run \`pnpm --dir web build:docs-embed\` before the site build`);
}
rmSync(target, { recursive: true, force: true });
cpSync(parts, target, { recursive: true });
console.log("embed: web/docs-embed-dist copied to dist/embed");
