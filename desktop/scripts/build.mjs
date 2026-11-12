// Bundles the main process and the preload into dist/, and copies the status
// page with the generated design tokens it is styled from. The tokens file is
// written for Tailwind: its `@theme` blocks, which a plain page drops, become
// `:root` blocks here, so the status page reads the same names the web shell
// does. The window's background is the dark theme's `--background` (the web
// shell's default theme) and the taskbar overlay's dot its `--warning`, read
// here so no color is written into the host.

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { build } from "esbuild";
import { copyTree } from "./copy-tree.mjs";
import { resolve } from "../../scripts/check-token-contrast.mjs";
import { readTokens } from "../../scripts/gen-tokens.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const dist = path.join(root, "dist");
const tokens = path.resolve(root, "..", "web", "src", "tokens.css");

const tokenCss = fs.readFileSync(tokens, "utf8");
// The two colors the host draws itself are read from design/tokens.json through
// the token resolver, never parsed out of the generated CSS, where a color may
// be a var() of a palette step.
const tokenValues = readTokens(path.resolve(root, ".."));
// The window's color before the page paints: Hide Dark, the default the page
// opens in (hide-color-theme D-13); the host cannot read the stored choice
// before it attaches to the core.
const background = resolve(tokenValues, "--background", "hide", "dark").toLowerCase();
// The Windows taskbar overlay's dot, in the color the shell's Needs You marks wear.
const warning = resolve(tokenValues, "--warning", "neutral", "dark").toLowerCase();
const themeBlocks = /@theme(?:\s+(?:static|inline))?\s*\{/g;
if (!themeBlocks.test(tokenCss)) throw new Error(`${tokens} has no @theme block; the status page expects Tailwind v4 tokens`);

fs.rmSync(dist, { recursive: true, force: true });
const common = {
  bundle: true,
  platform: "node",
  format: "cjs",
  target: "node22",
  external: ["electron"],
  sourcemap: "linked",
  logLevel: "warning",
};
await build({ ...common, entryPoints: [path.join(root, "src/main/index.ts")], outfile: path.join(dist, "main.js"), define: { __HIDE_BACKGROUND__: JSON.stringify(background), __HIDE_WARNING__: JSON.stringify(warning) } });
await build({ ...common, entryPoints: [path.join(root, "src/preload/index.ts")], outfile: path.join(dist, "preload.js") });

copyTree(path.join(root, "static"), path.join(dist, "static"));
// JSON is embedded in the local script: file: needs neither fetch nor a relaxed CSP.
await build({ entryPoints: [path.join(root, "static/status.js")], outfile: path.join(dist, "static/status.js"), bundle: true, platform: "browser", format: "iife", target: "chrome144", minify: true, legalComments: "eof" });
fs.rmSync(path.join(dist, "static/owl-motion.js"));
// The startup owl is the site's canonical pose, copied rather than kept twice.
for (const file of ["owl-blink.svg", "owl-blink-still.svg"]) fs.copyFileSync(path.join(root, "../site/public/assets/owls", file), path.join(dist, "static", file));
fs.writeFileSync(path.join(dist, "static", "tokens.css"), tokenCss.replace(themeBlocks, ":root {"));
