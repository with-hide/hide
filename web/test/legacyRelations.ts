// Frozen pre-refactor lineage selection from main 9f144877. Test fixtures only.
// What is connected to a thing, drawn the way a Project's Overview groups it
// (PRD cmdk-navigation D-07, D-08): the issues on top, then one group per
// checkout whose head is the checkout and whose rows are its pull request and
// the agent's lineage. A parent that works in another checkout is one parent
// line under the agent; a child delegated to another checkout stands in that
// checkout's own group. The facts are the ones the snapshot already carries -
// `checkout.task_key`, `closes_task_keys`, `checkout.pull_request` and the
// agent lineage fields - and a connection the snapshot does not name has no
// row, never an empty line (design principle 10).

import type { TFunction } from "i18next";
import { frontDeviceId } from "../src/devices";
import {
  agentEntry,
  checkoutEntry,
  issueEntry,
  pullRequestEntry,
  searchDevices,
  type SearchDevice,
  type SearchEntry,
} from "../src/search";
import { checkoutPlaces } from "../src/navigation";
import type { AgentRow, Checkout, SnapshotRest, Workspace } from "../src/snapshot";

/** One checkout's group: its head and the rows under it, in the order they are drawn. */
export type RelationGroup = { head: SearchEntry; rows: SearchEntry[] };

export type Relations = { issues: SearchEntry[]; groups: RelationGroup[] };

/** What the relations are of: an agent pane, a checkout, a pull request of a project, or an issue. */
export type RelationTarget =
  | { kind: "agent"; paneId: string }
  | { kind: "checkout"; checkoutId: string }
  | { kind: "pr"; workspaceId: string; number: number }
  | { kind: "issue"; taskKey: string };

/** The checkout a pane sits in, with its project, in one device's catalog. */
function checkoutOfPane(scope: SearchDevice, paneId: string): { workspace: Workspace; checkout: Checkout } | null {
  for (const workspace of scope.workspaces) {
    for (const checkout of workspace.checkouts) {
      if (checkout.tabs.some((tab) => tab.panes.some((pane) => pane.id === paneId))) return { workspace, checkout };
    }
  }
  return null;
}

function paneSet(checkout: Checkout): Set<string> {
  const ids = new Set<string>();
  for (const tab of checkout.tabs) for (const pane of tab.panes) ids.add(pane.id);
  return ids;
}

/** The issues a checkout works on or its pull request closes, as the project's own task rows. */
function checkoutIssues(scope: SearchDevice, workspace: Workspace, checkout: Checkout, front: string, t: TFunction<"translation">): SearchEntry[] {
  const keys = [checkout.task_key, ...(checkout.closes_task_keys ?? [])].filter((key): key is string => Boolean(key));
  const tasks = workspace.tasks?.tasks ?? [];
  const issues: SearchEntry[] = [];
  for (const key of new Set(keys)) {
    const task = tasks.find((row) => row.key === key);
    if (task) issues.push(issueEntry(scope, workspace, task, front, t));
  }
  return issues;
}

type Lineage = { byPane: Map<string, AgentRow>; here: Set<string> };

