// What the Overview, the sidebar and the Factory share about a project's
// checkouts and the work on them, as pure functions over the snapshot: a
// checkout's Git stage, its agent rows, a pull request's chip, and the
// layered dependency layout the Factory graph draws. Every value drawn is one
// the snapshot carries (design 10).

import { scopeOccurrences } from "./agentScope";
import { formatDateTime } from "./i18n/format";
import { requireInterfaceLanguage } from "./i18n/locale";
import { settled } from "./prMark";
import type { AgentRow, Checkout, PrState, PullRequest, Workspace } from "./snapshot";

/** A checkout's stage: Git decides it, so a checkout is never in the backlog. */
type GitStage = "working" | "review" | "done";

/** A checkout's Git stage: its work landed or its pull request merged, then an open pull request, else work in progress. Agents and the issue's state never move it. */
export function stageOf(checkout: Checkout): GitStage {
  const pr = checkout.pull_request;
  if (checkout.landed === true || pr?.badge === "merged") return "done";
  if (pr && pr.badge !== "closed") return "review";
  return "working";
}

export type BoardRow = {
  agent: AgentRow;
  /** 0 for a root, one more per delegation step. */
  depth: number;
};

/**
 * The result a card delivers: its pull request in the one PR state the core
 * decided, and whether a reviewer asked for changes, the one review word a
 * chip keeps beside its mark (the mark's colour says the rest).
 */
export type PrChip = {
  number: number;
  url: string;
  state: PrState;
  changesRequested: boolean;
};

export function prChip(pr: PullRequest): PrChip {
  return { number: pr.number, url: pr.url, state: pr.state, changesRequested: !settled(pr.state) && pr.review === "changes_requested" };
}

/** A Project the board reads: its catalog row, every agent its device reported, and the device's name when it is not this machine. */
export type BoardProject = { workspace: Workspace; agents: AgentRow[]; device: string | null };

/**
 * Each checkout's agent rows by checkout id, the rows the sidebar draws under
 * an opened checkout and a task card lists. `agents` is every agent of the
 * project's device. A checkout's rows are its agents, each drawn with its
 * whole lineage from the first ancestor that is not also working here, root
 * first, whatever is folded; the Projects sidebar drops a folded row's
 * descendants itself (`unfoldedRows`).
 */
export function checkoutAgentRows(workspace: Workspace, agents: AgentRow[], context: "device" | "global" | "visible" | "sidebar" = "device"): Map<string, BoardRow[]> {
  return new Map(workspace.checkouts.map((checkout) => {
    const tree = context === "sidebar" ? checkout.agent_scope.sidebar_tree : context === "device" ? checkout.agent_scope.tree : checkout.agent_scope.global_tree;
    // The sidebar draws its opened roots' children and grandchildren too (PRD agent-hierarchy-screens D-12).
    const rows = context === "visible" || context === "sidebar" ? tree.visible_rows : tree.rows;
    return [checkout.id, scopeOccurrences(rows, agents).map((agent, index) => ({ agent, depth: rows[index]!.depth }))];
  }));
}

/** One arrow of a dependency graph: from the blocker to the node it blocks. */
export type DependencyEdge = { from: string; to: string };

/** Nodes laid out by what they wait on: the layers left to right, the arrows between them, and the nodes with no relation. */
export type LayeredGraph<T> = { layers: T[][]; edges: DependencyEdge[]; unrelated: T[] };

/**
 * The layered dependency layout the Factory graph draws:
 * a node's column is the longest chain of blockers before it, and each
 * column is ordered by the mean row of the blockers it hangs from, so arrows
 * mostly run straight. A cycle, which a source should not allow, drops the
 * arrow that closes it rather than looping.
 */
