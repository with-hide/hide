// A Mermaid diagram in Markdown: a fenced block whose language is `mermaid`
// is drawn as its picture, in Markdown Live (`editor/markdownLivePlugin.ts`)
// and in the Markdown the phone and the PR card draw (`mobile/markdown.tsx`).
//
// Mermaid is large, so it is loaded the first time a diagram is drawn and
// never on a page without one. It renders one diagram at a time, because its
// render reads and writes module-wide state, and in the colours of the
// interface's tokens, read when the diagram is drawn; a theme change draws the
// diagrams on screen again (`watchPalette`).
//
// A body can come from anyone (a pull request, an agent's reply), so Mermaid
// runs at its `strict` security level and its SVG is cleaned again by
// DOMPurify before it reaches the page, without pictures or links, so a
// diagram neither runs nor loads anything.

import DOMPurify from "dompurify";

/** Mermaid's own default bound on a diagram's source; past it a block stays code. */
export const MERMAID_SOURCE_LIMIT = 50_000;

/** Drawn diagrams kept per page, so a list that remounts a message does not draw it again. */
const CACHE_LIMIT = 32;

export type MermaidDrawing = { kind: "svg"; svg: string } | { kind: "failed"; reason: string };

/** Whether a fence's info string names Mermaid (` ```mermaid `, ` ``` Mermaid title `). */
export function isMermaidInfo(info: string): boolean {
  return info.trim().split(/\s+/, 1)[0]?.toLowerCase() === "mermaid";
}

type Palette = {
  dark: boolean;
  background: string;
  foreground: string;
  secondary: string;
  muted: string;
  border: string;
  font: string;
};

function palette(): Palette {
  const root = document.documentElement;
  const style = getComputedStyle(root);
  const token = (name: string) => style.getPropertyValue(name).trim();
  return {
    dark: root.classList.contains("dark"),
    background: token("--background"),
    foreground: token("--foreground"),
    secondary: token("--secondary"),
    muted: token("--muted-foreground"),
    border: token("--border"),
    font: token("--font-sans"),
  };
}

type Mermaid = typeof import("mermaid").default;

let loading: Promise<Mermaid> | null = null;
let configured = "";
let queue: Promise<unknown> = Promise.resolve();
let sequence = 0;
const cache = new Map<string, MermaidDrawing>();

function load(): Promise<Mermaid> {
  loading ??= import("mermaid").then((module) => module.default);
  // A module that did not arrive (the phone went offline) is asked for again next time.
  loading.catch(() => {
    loading = null;
  });
  return loading;
}

function configure(mermaid: Mermaid, colors: Palette, key: string): void {
  if (configured === key) return;
  mermaid.initialize({
    startOnLoad: false,
    securityLevel: "strict",
    maxTextSize: MERMAID_SOURCE_LIMIT,
    suppressErrorRendering: true,
    // Mermaid puts an HTML label into the page to measure it before anything
    // can clean it, so a label's `<img>` would load; SVG text holds no markup.
    htmlLabels: false,
    theme: "base",
    darkMode: colors.dark,
    fontFamily: colors.font,
    themeVariables: {
      darkMode: colors.dark,
      background: colors.background,
      fontFamily: colors.font,
      primaryColor: colors.secondary,
      primaryTextColor: colors.foreground,
      primaryBorderColor: colors.muted,
      secondaryColor: colors.background,
      tertiaryColor: colors.background,
      lineColor: colors.muted,
      textColor: colors.foreground,
      clusterBkg: colors.background,
      clusterBorder: colors.border,
      edgeLabelBackground: colors.background,
    },
  });
  configured = key;
}

/**
 * Mermaid's SVG, with HTML labels inside `foreignObject`, cleaned of anything
 * that could run, load or navigate: Mermaid's strict level still lets a label
 * hold an `<img>` or a link.
 */
function clean(svg: string): string {
  return DOMPurify(window).sanitize(svg, {
    USE_PROFILES: { svg: true, svgFilters: true, html: true },
    ADD_TAGS: ["foreignObject"],
    FORBID_TAGS: ["img", "image", "a", "video", "audio", "picture", "source"],
    HTML_INTEGRATION_POINTS: { foreignobject: true },
  });
}

function remember(key: string, drawing: MermaidDrawing): MermaidDrawing {
  cache.delete(key);
  cache.set(key, drawing);
  for (const oldest of cache.keys()) {
    if (cache.size <= CACHE_LIMIT) break;
    cache.delete(oldest);
  }
  return drawing;
}

/**
 * Draw one diagram. A source Mermaid cannot read resolves to `failed` with
 * Mermaid's first line of explanation; the module failing to load does too,
 * and is not remembered, so the next draw tries again.
 */
export function drawMermaid(source: string): Promise<MermaidDrawing> {
  if (source.length > MERMAID_SOURCE_LIMIT) {
    return Promise.resolve({ kind: "failed", reason: `over ${MERMAID_SOURCE_LIMIT} characters` });
  }
  const colors = palette();
  const colorsKey = JSON.stringify(colors);
  const key = `${colorsKey}\n${source}`;
  const known = cache.get(key);
  if (known) return Promise.resolve(remember(key, known));
  const run = queue.then(async (): Promise<MermaidDrawing> => {
    let mermaid: Mermaid;
    try {
      mermaid = await load();
    } catch (error) {
      return { kind: "failed", reason: firstLine(error) };
    }
    configure(mermaid, colors, colorsKey);
    try {
      sequence += 1;
      const { svg } = await mermaid.render(`hide-mermaid-${sequence}`, source);
      return remember(key, { kind: "svg", svg: clean(svg) });
    } catch (error) {
      return remember(key, { kind: "failed", reason: firstLine(error) });
    }
  });
  queue = run;
  return run;
}

function firstLine(error: unknown): string {
  const text = error instanceof Error ? error.message : String(error);
  return text.split("\n", 1)[0]?.trim() || "unknown error";
}

/** Put a drawn diagram into `root`, replacing what it held. */
export function showMermaidSvg(root: HTMLElement, svg: string): void {
  root.innerHTML = svg;
  const element = root.querySelector("svg");
  // Mermaid sizes the picture to its natural width; it may shrink with the column but never grow past it.
  if (element) {
    element.removeAttribute("height");
    element.style.maxWidth = element.style.maxWidth || "100%";
    element.style.height = "auto";
  }
}

const listeners = new Set<() => void>();
let observer: MutationObserver | null = null;
let lastPalette = "";

/**
 * Call `listener` when the interface's colours change (light and dark, or the
 * colour theme), so a drawn diagram can be drawn again in the new ones. One
 * observer serves every diagram; it stops when the last listener leaves.
 */
export function watchPalette(listener: () => void): () => void {
  listeners.add(listener);
  if (!observer) {
    lastPalette = JSON.stringify(palette());
    observer = new MutationObserver(() => {
      const next = JSON.stringify(palette());
      if (next === lastPalette) return;
      lastPalette = next;
      for (const each of [...listeners]) each();
    });
    observer.observe(document.documentElement, { attributes: true });
  }
  return () => {
    listeners.delete(listener);
    if (listeners.size === 0) {
      observer?.disconnect();
      observer = null;
    }
  };
}
