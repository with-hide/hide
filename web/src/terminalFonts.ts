// The fonts a terminal draws in: the operator's Terminal font, then the
// `--font-mono` token, then Nerd Font icons from the bundled Symbols Nerd
// Font Mono, fitted to one cell.
//
// xterm.js draws a glyph a fallback font supplies at that font's own size,
// from the left edge of its cells, and has no option to scale it: a 1 em icon
// runs into the next cell. So the icon font is a `FontFace` under a family of
// its own whose `size-adjust` makes its glyphs fill one cell, measured at run
// time against the cell xterm measures from the primary font (the WebGL
// renderer's rounding, `cellBox`), and capped so a glyph never rises above
// or sinks below its row. The cell depends on the primary font, its size and
// the device pixel ratio, so one fitted face serves each of those (a `Plan`),
// shared by every terminal drawn at it.
//
// Korean, Japanese and Chinese text is not fitted: the browser draws it from
// the system's fallback at the terminal's own size, as it always has, since
// enlarging it to fill two cells crowded the rows.
//
// The icon face only takes the Private Use Areas and comes after the primary
// font and its generic family, so a font the operator chose that has its own
// icons keeps them.

import symbolsFontUrl from "./assets/fonts/SymbolsNerdFontMono-Regular.woff2?url";
import { useShellStore } from "./store";

/** The Private Use Areas, where Nerd Fonts puts its icons; the bundled font takes nothing else. */
export const SYMBOLS_RANGE = "U+E000-F8FF, U+F0000-FFFFD";

/** Icons from the sets a prompt uses (Font Awesome, Devicons, Material Design); the font is monospaced, so every icon has their advance. */
const SYMBOL_SAMPLES = "\u{f04fe}\u{f0668}";

/** A family name as CSS reads it: quoted, so a name with a comma or a quote is one family. */
export function cssFamily(name: string): string {
  return `"${name.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"`;
}

/** The family list that decides the terminal's cell: the operator's font when there is one, then the `--font-mono` token, which ends in a generic family every system has. */
export function primaryFamily(chosen: string, token: string): string {
  const name = chosen.trim();
  return name ? `${cssFamily(name)}, ${token}` : token;
}

/** What the primary font measures at the terminal's size, as xterm measures it: the advance of `W` and the font's ascent and descent, in CSS pixels. */
export type PrimaryMetrics = { width: number; ascent: number; descent: number };

/** A cell as the WebGL renderer lays it out, in CSS pixels: its width, and the room above and below the baseline it draws a glyph on. */
export type CellBox = { width: number; above: number; below: number };

/**
 * The cell xterm's WebGL renderer builds from `metrics` (`WebglRenderer`
 * `_updateDimensions`): a device-pixel width rounded down, a height rounded
 * up and multiplied by the line height, the character box centered in it,
 * and the glyph drawn on the font's ideographic baseline at the box's bottom,
 * which is its descent below the alphabetic one.
 */
export function cellBox(metrics: PrimaryMetrics, dpr: number, lineHeight = 1): CellBox {
  const charWidth = Math.floor(metrics.width * dpr);
  const charHeight = Math.ceil((metrics.ascent + metrics.descent) * dpr);
  const cellHeight = Math.floor(charHeight * lineHeight);
  const top = lineHeight === 1 ? 0 : Math.round((cellHeight - charHeight) / 2);
  const baseline = top + charHeight - metrics.descent * dpr;
  return { width: charWidth / dpr, above: baseline / dpr, below: (cellHeight - baseline) / dpr };
}

/** A fallback font's glyphs per em of its size: the widest advance and the highest and lowest ink among its samples. */
export type GlyphEm = { advance: number; ascent: number; descent: number };

/**
 * The `size-adjust` that fits `glyph` into `cells` cells of `cell` at
 * `fontSize`: as wide as the cells, and never higher than the room above the
 * baseline or deeper than the room below it, so a tall glyph is not cut off
 * at the top of its row (Ghostty 1.2.1's fix for the same). Rounded down, so
 * rounding never makes a glyph overflow.
 */
export function fitScale(cell: CellBox, glyph: GlyphEm, fontSize: number, cells: number): number {
  const limits = [(cells * cell.width) / (glyph.advance * fontSize)];
  if (glyph.ascent > 0) limits.push(cell.above / (glyph.ascent * fontSize));
  if (glyph.descent > 0) limits.push(cell.below / (glyph.descent * fontSize));
  return Math.floor(Math.min(...limits) * 1000) / 1000;
}

