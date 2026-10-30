// What the snapshot frames a page receives say about a pane's in-process subagents, for the specs that move
// a pane's counts through an agent's hook (OpenCode's plugin, Pi's and omp's extension). The hook reports them
// to the core, so the snapshot is where they are observed; Herdr holds none of them.

import type { Page } from "@playwright/test";

type Counts = { working?: number | null; done?: number | null };

/** The counts object anywhere in `node` that is `pane`'s row with its children, as the snapshot frames carry it. */
function paneCounts(node: unknown, pane: string): Counts | undefined {
  if (Array.isArray(node)) {
    for (const item of node) {
      const found = paneCounts(item, pane);
      if (found !== undefined) return found;
    }
    return undefined;
  }
  if (!node || typeof node !== "object") return undefined;
  const row = node as { id?: unknown; children?: { subagents?: Counts } | null };
  if (row.id === pane && row.children?.subagents) return row.children.subagents;
  for (const value of Object.values(node)) {
    const found = paneCounts(value, pane);
    if (found !== undefined) return found;
  }
  return undefined;
}

/** `pane`'s subagent counts, as the last snapshot frame the page received that names it says. */
export function snapshotCounts(page: Page, pane: string): () => { working: number | null; done: number | null } | undefined {
  let last: { working: number | null; done: number | null } | undefined;
  page.on("websocket", (ws) => ws.on("framereceived", (frame) => {
    const text = String(frame.payload);
    if (!text.includes('"subagents"') || !text.includes(pane)) return;
    try {
      const found = paneCounts(JSON.parse(text), pane);
      if (found !== undefined) last = { working: found.working ?? null, done: found.done ?? null };
    } catch {
      /* not a JSON frame */
    }
  }));
  return () => last;
}

/** The in-process subagents working in `pane`, as the last snapshot frame the page received that names it says. */
export function snapshotWorking(page: Page, pane: string): () => number | null | undefined {
  const counts = snapshotCounts(page, pane);
  return () => counts()?.working;
}
