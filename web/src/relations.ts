// What is connected to a thing, drawn the way a Project's Overview groups it
// (PRD cmdk-navigation D-07, D-08): the issues on top, then one group per
// checkout whose head is the checkout and whose rows are its pull request and
// the agent's lineage. A parent that works in another checkout is one parent
// line under the agent; a child delegated to another checkout stands in that
// checkout's own group. The facts are the ones the snapshot already carries -
// `checkout.task_key`, `closes_task_keys`, `checkout.pull_request` and the
// agent lineage fields - and a connection the snapshot does not name has no
// row, never an empty line (design principle 10).

import { scopeOccurrences } from "./agentScope";
import type { TFunction } from "i18next";
import { frontDeviceId } from "./devices";
import {
  agentEntry,
  checkoutEntry,
  issueEntry,
  pullRequestEntry,
  searchDevices,
  type SearchDevice,
  type SearchEntry,
} from "./search";
import { checkoutPlaces } from "./navigation";
import type { Checkout, SnapshotRest, Workspace } from "./snapshot";

/** One checkout's group: its head and the rows under it, in the order they are drawn. */
export type RelationGroup = { head: SearchEntry; rows: SearchEntry[] };

export type Relations = { issues: SearchEntry[]; groups: RelationGroup[] };

/** What the relations are of: an agent pane, a checkout, a pull request of a project, or an issue. */
export type RelationTarget =
  | { kind: "agent"; paneId: string }
  | { kind: "checkout"; checkoutId: string }
  | { kind: "pr"; workspaceId: string; number: number }
  | { kind: "issue"; taskKey: string };

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

function group(scope: SearchDevice, workspace: Workspace, checkout: Checkout, front: string, rows: SearchEntry[], t: TFunction<"translation">): RelationGroup {
  const pr = checkout.pull_request ? pullRequestEntry(scope, workspace, checkout.pull_request, front, t) : null;
  return { head: checkoutEntry(scope, workspace, checkout, front, true), rows: pr ? [pr, ...rows] : rows };
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
      const projected = scope.agentScope?.relations[agent.pane_id];
      if (!projected) return null;
      const places = checkoutPlaces(scope.allWorkspaces);
      const byPane = new Map(scope.agents.map((row) => [row.pane_id, row]));
      const groups = projected.map((value) => {
        const workspace = scope.allWorkspaces.find((p) => p.id === value.project_id);
        const checkout = workspace?.checkouts.find((c) => c.id === value.checkout_id);
        if (!workspace || !checkout) throw new Error("Missing core relation place");
        const rows = scopeOccurrences(value.rows, scope.agents).map((agent, index) => {
          const row = value.rows[index]!;
          const entry = agentEntry(scope, agent, places.get(row.pane_id) ?? null, front, t);
          const parent = row.caption_parent ? byPane.get(row.caption_parent) : null;
          if (parent === undefined) throw new Error(`Missing relation parent: ${row.caption_parent}`);
          const tag = row.tag === "here" && !anchor ? null : row.tag;
          return { ...entry, depth: row.depth, ...(tag ? { tag } : {}), ...(parent ? { subtitle: `↑ ${parent.identity_label} · ${entry.subtitle}` } : {}) };
        });
        return group(scope, workspace, checkout, front, rows, t);
      });
      const first = projected[0]!;
      const workspace = scope.allWorkspaces.find((p) => p.id === first.project_id)!;
      const issues = first.issues.map((key) => {
        const task = workspace.tasks?.tasks.find((task) => task.key === key);
        if (!task) throw new Error(`Missing core relation issue: ${key}`);
        return issueEntry(scope, workspace, task, front, t);
      });
      return { issues, groups };
    }
    return null;
  }
  for (const scope of scopes) {
    for (const workspace of scope.allWorkspaces) {
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
