// The logos every surface draws, by adapter id: the first-run grid, Settings,
// Hide AI and the AgentMark on each pane, tab and row. Every file is listed in
// `assets/agents/manifest.json` with its source and licence note, and an
// agent with none draws a monogram (`scripts/check-agent-logos.mjs`).
import { agentAdapter } from "./agentAdapters";
import claudeMark from "./assets/agent-claude.png";
import codexMark from "./assets/agent-codex.png";
import manifest from "./assets/agents/manifest.json";

// A vendor's mark is bundled in the format it publishes: SVG, or PNG where no SVG exists.
const bundled = import.meta.glob<string>("./assets/agents/*.{svg,png}", { eager: true, query: "?url", import: "default" });

const LOGOS: Record<string, string> = { "claude-code": claudeMark, claude: claudeMark, codex: codexMark };
for (const [path, url] of Object.entries(bundled)) {
  const id = /\/([^/]+)\.(?:svg|png)$/.exec(path)?.[1];
  if (id) LOGOS[id] = url;
}

// A mark drawn dark on a transparent ground disappears on the dark theme, and a vendor's mark is never recoloured (docs/BRAND.md), so the manifest names the ones that sit on the plate.
const PLATED = new Set(manifest.logos.filter((logo) => logo.plate).map((logo) => logo.id));

/** The bundled logo of an agent, or null when the tile draws a monogram. */
export function agentLogo(id: string): string | null {
  return LOGOS[agentAdapter(id)?.logo_id ?? id] ?? null;
}

/**
 * The agent a Herdr kind names, with its logo, for the mark on a pane, tab or
 * row; `logo` is null for an agent with no bundled mark, which draws the
 * monogram of `label` (design 10). Null for a plain shell and for a kind Hide
 * does not support, which draw the neutral mark: a retained history logo
 * does not make its name an active agent.
 */
export function agentMarkOf(kind: string | null | undefined): { id: string; logo: string | null; label: string; plated: boolean } | null {
  const adapter = agentAdapter(kind ?? "");
  if (!adapter) return null;
  const logo = LOGOS[adapter.logo_id] ?? null;
  return { id: adapter.herdr_kind, logo, label: adapter.label, plated: logo !== null && PLATED.has(adapter.logo_id) };
}

/** Up to two letters of a name: the monogram of an agent with no logo. */
export function monogram(label: string): string {
  const words = label.split(/[\s-]+/).filter(Boolean);
  const letters = words.length > 1 ? words.slice(0, 2).map((word) => word[0]) : [...(words[0] ?? "?")].slice(0, 2);
  return letters.join("").toUpperCase();
}
