// Proves each docs part holds one still picture under reduced motion: it loads every part id in the built
// `docs-embed-dist`, takes a picture, lets 120 seconds of page time pass (Playwright's fake clock fires every
// timer due in that span), takes another, and fails unless the two are byte-identical. A control run of the
// task-row without reduced motion must change, so a check that cannot see motion cannot pass.
//
//   pnpm --dir web build:docs-embed && node web/src/docs/check-still.mjs
//
// It needs the browser the web e2e lane installs (`bash scripts/verify-web.sh web playwright-install`); without it
// the script fails and says so. `scripts/docs/check-all.mjs` runs it after the embed build, and the docs lane installs it first.
/* global console, URL */
import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { createServer } from "node:http";
import { createRequire } from "node:module";
import { extname, join, normalize } from "node:path";
import { fileURLToPath } from "node:url";
import { NOW } from "./now.ts";
import { PART_STATES, PARTS } from "./parts.ts";

const web = fileURLToPath(new URL("../../", import.meta.url));
const dist = join(web, "docs-embed-dist");
if (!existsSync(join(dist, "docs-embed.html"))) throw new Error(`${dist} is missing: run \`pnpm --dir web build:docs-embed\` first`);
const { chromium } = createRequire(join(web, "package.json"))("@playwright/test");

const TYPES = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".svg": "image/svg+xml", ".wasm": "application/wasm", ".json": "application/json" };
const server = createServer((request, response) => {
  const path = normalize(decodeURIComponent(new URL(request.url, "http://x").pathname)).replace(/^(\.\.[/\\])+/, "");
  const file = join(dist, path === "/" ? "docs-embed.html" : path);
  if (!file.startsWith(dist) || !existsSync(file)) return response.writeHead(404).end();
  response.writeHead(200, { "content-type": TYPES[extname(file)] ?? "application/octet-stream" }).end(readFileSync(file));
});
await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
const base = `http://127.0.0.1:${server.address().port}/docs-embed.html`;

const SPAN_MS = 120_000;
const digest = (bytes) => createHash("sha256").update(bytes).digest("hex").slice(0, 12);

async function twoPictures(browser, part, theme, reducedMotion) {
  const context = await browser.newContext({ viewport: { width: 800, height: 900 }, reducedMotion: reducedMotion ? "reduce" : "no-preference" });
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  page.on("console", (message) => message.type() === "error" && errors.push(message.text()));
  // The page's clock starts at the instant the fixtures are written for, so an elapsed time that did tick would show.
  await page.clock.install({ time: NOW });
  await page.goto(`${base}?part=${part}&theme=${theme}&lang=en`);
  await page.waitForSelector("[data-docs-part]");
  await page.clock.runFor(1500);
  const first = await page.screenshot({ fullPage: true });
  await page.clock.runFor(SPAN_MS);
  const second = await page.screenshot({ fullPage: true });
  await context.close();
  return { first: digest(first), second: digest(second), errors };
}

// A missing browser is a failure with its cure, never a skipped check (the docs lane installs it before check-all).
const browser = await chromium.launch().catch((error) => {
  throw new Error(`check-still needs Playwright's Chromium and could not start it: run \`bash scripts/verify-web.sh web playwright-install\` (${error.message.split("\n")[0]})`);
});
const failures = [];
try {
  for (const part of PARTS) {
    for (const theme of ["dark", "light"]) {
      const { first, second, errors } = await twoPictures(browser, part, theme, true);
      const still = first === second && errors.length === 0;
      console.log(`${still ? "ok  " : "FAIL"} ${part.padEnd(14)} ${theme.padEnd(5)} ${first} ${second}${errors.length ? ` errors: ${errors.join(" | ")}` : ""}`);
      if (!still) failures.push(`${part} ${theme}`);
    }
  }
  const control = await twoPictures(browser, "task-row", "dark", false);
  console.log(`${control.first !== control.second ? "ok  " : "FAIL"} control: task-row with motion changes ${control.first} ${control.second}`);
  if (control.first === control.second) failures.push("control: the animated task-row did not change, so this check sees no motion");
  if (PARTS.length !== Object.keys(PART_STATES).length) failures.push("part list drifted");
} finally {
  await browser.close();
  server.close();
}
if (failures.length > 0) throw new Error(`Not still: ${failures.join("; ")}`);
console.log(`all ${PARTS.length} parts are byte-identical ${SPAN_MS / 1000} s apart under reduced motion`);
