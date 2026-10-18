#!/usr/bin/env node
// Regenerates the introduction site's product shots (site/assets/shots/) from
// the gallery's `landing` scene: the shell's own components over the invented
// story of the page's one-night timeline, never a capture of anyone's app.
// Each shot is photographed in Dark and Light at 2x (the header's close-up at
// 4x) and written as a lossy WebP, so the images can be remade whenever the UI
// changes:
//
//   node scripts/capture-site-shots.mjs [hero|agents|memory|attention|core ...]
//
// It starts this checkout's web dev server and a headless Chromium, both
// closed on every exit. Encoding needs `cwebp` (libwebp: `brew install webp`,
// `apt install webp`), because Chromium's screenshots are PNG or JPEG only.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {createRequire} from 'node:module';
import {execFileSync} from 'node:child_process';
import {pathToFileURL} from 'node:url';

const PIXEL_RATIO = 2;
/** The hero's tabs show one window each at this size, so switching tabs never moves the page. */
const WINDOW = {width: 1456, height: 900};
const THEMES = ['dark', 'light'];
const WEBP_QUALITY = '90';

/**
 * One shot: the window it is photographed in, in CSS pixels, what to do once the scene is up, and what to do with the
 * scene after it is photographed (`placed`, once, in the first theme).
 */
const SHOTS = {
  hero: {...WINDOW, ready: waitForTerminals, placed: placeOwlPeek},
  agents: {...WINDOW, ready: openLeadTree},
  memory: {...WINDOW, ready: waitForTerminals},
  core: WINDOW,
  attention: {width: 428, height: 740},
};

const root = fs.realpathSync(execFileSync('git', ['rev-parse', '--show-toplevel'], {encoding: 'utf8'}).trim());
const out = path.join(root, 'site', 'assets', 'shots');
const owls = path.join(root, 'site', 'assets', 'owls');

/** The hero writes its terminals once each has fitted its pane; the shot waits for that and the frame that draws it. */
async function waitForTerminals(page) {
  await page.locator('[data-landing-terminals="written"]').waitFor();
  await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
}

/**
 * The hero owl hides behind edges of the hero shot (site/README.md): it pops up over the window's top edge above the
 * lead's pane and above the PRs tool, leans out from behind the PRs tool's left edge, and rises from behind the
 * sidebar's footer. Its field is 1200 units wide, the window's width, with the window's top edge 200 units down; this
 * measures those edges on the scene just photographed and writes them into the loop and its still frame.
 */
async function placeOwlPeek(page) {
  const box = (selector) => page.locator(selector).first().evaluate((element) => element.getBoundingClientRect().toJSON());
  const [viewport, rail, sidebar, footer, lead, asking, lane] = await Promise.all([
    page.evaluate(() => ({width: innerWidth, height: innerHeight})),
    box('[data-device-rail]'), box('[data-sidebar-box]'), box('[data-sidebar-footer]'),
    box('[data-pane-view="a1"]'), box('[data-pane-view="a3"]'), box('[data-lane="prs"]'),
  ]);
  const unit = 1200 / viewport.width;
  const x = (px) => px * unit;
  const y = (px) => 200 + px * unit;
  const n = (value) => value.toFixed(2);
  const rect = (id, left, top, width, height) => `<clipPath id="${id}"><rect x="${n(left)}" y="${n(top)}" width="${n(width)}" height="${n(height)}"/>`;
  const bottom = y(viewport.height);
  // Each stop: where the owl stands, and the region it shows in (`show-`) and grips the edge in (`over-`, 24 units past it).
  const stops = {
    lead: {at: [x(lead.left + lead.width / 2), 200]},
    lane: {
      at: [x(lane.left) - 7.99, y(asking.top + asking.height * 0.6)],
      show: [x(sidebar.right), 200, x(lane.left) - x(sidebar.right), bottom - 200],
      over: [x(sidebar.right), 200, x(lane.left) - x(sidebar.right) + 24, bottom - 200],
    },
    sidebar: {
      at: [x((rail.right + sidebar.right) / 2), y(footer.top)],
      show: [x(rail.right), 200, x(sidebar.right) - x(rail.right), y(footer.top) - 200],
      over: [x(rail.right), 200, x(sidebar.right) - x(rail.right), y(footer.top) - 200 + 24],
    },
    prs: {at: [x(lane.left + lane.width / 2), 200]},
  };
  // The still frame draws only the first stop; it keeps every stop's clip regions with the loop's.
  for (const [file, drawn] of [['owl-peek.svg', Object.keys(stops)], ['owl-peek-still.svg', ['lead']]]) {
    const target = path.join(owls, file);
    let svg = fs.readFileSync(target, 'utf8');
    const field = Number(svg.match(/viewBox="0 0 1200 ([0-9.]+)"/)?.[1]);
    if (Math.abs(field - bottom) > 1) throw new Error(`${file} is 1200x${field} but the hero shot's field is 1200x${n(bottom)}; redraw its field for the new shot`);
    for (const [stop, {at, show, over}] of Object.entries(stops)) {
      // A stop's parts all stand at its anchor: the first one after its group names it, and every part moves with it.
      const anchor = svg.match(new RegExp(`(?:class="out-${stop}"|clip-path="url\\(#(?:show|over)-${stop}\\)")><g transform="translate\\(([-0-9.]+ [-0-9.]+)\\) rotate\\(`))?.[1];
      if (anchor) svg = svg.replaceAll(`translate(${anchor}) rotate(`, `translate(${n(at[0])} ${n(at[1])}) rotate(`);
      else if (drawn.includes(stop)) throw new Error(`${file} has no ${stop} stop to place`);
      if (show) svg = svg.replace(new RegExp(`<clipPath id="show-${stop}"><rect[^>]*/>`), rect(`show-${stop}`, ...show));
      if (over) svg = svg.replace(new RegExp(`<clipPath id="over-${stop}"><rect[^>]*/>`), rect(`over-${stop}`, ...over));
    }
    fs.writeFileSync(target, svg);
    console.log(`${path.relative(root, target)}  placed on the hero's edges`);
  }
}