/** The agents of `agent`'s lineage that sit in this checkout, as rows: ancestors in it, the agent, its descendants in it. */
function lineageRows(scope: SearchDevice, lineage: Lineage, agent: AgentRow, places: Map<string, string>, front: string, tagHere: boolean, t: TFunction<"translation">): SearchEntry[] {
  const chain: AgentRow[] = [];
  let top = agent;
  const seen = new Set<string>([agent.pane_id]);
  for (;;) {
    const parentId = top.lineage_parent_pane_id;
    const parent = parentId ? lineage.byPane.get(parentId) : undefined;
    if (!parent || seen.has(parent.pane_id) || !lineage.here.has(parent.pane_id)) break;
    seen.add(parent.pane_id);
    chain.unshift(parent);
    top = parent;
  }
  const rows: SearchEntry[] = [];
  const row = (row: AgentRow, depth: number, tag?: SearchEntry["tag"]): SearchEntry => ({ ...agentEntry(scope, row, places.get(row.pane_id) ?? null, front, t), depth, ...(tag ? { tag } : {}) });
  chain.forEach((ancestor, depth) => rows.push(row(ancestor, depth)));
  const anchorDepth = chain.length;
  rows.push(row(agent, anchorDepth, tagHere ? "here" : undefined));
  // The parent in another checkout is one line under the agent, named with its branch and state.
  const outside = (top.lineage_parent_pane_id ? lineage.byPane.get(top.lineage_parent_pane_id) : undefined);
  if (outside && !lineage.here.has(outside.pane_id)) {
    const place = places.get(outside.pane_id) ?? null;
    rows.push({ ...agentEntry(scope, outside, place, front, t), depth: anchorDepth + 1, tag: "parent" });
  }
  const walk = (parent: AgentRow, depth: number) => {
    for (const childId of parent.lineage_child_pane_ids ?? []) {
      const child = lineage.byPane.get(childId);
      if (!child || seen.has(child.pane_id) || !lineage.here.has(child.pane_id)) continue;
      seen.add(child.pane_id);
      rows.push(row(child, depth));
      walk(child, depth + 1);
    }
  };
  walk(agent, anchorDepth + 1);
  return rows;
}

/** Descendants of `agent`, anywhere in the device's agents, in lineage order. */
function descendantsOf(agent: AgentRow, byPane: Map<string, AgentRow>): AgentRow[] {
  const found: AgentRow[] = [];
  const seen = new Set<string>([agent.pane_id]);
  const walk = (parent: AgentRow) => {
    for (const childId of parent.lineage_child_pane_ids ?? []) {
      const child = byPane.get(childId);
      if (!child || seen.has(child.pane_id)) continue;
      seen.add(child.pane_id);
      found.push(child);
      walk(child);
    }
  };
  walk(agent);
  return found;
}

function group(scope: SearchDevice, workspace: Workspace, checkout: Checkout, front: string, rows: SearchEntry[], t: TFunction<"translation">): RelationGroup {
  const pr = checkout.pull_request ? pullRequestEntry(scope, workspace, checkout.pull_request, front, t) : null;
  return { head: checkoutEntry(scope, workspace, checkout, front, true), rows: pr ? [pr, ...rows] : rows };
}

/** The group of a checkout the agent was delegated work in: the descendants there, each naming its parent. */
function delegatedGroup(scope: SearchDevice, workspace: Workspace, checkout: Checkout, front: string, lineage: Lineage, descendants: AgentRow[], places: Map<string, string>, t: TFunction<"translation">): RelationGroup {
  const inside = paneSet(checkout);
  const rows = descendants
    .filter((agent) => inside.has(agent.pane_id))
    .map((agent) => {
      const parent = agent.lineage_parent_pane_id ? lineage.byPane.get(agent.lineage_parent_pane_id) : undefined;
      const entry = agentEntry(scope, agent, places.get(agent.pane_id) ?? null, front, t);
      return {
        ...entry,
        depth: parent && inside.has(parent.pane_id) ? 1 : 0,
        subtitle: parent && !inside.has(parent.pane_id) ? `↑ ${parent.identity_label} · ${entry.subtitle}` : entry.subtitle,
      };
    });
  return group(scope, workspace, checkout, front, rows, t);
}

/**
 * The relations of `target`, or null when it has none the snapshot names.
 * An agent's are its checkout's issues, its checkout's group with its pull
 * request and lineage, and a group for each other checkout it delegated work
 * to; a checkout's are its issues, itself and its pull request; a pull
 * request's or an issue's are those of the checkouts that carry it. `anchor`
 * marks the agent the operator is in front of with the `here` tag.
 */
