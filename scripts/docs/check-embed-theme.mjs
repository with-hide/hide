#!/usr/bin/env node
// Proves that every live part a docs page frames ends on the page's own theme, by loading the built site many times
// and reading back the document each frame actually loaded. It exists because the fault is a race (a frame whose
// first navigation finishes after its `src` was changed), which one load does not show: on a build without the
// repair in site/src/embed-script.mjs a few percent of loads leave a frame on the wrong theme; with it, none do.
//
//   bash scripts/verify-web.sh web build:docs-embed && bash scripts/verify-web.sh site build   # a built site: site/dist
//   node scripts/docs/check-embed-theme.mjs site/dist [--loads 30] [--jobs 3]
//
// Not part of check-all: 9 pages x 30 loads x 2 colour schemes is 540 fresh browser contexts, minutes of work. Run it
// when `site/src/embed-script.mjs` or how a page frames a part changes, once on the build of main and once on the
// change; the first line printed says whether the build carries the repair, and the exit status is 1 when any frame
// of any load is wrong. A race shows as a count, so zero on main proves nothing and non-zero on main is the defect.
//
// What it does, per page and colour scheme, `--loads` times:
//   1. a new browser context (a cold cache: a cached frame would never race), viewport 1440x900, the colour scheme;
//   2. the page loads; every frame `load` is counted from a script that runs before the page's own;
//   3. the frames are `loading="lazy"`, so each framed part is scrolled into view in turn and waited for until it has
//      loaded once (a state, not a delay);
//   4. a frame is right when its loaded document's own URL says the page's theme, the part has drawn, and the
//      document's root carries that theme's class; the wait for that is bounded (WRONG_AFTER_MS): a frame that is
//      still not right that long after it was in view, with nothing loading, is wrong. The repair navigates once after
//      the first load, so a repaired frame is right well inside the bound; the bound is only paid by a wrong frame.
// The pages are every `index.html` under `<dist>/docs` that holds an iframe whose `src` contains `/embed/`.
/* global console, document, process */
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative, resolve } from "node:path";
import { launchChromium, serveDirectory } from "./browser.mjs";

const args = process.argv.slice(2);
const option = (name, fallback) => (args.includes(name) ? Number(args[args.indexOf(name) + 1]) : fallback);
const dist = resolve(args.find((arg, i) => !arg.startsWith("--") && !args[i - 1]?.startsWith("--")) ?? "");
const LOADS = option("--loads", 30);
const JOBS = option("--jobs", 3);
const WRONG_AFTER_MS = 5000;
if (!existsSync(join(dist, "embed/docs-embed.html")) || !existsSync(join(dist, "docs"))) {
  throw new Error(`usage: check-embed-theme.mjs <built site folder> [--loads N] [--jobs N]; ${dist} is not a built site (needs docs/ and embed/docs-embed.html: build the live parts, then the site)`);
}

function pagesWithFrames(folder) {
  return readdirSync(folder).flatMap((name) => {
    const path = join(folder, name);
    if (statSync(path).isDirectory()) return pagesWithFrames(path);
    return name === "index.html" && /<iframe[^>]*src="[^"]*\/embed\//.test(readFileSync(path, "utf8")) ? [`/${relative(dist, folder)}/`] : [];
  });
}
const pages = pagesWithFrames(join(dist, "docs")).sort();
const repair = pages.some((page) => readFileSync(join(dist, page, "index.html"), "utf8").includes("location.replace"));
console.log(`build: ${dist}\nthe head script ${repair ? "carries" : "does NOT carry"} the load repair; ${pages.length} pages with frames; ${LOADS} loads x 2 colour schemes each`);

const { base, close } = await serveDirectory(dist, { cleanUrls: true });
const browser = await launchChromium("check-embed-theme");

/** Counts each frame's loads from before any page script runs (the capture phase hears a load that does not bubble). */
const countLoads = () => document.addEventListener("load", (event) => { if (event.target.tagName === "IFRAME") event.target.__loads = (event.target.__loads ?? 0) + 1; }, true);

/** In the page: frame `index` is right for `theme`. */
const frameIsRight = ([index, theme]) => {
  const frame = document.querySelectorAll('iframe[src*="/embed/"]')[index];
  if (!frame || !frame.__loads) return false;
  const inner = frame.contentDocument;
  return new URL(frame.contentWindow.location.href).searchParams.get("theme") === theme && !!inner.querySelector("[data-docs-part]") && inner.documentElement.classList.contains(theme);
};

async function oneLoad(scheme, path) {
  const context = await browser.newContext({ viewport: { width: 1440, height: 900 }, colorScheme: scheme });
  try {
    const page = await context.newPage();
    await page.addInitScript(countLoads);
    await page.goto(`${base}${path}`, { waitUntil: "load" });
    const theme = await page.evaluate(() => document.documentElement.dataset.theme);
    const count = await page.evaluate(() => document.querySelectorAll('iframe[src*="/embed/"]').length);
    let wrong = 0;
    for (let index = 0; index < count; index++) {
      await page.evaluate((i) => document.querySelectorAll('iframe[src*="/embed/"]')[i].scrollIntoView(), index);
      const right = await page.waitForFunction(frameIsRight, [index, theme], { timeout: WRONG_AFTER_MS }).then(() => true, () => false);
      if (!right) wrong++;
    }
    return { frames: count, wrong, theme };
  } finally {
    await context.close();
  }
}

const queue = [];
for (const scheme of ["light", "dark"]) for (const path of pages) queue.push({ scheme, path, frames: 0, wrongFrames: 0, wrongLoads: 0 });
const total = { loads: 0, wrongLoads: 0, frames: 0, wrongFrames: 0 };
try {
  const work = [...queue];
  await Promise.all(Array.from({ length: JOBS }, async () => {
    for (let row = work.shift(); row; row = work.shift()) {
      for (let i = 0; i < LOADS; i++) {
        const { frames, wrong, theme } = await oneLoad(row.scheme, row.path);
        if (theme !== row.scheme) throw new Error(`${row.path} under a ${row.scheme} colour scheme has data-theme=${theme}: the page, not a frame, is on the wrong theme`);
        row.frames += frames;
        row.wrongFrames += wrong;
        row.wrongLoads += wrong ? 1 : 0;
        total.loads++;
        total.frames += frames;
        total.wrongFrames += wrong;
        total.wrongLoads += wrong ? 1 : 0;
      }
    }
  }));
} finally {
  await browser.close();
  await close();
}
for (const row of queue) console.log(`${row.scheme.padEnd(5)} ${row.path} loads=${LOADS} frames=${row.frames} wrongFrames=${row.wrongFrames} loadsWithWrong=${row.wrongLoads}`);
console.log(`TOTAL loads=${total.loads} loadsWithWrong=${total.wrongLoads} frames=${total.frames} wrongFrames=${total.wrongFrames}`);
if (total.wrongFrames > 0) process.exit(1);