const PROBE_SIZE = 100;

let probeCanvas: CanvasRenderingContext2D | null = null;

function canvas(): CanvasRenderingContext2D {
  if (probeCanvas) return probeCanvas;
  const context = document.createElement("canvas").getContext("2d");
  if (!context) throw new Error("terminal fonts need a 2d canvas");
  probeCanvas = context;
  return context;
}

/** The primary font at `size` as xterm's `TextMetricsMeasureStrategy` measures it. */
function measurePrimary(size: number, family: string): PrimaryMetrics {
  const context = canvas();
  context.font = `${size}px ${family}`;
  const metrics = context.measureText("W");
  return { width: metrics.width, ascent: metrics.fontBoundingBoxAscent, descent: metrics.fontBoundingBoxDescent };
}

let probes = 0;

/** Loads `source` under a family of its own, measures `samples` in it and takes it out again; null when it does not load. */
async function measureFace(source: ArrayBuffer, samples: string): Promise<GlyphEm | null> {
  probes += 1;
  const family = `Hide Font Probe ${probes}`;
  const face = new FontFace(family, source);
  try {
    await face.load();
  } catch {
    return null;
  }
  document.fonts.add(face);
  try {
    const context = canvas();
    context.font = `${PROBE_SIZE}px ${cssFamily(family)}`;
    let advance = 0;
    let ascent = 0;
    let descent = 0;
    for (const sample of samples) {
      const metrics = context.measureText(sample);
      advance = Math.max(advance, metrics.width);
      ascent = Math.max(ascent, metrics.actualBoundingBoxAscent);
      descent = Math.max(descent, metrics.actualBoundingBoxDescent);
    }
    if (advance <= 0) return null;
    return { advance: advance / PROBE_SIZE, ascent: ascent / PROBE_SIZE, descent: descent / PROBE_SIZE };
  } finally {
    document.fonts.delete(face);
  }
}

let symbolsData: Promise<ArrayBuffer> | null = null;

/** The bundled icon font, fetched once and shared by every fitted face made from it. */
function symbolsFont(): Promise<ArrayBuffer> {
  symbolsData ??= fetch(symbolsFontUrl).then((response) => {
    if (!response.ok) throw new Error(`symbols font answered ${response.status}`);
    return response.arrayBuffer();
  });
  return symbolsData;
}

/** The icon font's measure: it does not change while the page lives. */
let symbolsGlyph: Promise<GlyphEm | null> | null = null;

function symbolsEm(): Promise<GlyphEm | null> {
  symbolsGlyph ??= symbolsFont().then((data) => measureFace(data, SYMBOL_SAMPLES));
  return symbolsGlyph;
}

/** One fitted icon face: a family of its own at one `size-adjust`, and how many plans list it. */
type FittedFamily = { scale: number; family: string; face: Promise<FontFace>; plans: number };

const fitted = new Map<number, FittedFamily>();
let families = 0;

async function loadFace(family: string, scale: number): Promise<FontFace> {
  // `size-adjust` is in every engine Hide runs in, not yet in TypeScript's DOM types.
  const descriptors: FontFaceDescriptors & { sizeAdjust: string } = { unicodeRange: SYMBOLS_RANGE, sizeAdjust: `${(scale * 100).toFixed(1)}%` };
  const face = new FontFace(family, await symbolsFont(), descriptors);
  await face.load();
  document.fonts.add(face);
  return face;
}

/** The fitted icon family at `scale`, shared by every plan that fits it the same. */
function fittedFamily(scale: number): FittedFamily {
  const known = fitted.get(scale);
  if (known) {
    known.plans += 1;
    return known;
  }
  families += 1;
  const family = `Hide Terminal Fallback ${families}`;
  const entry: FittedFamily = { scale, family, face: loadFace(family, scale), plans: 1 };
  fitted.set(scale, entry);
  return entry;
}

function releaseFamily(entry: FittedFamily) {
  entry.plans -= 1;
  if (entry.plans > 0) return;
  fitted.delete(entry.scale);
  void entry.face.then(
    (face) => document.fonts.delete(face),
    () => {},
  );
}

/**
 * The fitted icon font for one primary font, size and pixel ratio. `family`
 * is the primary family alone until the icon face is fitted and loaded, then
 * with it; the terminals drawn at it hold it (`users`), and an unheld plan
 * stays for a later terminal until `IDLE_PLAN_LIMIT` newer ones push it out,
 * which removes the face only it listed.
 */
