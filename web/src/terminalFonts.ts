// The fonts a terminal draws in: the operator's Terminal font, then the
// `--font-mono` token, then fallbacks fitted to the terminal's cells: Nerd
// Font icons from the bundled Symbols Nerd Font Mono, one cell, and the
// system's own Korean, Japanese and Chinese fonts, two cells, in the order of
// the operator's languages.
//
// xterm.js draws a glyph a fallback font supplies at that font's own size,
// from the left edge of its cells, and has no option to scale it: a 1 em icon
// runs into the next cell, and a Hangul syllable 0.87 em wide leaves a gap in
// its two cells. So each fallback is a `FontFace` under a family of its own
// whose `size-adjust` makes its glyphs fill their cells, measured at run time
// against the cell xterm measures from the primary font (the WebGL
// renderer's rounding, `cellBox`), and capped so a glyph never rises above
// or sinks below its row. The cell depends on the primary font, its size and
// the device pixel ratio, so one fitted set serves each of those (a `Plan`),
// shared by every terminal drawn at it.
//
// The fallbacks only take the characters their `unicode-range` names, and
// come after the primary font and its generic family, so a font the operator
// chose that has its own icons or CJK glyphs keeps them.

import symbolsFontUrl from "./assets/fonts/SymbolsNerdFontMono-Regular.woff2?url";
import { hostPlatform } from "./host";
import { useShellStore } from "./store";

export type FontSystem = "mac" | "windows" | "linux";

/** Which system's fonts a platform name stands for: Node's `darwin`/`win32`, the browser's `MacIntel`, `Win32`, `macOS`, `Windows`, or a user agent; anything else is a Linux-like system. */
export function fontSystemOf(platform: string): FontSystem {
  if (/mac|darwin|iphone|ipad/i.test(platform)) return "mac";
  if (/^win|windows/i.test(platform)) return "windows";
  return "linux";
}

/** The writing systems whose Han shapes differ by region; Hangul is only in the Korean fonts and kana in the Japanese ones. */
export type CjkScript = "kr" | "ja" | "sc" | "tc";

type CjkFont = {
  script: CjkScript;
  /** Full or PostScript names `local()` takes for the regular face, tried in order. */
  regular: string[];
  /** The same for the bold face; when none is installed the browser emboldens the regular one. */
  bold: string[];
};

/** Each system's own CJK fonts; none is bundled, so a machine without one draws that script as it did before. */
export const CJK_FONTS: Record<FontSystem, CjkFont[]> = {
  mac: [
    { script: "kr", regular: ["Apple SD Gothic Neo", "AppleSDGothicNeo-Regular"], bold: ["Apple SD Gothic Neo Bold", "AppleSDGothicNeo-Bold"] },
    { script: "ja", regular: ["Hiragino Sans W3", "HiraginoSans-W3"], bold: ["Hiragino Sans W6", "HiraginoSans-W6"] },
    { script: "sc", regular: ["PingFang SC Regular", "PingFangSC-Regular"], bold: ["PingFang SC Semibold", "PingFangSC-Semibold"] },
    { script: "tc", regular: ["PingFang TC Regular", "PingFangTC-Regular"], bold: ["PingFang TC Semibold", "PingFangTC-Semibold"] },
  ],
  windows: [
    { script: "kr", regular: ["Malgun Gothic", "MalgunGothic", "MalgunGothicRegular"], bold: ["Malgun Gothic Bold", "MalgunGothicBold", "MalgunGothic-Bold"] },
    { script: "ja", regular: ["Yu Gothic Regular", "YuGothic-Regular", "Meiryo", "Meiryo-Regular"], bold: ["Yu Gothic Bold", "YuGothic-Bold", "Meiryo Bold", "Meiryo-Bold"] },
    { script: "sc", regular: ["Microsoft YaHei", "MicrosoftYaHei"], bold: ["Microsoft YaHei Bold", "MicrosoftYaHei-Bold"] },
    { script: "tc", regular: ["Microsoft JhengHei", "MicrosoftJhengHeiRegular"], bold: ["Microsoft JhengHei Bold", "MicrosoftJhengHeiBold"] },
  ],
  linux: [
    { script: "kr", regular: ["Noto Sans CJK KR", "Noto Sans CJK KR Regular", "NotoSansCJKkr-Regular"], bold: ["Noto Sans CJK KR Bold", "NotoSansCJKkr-Bold"] },
    { script: "ja", regular: ["Noto Sans CJK JP", "Noto Sans CJK JP Regular", "NotoSansCJKjp-Regular"], bold: ["Noto Sans CJK JP Bold", "NotoSansCJKjp-Bold"] },
    { script: "sc", regular: ["Noto Sans CJK SC", "Noto Sans CJK SC Regular", "NotoSansCJKsc-Regular"], bold: ["Noto Sans CJK SC Bold", "NotoSansCJKsc-Bold"] },
    { script: "tc", regular: ["Noto Sans CJK TC", "Noto Sans CJK TC Regular", "NotoSansCJKtc-Regular"], bold: ["Noto Sans CJK TC Bold", "NotoSansCJKtc-Bold"] },
  ],
};