export function relationsOf(rest: SnapshotRest | null, target: RelationTarget, t: TFunction<"translation">, anchor = false): Relations | null {
  if (!rest) return null;
  const front = frontDeviceId(rest);
  const scopes = searchDevices(rest);
  if (target.kind === "agent") {
    for (const scope of scopes) {
      const agent = scope.agents.find((row) => row.pane_id === target.paneId);
      if (!agent) continue;
      const placed = checkoutOfPane(scope, agent.pane_id);
      if (!placed) return null;
      const places = checkoutPlaces(scope.workspaces);
      const byPane = new Map(scope.agents.map((row) => [row.pane_id, row]));
      const lineage: Lineage = { byPane, here: paneSet(placed.checkout) };
      const own = group(scope, placed.workspace, placed.checkout, front, lineageRows(scope, lineage, agent, places, front, anchor, t), t);
      const groups = [own];
      const delegated = descendantsOf(agent, byPane);
      for (const workspace of scope.workspaces) {
        for (const checkout of workspace.checkouts) {
          if (checkout.id === placed.checkout.id) continue;
          const inside = paneSet(checkout);
          if (!delegated.some((child) => inside.has(child.pane_id))) continue;
          groups.push(delegatedGroup(scope, workspace, checkout, front, lineage, delegated, places, t));
        }
      }
      return { issues: checkoutIssues(scope, placed.workspace, placed.checkout, front, t), groups };
    }
    return null;
  }
  for (const scope of scopes) {
    for (const workspace of scope.workspaces) {
      const matching = workspace.checkouts.filter((checkout) => {
        if (target.kind === "checkout") return checkout.id === target.checkoutId;
        if (target.kind === "pr") return workspace.id === target.workspaceId && checkout.pull_request?.number === target.number;
        return checkout.task_key === target.taskKey || (checkout.closes_task_keys ?? []).includes(target.taskKey);
      });
      if (matching.length === 0) continue;
      const issues: SearchEntry[] = [];
      const seen = new Set<string>();
      for (const checkout of matching) {
        for (const issue of checkoutIssues(scope, workspace, checkout, front, t)) {
          if (!seen.has(issue.id)) {
            seen.add(issue.id);
            issues.push(issue);
          }
        }
      }
      return { issues, groups: matching.map((checkout) => group(scope, workspace, checkout, front, [], t)) };
    }
  }
  return null;
}

/** Every row of `relations` in the order it is drawn: issues, then each group's head and rows. */
export function relationRows(relations: Relations): SearchEntry[] {
  return [...relations.issues, ...relations.groups.flatMap((entry) => [entry.head, ...entry.rows])];
}

/**
 * What is in front, as the thing ⌘K's empty list relates to (PRD B2, B5, B7):
 * an agent pane when the keyboard is in one, else the Workspace's checkout,
 * and nothing on a screen that has no thing in front.
 */
export function frontTarget(
  rest: SnapshotRest | null,
  screen: "main" | "overview" | "workspace" | null,
  over: boolean,
  focusedPaneId: string | null,
  fromAgent: boolean,
  frontCheckoutId: string | null,
): RelationTarget | null {
  if (!rest || over || screen !== "workspace") return null;
  if (fromAgent && focusedPaneId && searchDevices(rest).some((scope) => scope.agents.some((agent) => agent.pane_id === focusedPaneId))) {
    return { kind: "agent", paneId: focusedPaneId };
  }
  return frontCheckoutId ? { kind: "checkout", checkoutId: frontCheckoutId } : null;
}

export function legacyRelations(workspaces: Workspace[], agents: AgentRow[]) {
  const result: import("../src/agentScope").AgentScope["relations"] = {};
  const rest = { navigator: { workspaces, agents } } as SnapshotRest;
  const byPane = new Map(agents.map((a) => [a.pane_id, a]));
  for (const agent of agents) {
    if (result[agent.pane_id]) continue;
    const value = relationsOf(rest, { kind: "agent", paneId: agent.pane_id }, ((key: string) => key) as never, true);
    if (!value) continue;
    result[agent.pane_id] = value.groups.map((g, index) => ({ issues: index === 0 ? value.issues.map(i => i.taskKey!) : [], project_id: g.head.workspaceId!, checkout_id: g.head.checkoutId!, rows: g.rows.filter((r) => r.kind === "agent").map((r) => ({ pane_id: r.paneId!, occurrence: r.tag === "here" ? 0 : agents.filter(a => a.pane_id === r.paneId).length - 1, depth: r.depth ?? 0, tag: r.tag ?? null, caption_parent: r.subtitle?.startsWith("↑ ") ? byPane.get(r.paneId!)?.lineage_parent_pane_id ?? null : null })) }));
  }
  return result;
}