/** The agents shot also opens the lead's child row onto the tree of who it started, as a click on it does. */
async function openLeadTree(page) {
  await waitForTerminals(page);
  await page.locator('[data-pane-tree]').first().click();
  await page.locator('[role=dialog], [data-radix-popper-content-wrapper]').first().waitFor();
}

function encode(png, webp) {
  try {
    execFileSync('cwebp', ['-quiet', '-q', WEBP_QUALITY, '-m', '6', '-sharp_yuv', png, '-o', webp]);
  } catch (error) {
    if (error.code === 'ENOENT') throw new Error('cwebp is not installed; install libwebp (brew install webp, apt install webp) and run again');
    throw error;
  }
}

async function main(names) {
  for (const name of names) if (!(name in SHOTS)) throw new Error(`Unknown shot ${name}; the shots are ${Object.keys(SHOTS).join(', ')}`);
  const web = path.join(root, 'web');
  if (!fs.existsSync(path.join(web, 'node_modules'))) throw new Error('web dependencies are not installed; run: bash scripts/verify-web.sh install');
  const require = createRequire(path.join(web, 'package.json'));
  const {createServer} = await import(pathToFileURL(require.resolve('vite')).href);
  const {chromium} = require('@playwright/test');

  const cleanup = [];
  const close = async () => { for (const step of cleanup.reverse()) await step(); };
  for (const signal of ['SIGINT', 'SIGTERM']) process.once(signal, () => close().finally(() => process.exit(130)));
  const scratch = fs.mkdtempSync(path.join(os.tmpdir(), 'hide-site-shots-'));
  cleanup.push(() => fs.rmSync(scratch, {recursive: true, force: true}));
  try {
    const server = await createServer({root: web, configFile: path.join(web, 'vite.config.ts'), logLevel: 'error', server: {host: '127.0.0.1', port: 0, strictPort: false, hmr: false}});
    cleanup.push(() => server.close());
    await server.listen();
    const origin = server.resolvedUrls?.local?.[0]?.replace(/\/$/, '');
    if (!origin) throw new Error('The web dev server did not report an address');
    // Headless Chromium draws xterm's WebGL canvas blank at 2x, so WebGL is off and the
    // hero's terminals use xterm's DOM renderer, the one the app falls back to (terminalRenderer.ts).
    const browser = await chromium.launch({args: ['--disable-3d-apis']});
    cleanup.push(() => browser.close());

    fs.mkdirSync(out, {recursive: true});
    for (const name of names) {
      const {width, height, ready, placed} = SHOTS[name];
      for (const theme of THEMES) {
        const context = await browser.newContext({viewport: {width, height}, deviceScaleFactor: PIXEL_RATIO, reducedMotion: 'reduce'});
        const page = await context.newPage();
        const problems = [];
        page.on('pageerror', (error) => problems.push(error.message));
        await page.goto(`${origin}/gallery?${new URLSearchParams({scene: 'landing', shot: name, theme})}`);
        await page.locator(`[data-landing-shot="${name}"]`).waitFor();
        await page.addStyleTag({content: '*, *::before, *::after { transition: none !important; animation: none !important; caret-color: transparent !important; }'});
        await page.evaluate(() => document.fonts.ready);
        await page.waitForFunction(() => [...document.images].every((image) => image.complete && image.naturalWidth > 0));
        if (ready) await ready(page);
        await page.mouse.move(0, 0);
        await page.evaluate(() => document.activeElement?.blur());
        // The page's story is English; a Korean string means a fixture row escaped its rewording.
        const text = await page.evaluate(() => document.body.innerText);
        const korean = text.match(/[ㄱ-힝]+/);
        if (korean) throw new Error(`${name} (${theme}) shows Korean text "${korean[0]}"; reword that row in web/src/gallery/landingSceneData.ts`);
        if (problems.length) throw new Error(`${name} (${theme}) raised a page error: ${problems[0]}`);
        const png = path.join(scratch, `${name}-${theme}.png`);
        await page.screenshot({path: png});
        const webp = path.join(out, `${name}-${theme}.webp`);
        encode(png, webp);
        console.log(`${path.relative(root, webp)}  ${width * PIXEL_RATIO}x${height * PIXEL_RATIO}  ${(fs.statSync(webp).size / 1024).toFixed(0)} KB`);
        if (placed && theme === THEMES[0]) await placed(page);
        await context.close();
      }
    }
  } finally {
    await close();
  }
}

main(process.argv.slice(2).length ? process.argv.slice(2) : Object.keys(SHOTS)).catch((error) => {
  console.error(error.message);
  process.exitCode = 1;
});
