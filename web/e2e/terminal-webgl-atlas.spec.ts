// The WebGL addon's glyph atlas after it merges pages, drawn in real software
// WebGL with the addon the shell bundles: the installed package rewritten by
// the same function the build applies (src/webglAtlasSlots.ts).
//
// This spec starts no Herdr or hided: the fault is inside the renderer, between
// the atlas and the GPU, and terminal bytes take the same path whether a PTY or
// `term.write` produced them. What it checks is what the GPU draws from: at
// every glyph draw, the canvas last uploaded to each texture slot must be the atlas
// page now at that slot's index. Without the fix, a later merge leaves a slot
// holding an earlier merged page (both carry version 1), and every glyph on
// that page is drawn from the wrong picture.
import { expect, test } from "@playwright/test";
import fs from "node:fs";
import { createRequire } from "node:module";
import { fixWebglAtlasSlots } from "../src/webglAtlasSlots";

// WebGL is the subject; the suite's --disable-webgl would test the DOM renderer.
test.use({ launchOptions: { args: ["--use-gl=angle", "--use-angle=swiftshader", "--enable-unsafe-swiftshader"] } });

const resolve = createRequire(import.meta.url).resolve;
const ORIGIN = "http://atlas.test";
const FILES: Record<string, () => string> = {
  "/xterm.mjs": () => fs.readFileSync(resolve("@xterm/xterm/lib/xterm.mjs"), "utf8"),
  "/addon-webgl.mjs": () => fixWebglAtlasSlots(fs.readFileSync(resolve("@xterm/addon-webgl/lib/addon-webgl.mjs"), "utf8")),
  "/xterm.css": () => fs.readFileSync(resolve("@xterm/xterm/css/xterm.css"), "utf8"),
  "/": () => `<!doctype html><meta charset="utf-8"><link rel="stylesheet" href="/xterm.css"><body style="margin:0"></body>`,
};

type Drawn = { rounds: number; merges: number; misplaced: number[] };

declare global {
  interface Window {
    __atlas: (merges: number, rounds: number) => Promise<Drawn>;
  }
}

// Runs in the page: one terminal given a new screen of distinct Hangul in 216
// colors a frame, about 2,500 new glyphs each time.
async function installAtlasPage() {
  const { Terminal } = await import(`${location.origin}/xterm.mjs`);
  const { WebglAddon } = await import(`${location.origin}/addon-webgl.mjs`);
  const proto = WebGL2RenderingContext.prototype;
  // The atlas keeps one page per texture unit and merges four when it runs
  // out. Apple GPUs report 16 units and the software renderer 32; reporting 8
  // brings the merges after fewer glyphs, at the same point on every runner.
  const getParameter = proto.getParameter;
  proto.getParameter = function (this: WebGL2RenderingContext, name: number) {
    return name === this.MAX_TEXTURE_IMAGE_UNITS ? 8 : getParameter.call(this, name);
  };
  let removed = 0;
  const element = document.createElement("div");
  element.style.cssText = "width:1700px;height:900px";
  document.body.appendChild(element);
  const term = new Terminal({ fontFamily: "monospace", fontSize: 13, cols: 200, rows: 50, scrollback: 0 });
  term.open(element);
  const addon = new WebglAddon();
  addon.onRemoveTextureAtlasCanvas(() => (removed += 1));
  term.loadAddon(addon);
  // xterm's renderer has no public view of its atlas pages or texture slots.
  const renderer = term._core._renderService._renderer.value;

  // The canvas each texture slot was last given, and at each glyph draw the
  // slots that hold another page than the atlas has at their index.
  const uploaded = new Map<number, unknown>();
  const misplaced: number[] = [];
  let unit = 0;
  const activeTexture = proto.activeTexture;
  proto.activeTexture = function (this: WebGL2RenderingContext, texture: number) {
    unit = texture - this.TEXTURE0;
    activeTexture.call(this, texture);
  };
  const methods = proto as unknown as Record<"texImage2D" | "drawElementsInstanced", (...args: unknown[]) => void>;
  const texImage2D = methods.texImage2D;
  methods.texImage2D = function (this: WebGL2RenderingContext, ...args: unknown[]) {
    const source = args[args.length - 1];
    if (source instanceof HTMLCanvasElement) uploaded.set(unit, source);
    texImage2D.apply(this, args);
  };
  const drawElementsInstanced = methods.drawElementsInstanced;
  methods.drawElementsInstanced = function (this: WebGL2RenderingContext, ...args: unknown[]) {
    if (this.getParameter(this.CURRENT_PROGRAM) === renderer._glyphRenderer.value._program) {
      renderer._charAtlas.pages.forEach((page: { canvas: HTMLCanvasElement }, slot: number) => {
        if (uploaded.get(slot) !== page.canvas) misplaced.push(slot);
      });
    }
    drawElementsInstanced.apply(this, args);
  };

  let seed = 1;
  const random = () => {
    seed = (seed + 0x6d2b79f5) | 0;
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
  // One screen: 50 rows of 100 two-cell syllables, each in its own color.
  const screen = () =>
    Array.from({ length: 50 }, () =>
      Array.from({ length: 100 }, () => `\x1b[38;5;${16 + Math.floor(random() * 216)}m${String.fromCodePoint(0xac00 + Math.floor(random() * 11172))}`).join(""),
    ).join("\x1b[0m\r\n");
  const frame = () => new Promise((done) => requestAnimationFrame(done));

  // A merge removes four pages.
  window.__atlas = async (merges, rounds) => {
    let round = 0;
    while (round < rounds && removed < merges * 4 && !misplaced.length) {
      await new Promise<void>((done) => term.write(`\x1b[H${screen()}`, done));
      await frame();
      round += 1;
    }
    return { rounds: round, merges: removed / 4, misplaced };
  };
}

test("every texture slot draws the atlas page at its index after the atlas merges pages", async ({ page }) => {
  await page.route(`${ORIGIN}/**`, (route) => {
    const path = new URL(route.request().url()).pathname;
    const body = FILES[path];
    const contentType = path.endsWith(".mjs") ? "text/javascript" : path.endsWith(".css") ? "text/css" : "text/html";
    return body ? route.fulfill({ body: body(), contentType }) : route.abort();
  });
  await page.goto(`${ORIGIN}/`);
  await page.evaluate(installAtlasPage);
  // Without the fix a glyph draw read another page right after the second
  // merge (macOS, round 5); drawing to the fourth passes it on a system whose
  // fonts fill the pages at another pace, and a run that never got there
  // proves nothing.
  const drawn = await page.evaluate(() => window.__atlas(4, 16));
  expect(drawn.misplaced, `slots holding another page at round ${drawn.rounds}`).toEqual([]);
  expect(drawn.merges).toBeGreaterThanOrEqual(4);
});