export function layerDependencies<T>(nodes: readonly T[], idOf: (node: T) => string, blockersOf: (node: T) => readonly T[]): LayeredGraph<T> {
  const blockers = new Map<string, readonly T[]>();
  const edges: DependencyEdge[] = [];
  const related = new Set<string>();
  for (const value of nodes) {
    const before = blockersOf(value);
    blockers.set(idOf(value), before);
    for (const from of before) {
      edges.push({ from: idOf(from), to: idOf(value) });
      related.add(idOf(from));
      related.add(idOf(value));
    }
  }
  const depth = new Map<string, number>();
  const visiting = new Set<string>();
  const depthOf = (value: T): number => {
    const id = idOf(value);
    const known = depth.get(id);
    if (known !== undefined) return known;
    if (visiting.has(id)) return -1;
    visiting.add(id);
    const found = Math.max(-1, ...(blockers.get(id) ?? []).map(depthOf)) + 1;
    visiting.delete(id);
    depth.set(id, found);
    return found;
  };
  const layers: T[][] = [];
  for (const value of nodes) {
    if (!related.has(idOf(value))) continue;
    const layer = depthOf(value);
    (layers[layer] ??= []).push(value);
  }
  const row = new Map<string, number>();
  const dense = layers.filter((layer) => layer !== undefined);
  for (const layer of dense) {
    const weight = (value: T) => {
      const rows = (blockers.get(idOf(value)) ?? []).flatMap((from) => (row.has(idOf(from)) ? [row.get(idOf(from)) as number] : []));
      return rows.length > 0 ? rows.reduce((sum, at) => sum + at, 0) / rows.length : Number.POSITIVE_INFINITY;
    };
    const ordered = layer.map((value, index) => ({ value, index, weight: weight(value) })).sort((a, b) => a.weight - b.weight || a.index - b.index);
    layer.splice(0, layer.length, ...ordered.map(({ value }) => value));
    layer.forEach((value, index) => row.set(idOf(value), index));
  }
  const forward = edges.filter((edge) => (depth.get(edge.from) ?? 0) < (depth.get(edge.to) ?? 0));
  return { layers: dense, edges: forward, unrelated: nodes.filter((value) => !related.has(idOf(value))) };
}

/**
 * A list read top down by what each item waits on (PRD mission-plan-visibility
 * D-07, B3, B17): an item's depth is the longest chain of items before it,
 * the list is ordered by depth and then as given, and an item that shares its
 * depth with an earlier one is `parallel` to it. A cycle, which a source
 * should not allow, is cut where it closes, as `layerDependencies` cuts it.
 */
export function orderByDependencies<T>(items: readonly T[], idOf: (item: T) => string, dependsOn: (item: T) => readonly string[]): { item: T; depth: number; parallel: boolean }[] {
  const byId = new Map(items.map((item) => [idOf(item), item]));
  const depth = new Map<string, number>();
  const visiting = new Set<string>();
  const depthOf = (item: T): number => {
    const id = idOf(item);
    const known = depth.get(id);
    if (known !== undefined) return known;
    if (visiting.has(id)) return -1;
    visiting.add(id);
    const found = Math.max(-1, ...dependsOn(item).flatMap((key) => (byId.has(key) ? [depthOf(byId.get(key)!)] : []))) + 1;
    visiting.delete(id);
    depth.set(id, found);
    return found;
  };
  const ordered = items.map((item, index) => ({ item, index, depth: depthOf(item) })).sort((a, b) => a.depth - b.depth || a.index - b.index);
  return ordered.map(({ item, depth: at }, position) => ({ item, depth: at, parallel: position > 0 && ordered[position - 1]!.depth === at }));
}

/**
 * The arrows left once every arrow implied by a longer path is dropped
 * (PRD software-factory-ui D-08): with A→B→C, A→C is not drawn. The data
 * keeps every edge; this is a drawing step, used by the Factory graph only.
 * An edge on a cycle is kept, since no other path stands for it.
 */
export function transitiveReduction(edges: readonly DependencyEdge[]): DependencyEdge[] {
  const next = new Map<string, string[]>();
  for (const edge of edges) next.set(edge.from, [...(next.get(edge.from) ?? []), edge.to]);
  // Whether `to` is reachable from `from` by a path of two or more arrows.
  const longer = (from: string, to: string) => {
    const seen = new Set<string>();
    const stack = (next.get(from) ?? []).filter((step) => step !== to);
    while (stack.length > 0) {
      const at = stack.pop()!;
      if (at === to) return true;
      if (seen.has(at) || at === from) continue;
      seen.add(at);
      stack.push(...(next.get(at) ?? []));
    }
    return false;
  };
  const unique = edges.filter((edge, index) => edges.findIndex((other) => other.from === edge.from && other.to === edge.to) === index);
  return unique.filter((edge) => !longer(edge.from, edge.to));
}

/** A day the way an issue states it, by this machine's calendar: `Sep 27`, `9월 27일`. */
export function issueDate(unixMs: number, language: string): string {
  return formatDateTime(requireInterfaceLanguage(language), unixMs, { month: "short", day: "numeric" });
}