/** Characters each script's font is measured on, ones every font of that script has (가한글뷁, あア漢字, 国汉字语, 國漢字語), by code point since they are not product text. */
const CJK_SAMPLES: Record<CjkScript, string> = {
  kr: String.fromCodePoint(0xac00, 0xd55c, 0xae00, 0xbdc1),
  ja: String.fromCodePoint(0x3042, 0x30a2, 0x6f22, 0x5b57),
  sc: String.fromCodePoint(0x56fd, 0x6c49, 0x5b57, 0x8bed),
  tc: String.fromCodePoint(0x570b, 0x6f22, 0x5b57, 0x8a9e),
};

/**
 * The script order for the operator's languages: the first CJK language
 * leads, Korean (`ko`), Japanese (`ja`), Traditional Chinese (`zh-TW`,
 * `zh-HK`, `zh-MO`, `zh-Hant…`) or Simplified (any other `zh`); with none,
 * Simplified Chinese, Japanese, Traditional, Korean. Only Han takes the first
 * font that has it; Hangul and kana each have one script that holds them.
 */
export function cjkScriptOrder(languages: readonly string[]): CjkScript[] {
  const order: CjkScript[] = ["sc", "ja", "tc", "kr"];
  for (const language of languages) {
    const tag = language.toLowerCase();
    const script: CjkScript | null =
      tag === "ko" || tag.startsWith("ko-") ? "kr"
      : tag === "ja" || tag.startsWith("ja-") ? "ja"
      : tag === "zh" || tag.startsWith("zh-") ? (/^zh-(tw|hk|mo|hant)\b/.test(tag) ? "tc" : "sc")
      : null;
    if (script) return [script, ...order.filter((other) => other !== script)];
  }
  return order;
}