type Plan = { key: string; family: string; ready: Promise<void>; users: number; families: FittedFamily[] };

const plans = new Map<string, Plan>();
/** Unheld plans kept for reuse, beyond which the oldest is released: a few sizes and pixel ratios an operator moves between. */
const IDLE_PLAN_LIMIT = 8;

async function fitPlan(plan: Plan, size: number, primary: string, dpr: number) {
  // A local font is drawn the moment it is named, but a name the browser
  // has to load first must be measured once it has.
  await document.fonts.load(`${size}px ${primary}`);
  const cell = cellBox(measurePrimary(size, primary), dpr);
  const glyph = await symbolsEm();
  if (!glyph) return;
  const entry = fittedFamily(fitScale(cell, glyph, size, 1));
  plan.families.push(entry);
  await entry.face;
  plan.family = `${primary}, ${cssFamily(entry.family)}`;
}

function trimIdlePlans() {
  const idle = [...plans.values()].filter((plan) => plan.users === 0);
  for (const plan of idle.slice(0, Math.max(0, idle.length - IDLE_PLAN_LIMIT))) {
    plans.delete(plan.key);
    // A plan still fitting releases its families when it finishes.
    void plan.ready.then(() => plan.families.forEach(releaseFamily));
  }
}

/** A terminal's hold on the fonts for its size: the family to draw in now, and when a better one is ready. */
export type TerminalFont = {
  /** The primary family alone until the icon face is fitted, then with it. */
  readonly family: () => string;
  /** Settles once `family` lists the fitted icon face (or it failed and it never will). */
  readonly ready: Promise<void>;
  /** Lets the plan go; a later terminal at the same size may still take it. */
  release(): void;
};

/** The operator's Terminal font; read when a plan is made. */
let chosenFamily = "";

/** Sets the operator's Terminal font; true when it changed, so the caller re-fonts its terminals. */
export function setTerminalFontFamily(chosen: string): boolean {
  if (chosenFamily === chosen) return false;
  chosenFamily = chosen;
  return true;
}

/** The font a terminal at `size` CSS pixels draws in, given the `--font-mono` token's value. */
export function holdTerminalFont(size: number, token: string): TerminalFont {
  const primary = primaryFamily(chosenFamily, token);
  const dpr = window.devicePixelRatio || 1;
  const key = JSON.stringify([size, dpr, primary]);
  let plan = plans.get(key);
  if (!plan) {
    const made: Plan = { key, family: primary, ready: Promise.resolve(), users: 0, families: [] };
    made.ready = fitPlan(made, size, primary, dpr).catch((error: unknown) => {
      // The terminal keeps drawing in the primary family, as it did before
      // the icon face existed; why is the diagnostic log's.
      useShellStore.getState().noteDiagnostic(`terminal.fonts fit_failed size=${size} dpr=${dpr} reason=${error instanceof Error ? error.message : String(error)}`);
    });
    plan = made;
  }
  // Most recently used last, so the idle trim takes the oldest.
  plans.delete(key);
  plans.set(key, plan);
  plan.users += 1;
  const held = plan;
  let released = false;
  return {
    family: () => held.family,
    ready: held.ready,
    release() {
      if (released) return;
      released = true;
      held.users -= 1;
      trimIdlePlans();
    },
  };
}

/**
 * Calls `onChange` when the device pixel ratio changes (another display, the
 * browser's zoom): the cell is measured in device pixels, so the fit moves
 * with it. One query is watched at a time.
 */
export function watchPixelRatio(onChange: () => void): () => void {
  let query: MediaQueryList | null = null;
  const listen = () => {
    query = window.matchMedia(`(resolution: ${window.devicePixelRatio}dppx)`);
    query.addEventListener("change", changed, { once: true });
  };
  const changed = () => {
    listen();
    onChange();
  };
  listen();
  return () => query?.removeEventListener("change", changed);
}

/**
 * Whether `name` is a font this machine draws: text set in it measures
 * differently from each of three generic families it would otherwise fall
 * back to. Read for the Terminal font field, not per frame.
 */
export function fontInstalled(name: string): boolean {
  const context = canvas();
  const text = "mmmmmmmmmmlli10WQ@#";
  return ["monospace", "serif", "sans-serif"].some((generic) => {
    context.font = `${PROBE_SIZE}px ${generic}`;
    const fallback = context.measureText(text).width;
    context.font = `${PROBE_SIZE}px ${cssFamily(name)}, ${generic}`;
    return context.measureText(text).width !== fallback;
  });
}
