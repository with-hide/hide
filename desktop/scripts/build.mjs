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

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const dist = path.join(root, "dist");
const tokens = path.resolve(root, "..", "web", "src", "tokens.css");

const tokenCss = fs.readFileSync(tokens, "utf8");
// The window's color before the page paints: Hide Dark, the default the page
// opens in (hide-color-theme D-13); the host cannot read the stored choice
// before it attaches to the core.
const background = /\.palette-hide \.dark\s*\{[^}]*?--background:\s*(#[0-9a-fA-F]{3,8})\s*;/.exec(tokenCss)?.[1];
if (!background) throw new Error(`${tokens} has no Hide dark --background`);
// The Windows taskbar overlay's dot, in the color the shell's Needs You marks wear.
const warning = /\.dark\s*\{[^}]*?--warning:\s*(#[0-9a-fA-F]{6})\s*;/.exec(tokenCss)?.[1];
if (!warning) throw new Error(`${tokens} has no dark --warning`);
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

fs.cpSync(path.join(root, "static"), path.join(dist, "static"), { recursive: true });
// JSON is embedded in the local script: file: needs neither fetch nor a relaxed CSP.
await build({ entryPoints: [path.join(root, "static/status.js")], outfile: path.join(dist, "static/status.js"), bundle: true, platform: "browser", format: "iife", target: "chrome144", minify: true, legalComments: "eof" });
fs.rmSync(path.join(dist, "static/owl-motion.js"));
fs.rmSync(path.join(dist, "static/owl-roam.json"));
fs.writeFileSync(path.join(dist, "static", "tokens.css"), tokenCss.replace(themeBlocks, ":root {"));