/** Hangul, kana, Han and the wide CJK punctuation and forms: the characters a terminal gives two cells and a CJK font draws. */
export const CJK_RANGE = [
  "U+1100-115F", // Hangul leading jamo
  "U+2E80-2FDF", // CJK and Kangxi radicals
  "U+3000-303F", // CJK symbols and punctuation
  "U+3040-30FF", // Hiragana, Katakana
  "U+3100-312F", // Bopomofo
  "U+3130-318F", // Hangul compatibility jamo
  "U+3190-31FF", // Kanbun, Bopomofo extended, CJK strokes, Katakana extension
  "U+3200-33FF", // Enclosed CJK letters, CJK compatibility
  "U+3400-4DBF", // Han extension A
  "U+4E00-9FFF", // Han
  "U+A960-A97F", // Hangul jamo extended A
  "U+AC00-D7AF", // Hangul syllables
  "U+F900-FAFF", // Han compatibility
  "U+FE30-FE4F", // CJK compatibility forms
  "U+FF01-FF60", // Fullwidth forms
  "U+FFE0-FFE6", // Fullwidth signs
  "U+20000-3134F", // Han extensions B to G
].join(", ");

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
 * baseline or deeper than the room below it, so a large CJK font is not
 * cut off at the top of its row (Ghostty 1.2.1's fix for the same). Rounded
 * down, so rounding never makes a glyph overflow.
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

/** Loads `sources` under a family of its own, measures `samples` in it and takes it out again; null when no source loads. */
async function measureFace(source: string | ArrayBuffer, samples: string): Promise<GlyphEm | null> {
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

function localSource(names: string[]): string {
  return names.map((name) => `local(${cssFamily(name)})`).join(", ");
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

/** A fallback font's measure, per font: it does not change while the page lives. */
const glyphs = new Map<string, Promise<GlyphEm | null>>();

function glyphEm(key: string, measure: () => Promise<GlyphEm | null>): Promise<GlyphEm | null> {
  let known = glyphs.get(key);
  if (!known) {
    known = measure();
    glyphs.set(key, known);
  }
  return known;
}

type Fallback = {
  key: string;
  cells: number;
  range: string;
  glyph: () => Promise<GlyphEm | null>;
  faces: () => Promise<{ source: string | ArrayBuffer; weight: string }[]>;
};

function fallbacks(system: FontSystem, languages: readonly string[]): Fallback[] {
  const symbols: Fallback = {
    key: "symbols",
    cells: 1,
    range: SYMBOLS_RANGE,
    glyph: () => glyphEm("symbols", async () => measureFace(await symbolsFont(), SYMBOL_SAMPLES)),
    faces: async () => [{ source: await symbolsFont(), weight: "normal" }],
  };
  const order = cjkScriptOrder(languages);
  const cjk = [...CJK_FONTS[system]].sort((a, b) => order.indexOf(a.script) - order.indexOf(b.script));
  return [
    symbols,
    ...cjk.map((font): Fallback => ({
      key: font.regular[0] ?? font.script,
      cells: 2,
      range: CJK_RANGE,
      glyph: () => glyphEm(font.regular.join(), () => measureFace(localSource(font.regular), CJK_SAMPLES[font.script])),
      faces: async () => [
        { source: localSource(font.regular), weight: "normal" },
        { source: localSource(font.bold), weight: "bold" },
      ],
    })),
  ];
}

/** One fitted fallback: a family of its own, its regular and (when installed) bold face at one `size-adjust`, and how many plans list it. */
type FittedFamily = { key: string; family: string; faces: Promise<FontFace[]>; plans: number };

const fitted = new Map<string, FittedFamily>();
let families = 0;

async function loadFaces(fallback: Fallback, family: string, scale: number): Promise<FontFace[]> {
  const loaded: FontFace[] = [];
  for (const { source, weight } of await fallback.faces()) {
    // `size-adjust` is in every engine Hide runs in, not yet in TypeScript's DOM types.
    const descriptors: FontFaceDescriptors & { sizeAdjust: string } = { weight, unicodeRange: fallback.range, sizeAdjust: `${(scale * 100).toFixed(1)}%` };
    const face = new FontFace(family, source, descriptors);
    try {
      await face.load();
    } catch {
      // Not installed here: the browser emboldens the regular face, or the
      // script falls through to the next font as it did before.
      continue;
    }
    document.fonts.add(face);
    loaded.push(face);
  }
  return loaded;
}

/** The fitted family for `fallback` at `scale`, shared by every plan that fits it the same. */
function fittedFamily(fallback: Fallback, scale: number): FittedFamily {
  const key = `${fallback.key}|${scale}`;
  const known = fitted.get(key);
  if (known) {
    known.plans += 1;
    return known;
  }
  families += 1;
  const family = `Hide Terminal Fallback ${families}`;
  const entry: FittedFamily = { key, family, faces: loadFaces(fallback, family, scale), plans: 1 };
  fitted.set(key, entry);
  return entry;
}

function releaseFamily(entry: FittedFamily) {
  entry.plans -= 1;
  if (entry.plans > 0) return;
  fitted.delete(entry.key);
  void entry.faces.then(
    (faces) => faces.forEach((face) => document.fonts.delete(face)),
    () => {},
  );
}

/**
 * The fitted fonts for one primary font, size and pixel ratio. `family` is
 * the primary family alone until the fallbacks are fitted and loaded, then
 * the whole list; the terminals drawn at it hold it (`users`), and an unheld
 * plan stays for a later terminal until `IDLE_PLAN_LIMIT` newer ones push it
 * out, which removes the faces only it listed.
 */
type Plan = { key: string; family: string; ready: Promise<void>; users: number; families: FittedFamily[] };

const plans = new Map<string, Plan>();
/** Unheld plans kept for reuse, beyond which the oldest is released: a few sizes and pixel ratios an operator moves between. */
const IDLE_PLAN_LIMIT = 8;

async function fitPlan(plan: Plan, size: number, primary: string, dpr: number, system: FontSystem, languages: readonly string[]) {
  // A local font is drawn the moment it is named, but a name the browser
  // has to load first must be measured once it has.
  await document.fonts.load(`${size}px ${primary}`);
  const cell = cellBox(measurePrimary(size, primary), dpr);
  const names: string[] = [];
  for (const fallback of fallbacks(system, languages)) {
    const glyph = await fallback.glyph();
    if (!glyph) continue;
    const entry = fittedFamily(fallback, fitScale(cell, glyph, size, fallback.cells));
    plan.families.push(entry);
    if ((await entry.faces).length > 0) names.push(cssFamily(entry.family));
  }
  plan.family = names.length > 0 ? `${primary}, ${names.join(", ")}` : primary;
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
  /** The primary family alone until the fallbacks are fitted, then with them. */
  readonly family: () => string;
  /** Settles once `family` lists the fitted fallbacks (or they failed and it never will). */
  readonly ready: Promise<void>;
  /** Lets the plan go; a later terminal at the same size may still take it. */
  release(): void;
};

/** The operator's Terminal font and the system and languages the fallbacks follow; the token is read when a plan is made. */
type FontChoice = { chosen: string; system: FontSystem; languages: readonly string[] };

let choice: FontChoice | null = null;

function currentChoice(): FontChoice {
  choice ??= {
    chosen: "",
    system: fontSystemOf(hostPlatform()),
    languages: typeof navigator === "undefined" ? [] : navigator.languages?.length ? navigator.languages : [navigator.language],
  };
  return choice;
}

/** Sets the operator's Terminal font; true when it changed, so the caller re-fonts its terminals. */
export function setTerminalFontFamily(chosen: string): boolean {
  const current = currentChoice();
  if (current.chosen === chosen) return false;
  choice = { ...current, chosen };
  return true;
}

/** The font a terminal at `size` CSS pixels draws in, given the `--font-mono` token's value. */
export function holdTerminalFont(size: number, token: string): TerminalFont {
  const { chosen, system, languages } = currentChoice();
  const primary = primaryFamily(chosen, token);
  const dpr = window.devicePixelRatio || 1;
  const key = JSON.stringify([size, dpr, primary]);
  let plan = plans.get(key);
  if (!plan) {
    const made: Plan = { key, family: primary, ready: Promise.resolve(), users: 0, families: [] };
    made.ready = fitPlan(made, size, primary, dpr, system, languages).catch((error: unknown) => {
      // The terminal keeps drawing in the primary family, as it did before
      // the fallbacks existed; why is the diagnostic log's.
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
