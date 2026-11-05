// The phone's PR groups and merge rules (PRD mobile-companion-ui B37, B40, B41, D-12): which of the four groups a core reason lands in,
// that a settled pull request is not listed, and that a confirmation closes unless the page still shows what it named.
import { expect, it } from "vitest";
import type { PrReasonKind } from "../agentScope";
import { HEAD, card, pr, project, standing } from "./prFixture";
import { confirmOf, groupPrs, holds, prActions, prGroupOf, type PrGroup } from "./prModel";

const EVERY_KIND: Record<PrReasonKind, PrGroup> = {
  merge: "ready",
  ci: "running",
  working: "running",
  queued: "running",
  failed: "failed",
  changes: "failed",
  conflict: "failed",
  update: "failed",
  waiting_review: "review",
  assign_reviewer: "review",
  assign_reviewer_optional: "review",
  review: "review",
  team_review: "review",
  reply: "review",
  check: "review",
  unjudged: "review",
  checking: "review",
  draft: "review",
  merged: "review",
};

it("puts every reason the core can give in one of the four groups, and refuses one it does not know", () => {
  for (const [kind, group] of Object.entries(EVERY_KIND)) expect(prGroupOf(kind as PrReasonKind), kind).toBe(group);
  expect(() => prGroupOf("invented" as PrReasonKind)).toThrow("does not group");
});

it("lists the open pull requests in the phone's group order, each group in the core's order, and leaves out the settled and the empty groups", () => {
  const items = [
    standing(1, "waiting_review"),
    standing(2, "failed"),
    pr(3),
    standing(4, "ci"),
    pr(5),
    standing(6, "merged", { state: "merged", badge: "merged", group: "merged" }),
  ];
  const groups = groupPrs(items, [project()]);
  expect(groups.map((view) => [view.group, view.entries.map((entry) => entry.pr.number)])).toEqual([
    ["ready", [3, 5]],
    ["running", [4]],
    ["failed", [2]],
    ["review", [1]],
  ]);
  expect(groupPrs([pr(7, { state: "closed", badge: "closed" })], [project()])).toEqual([]);
});

it("refuses a listed pull request whose project the frame does not carry", () => {
  expect(() => groupPrs([pr(1, { project: "elsewhere" })], [project()])).toThrow("without its project");
});

it("opens a confirmation only where the desktop card would offer Merge: the operator's own pull request, nothing blocking, the card read", () => {
  expect(confirmOf(pr(1))).toEqual({ key: "p1#1", action: "merge", head: HEAD, method: "squash", queue: false });
  // Someone else's pull request has no Merge on the desktop card.
  expect(confirmOf(pr(2, { work: { ...pr(2).work!, mine: false } }))).toBeNull();
  // A card not read yet has no head to name.
  expect(confirmOf(pr(3, { card: null }))).toBeNull();
  expect(prActions(pr(3, { card: null })).why?.key).toBe("unread");
  // A draft, a failing check and a missing permission block Merge with the desktop's reason.
  expect(prActions(pr(4, { is_draft: true })).why?.key).toBe("draft");
  expect(prActions(pr(5, { work: { ...pr(5).work!, merge_state: "blocked", pending_checks: 2 } })).why).toMatchObject({ key: "ci", count: 2 });
  expect(prActions(pr(6, { card: card({ merge: { methods: ["squash"], queue: false, can_merge: false } }) })).why?.key).toBe("permission");
  // An agent working on it blocks Merge.
  expect(prActions(pr(7, { group: "working", reason: { kind: "working" } })).why?.key).toBe("agent");
  // A merge in flight or already queued is not asked twice.
  expect(confirmOf(pr(8, { card: card({ write: { action: "merge", head: HEAD, method: "squash", phase: "working", reason: null } }) }))).toBeNull();
  expect(confirmOf(pr(9, { work: { ...pr(9).work!, queue: { state: "at", position: 1 } } }))).toBeNull();
  // A refusal GitHub would only repeat keeps Merge off until the card is read again; a transient one does not.
  const refused = (reason: "head_moved" | "unreachable") => pr(10, { card: card({ write: { action: "merge", head: HEAD, method: "squash", phase: "failed", reason } }) });
  expect(confirmOf(refused("head_moved"))).toBeNull();
  expect(confirmOf(refused("unreachable"))).not.toBeNull();
});

it("uses the merge queue's words and no method when the repository has a queue", () => {
  const queued = pr(1, { card: card({ merge: { methods: ["squash"], queue: true, can_merge: true } }) });
  expect(confirmOf(queued)).toEqual({ key: "p1#1", action: "merge", head: HEAD, method: "squash", queue: true });
});

it("closes a confirmation when the page no longer shows the head, the queue or the method it named, or no longer allows Merge", () => {
  const confirm = confirmOf(pr(1))!;
  expect(holds(confirm, pr(1))).toBe(true);
  expect(holds(confirm, pr(1, { card: card({ head: "b".repeat(40) }) }))).toBe(false);
  expect(holds(confirm, pr(1, { card: card({ merge: { methods: ["merge"], queue: false, can_merge: true } }) }))).toBe(false);
  expect(holds(confirm, pr(1, { card: card({ merge: { methods: ["squash"], queue: true, can_merge: true } }) }))).toBe(false);
  expect(holds(confirm, pr(1, { is_draft: true }))).toBe(false);
  expect(holds(confirm, pr(2))).toBe(false);
  expect(holds(confirm, null)).toBe(false);
});
