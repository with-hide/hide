// A mission node card's bottom line (PRD mission-destination D-52-D-54,
// B55-B57, B60): the branch line of a node that changes code and the result
// line of one that does not, as the desktop and the phone both read them.
import { expect, it } from "vitest";
import { initializeInterfaceI18n } from "./i18n/instance";
import { branchLine, branchLineWords, lineNode, resultLine, resultLineWords, type LineNode } from "./missionLine";
import type { MissionNode } from "./snapshot";

function node(extra: Partial<LineNode> = {}): LineNode {
  return { kind: "change", status: "working", pr: null, pr_state: null, landing: null, delivery: "pr", base_pr: null, branch: "feat/refund-db", ...extra };
}

const t = () => initializeInterfaceI18n("ko").t;

it("draws a change by pull request as its PR list mark, number, branch and destination, a grey branch before the pull request (B55)", () => {
  expect(branchLine(node(), "main")).toEqual({ mark: "branch", pr: null, branch: { name: "feat/refund-db", outside: false }, plan: null, direct: false, to: { kind: "branch", name: "main", elsewhere: false }, person: null });
  // Each state is the one the PR list draws; several pull requests show the newest and +N; one GitHub has not answered yet is a plain PR mark.
  for (const state of ["pending", "failed", "mergeable", "draft", "merged", "closed"] as const) {
    expect(branchLine(node({ pr: 3105, pr_state: state }), "main")?.mark).toBe(state);
  }
  expect(branchLine(node({ pr: 3105, pr_state: null, pr_more: 2 }), "main")).toMatchObject({ mark: "pull", pr: { number: 3105, outside: false, more: 2 } });
});

it("ends in another base, or in the pull request it is stacked on with that pull request's state as the mission's nodes know it (B55, D-57)", () => {
  expect(branchLine(node({ pr: 3106, pr_state: "pending", base_pr: 3105 }), "main")?.to).toEqual({ kind: "stack", pr: 3105, state: null });
  // The desktop reads the base's state from whichever node of the mission holds that pull request; none holds it, none is named.
  const held = (number: number, state: MissionNode["prs"][number]["state"]) => ({ prs: [{ number, state, url: null }] });
  const stacked = { kind: "change", status: "working", pr: 3106, pr_state: "pending", landing: null, merge: null, delivery: "pr", base_pr: 3105, branch: "feat/screen", prs: [{ number: 3106, state: "pending", url: null }] } as unknown as MissionNode;
  expect(branchLine(lineNode(stacked, [held(3104, "merged"), held(3105, "mergeable")]), "main")?.to).toEqual({ kind: "stack", pr: 3105, state: "mergeable" });
  expect(branchLine(lineNode(stacked, [held(3104, "merged")]), "main")?.to).toEqual({ kind: "stack", pr: 3105, state: null });
  expect(branchLine(node({ pr: 3106, pr_state: "pending", pr_base: "release/1.2" }), "main")?.to).toEqual({ kind: "branch", name: "release/1.2", elsewhere: true });
  expect(branchLine(node({ pr: 3106, pr_state: "pending", pr_base: "main" }), "main")?.to).toEqual({ kind: "branch", name: "main", elsewhere: false });
});

it("says the plan before a node starts: a faint mark and `PR` or `PR 없이` to the destination, the operator's merge already on it (D-55, B55, B58)", () => {
  expect(branchLine(node({ status: "planned", branch: null, merge: { by: "operator", state: "ahead" } }), "main")).toEqual({ mark: "plan_pr", pr: null, branch: null, plan: "pr", direct: false, to: { kind: "branch", name: "main", elsewhere: false }, person: "ahead" });
  expect(branchLine(node({ status: "planned", delivery: "direct", branch: null }), "main")).toMatchObject({ mark: "plan_direct", plan: "direct", direct: false });
  // A node replanned after it opened a pull request keeps the pull request it has.
  expect(branchLine(node({ status: "planned", pr: 7, pr_state: "pending" }), "main")).toMatchObject({ mark: "pending", plan: null });
});

it("draws a direct change as its branch ending in `PR 없이`, the merge mark once it is in the destination (B55)", () => {
  expect(branchLine(node({ delivery: "direct" }), "main")).toEqual({ mark: "branch", pr: null, branch: { name: "feat/refund-db", outside: false }, plan: null, direct: true, to: { kind: "branch", name: "main", elsewhere: false }, person: null });
  expect(branchLine(node({ delivery: "direct", status: "done", landing: { state: "landed", proof: { by: "in_main", commit: "abc" } } }), "main")?.mark).toBe("merged");
  expect(branchLine(node({ delivery: "direct", status: "done", landing: { state: "left", action: "join" } }), "main")?.mark).toBe("branch");
});

it("puts the operator's merge before the destination as where it stands, and nothing for the Conductor's (D-53, B56)", () => {
  const pr = { pr: 3105, pr_state: "mergeable" as const };
  expect(["ahead", "due", "merged"].map((state) => branchLine(node({ ...pr, merge: { by: "operator", state: state as "ahead" } }), "main")?.person)).toEqual(["ahead", "due", "merged"]);
  expect(branchLine(node({ ...pr, merge: { by: "conductor", state: "ahead" } }), "main")?.person).toBeNull();
});

it("draws an outside node as `↗#n` lit by GitHub's answer or `↗ <branch>`, to the base it goes to, merged once it is done (D-52, B55)", () => {
  const outside = { kind: "external" as const, delivery: null, branch: null };
  expect(branchLine(node({ ...outside, external: { pr: 1232 }, outside: { base: "main", state: "pending" } }), "main")).toEqual({ mark: "pending", pr: { number: 1232, outside: true, more: 0 }, branch: null, plan: null, direct: false, to: { kind: "branch", name: "main", elsewhere: false }, person: null });
  expect(branchLine(node({ ...outside, status: "done", external: { pr: 1232 }, outside: { base: "release/1.2" } }), "main")).toMatchObject({ mark: "merged", to: { kind: "branch", name: "release/1.2", elsewhere: true } });
  expect(branchLine(node({ ...outside, external: { branch: "vendor/sdk" }, outside: { base: "develop" } }), "main")).toMatchObject({ mark: "branch", branch: { name: "vendor/sdk", outside: true }, to: { kind: "branch", name: "develop", elsewhere: true } });
});

it("names no branch or destination it was not sent: no branch, no `→` without a destination, no bold without one to differ from (B59, B60)", () => {
  // A mission from before destinations, an outside node from before its base, and the phone's frame, which carries no branch.
  expect(branchLine(node({ branch: null, pr: 31, pr_state: "pending" }), null)).toMatchObject({ pr: { number: 31 }, branch: null, to: null });
  expect(branchLine(node({ kind: "external", delivery: null, external: { pr: 9 } }), "main")?.to).toBeNull();
  expect(branchLine(node({ pr: 31, pr_state: "pending", pr_base: "release/1.2" }), null)?.to).toEqual({ kind: "branch", name: "release/1.2", elsewhere: false });
  const phone: LineNode = { ...node({ pr: 31, pr_state: "pending" }) };
  delete phone.branch;
  expect(branchLine(phone, "main")).toMatchObject({ branch: null, pr: { number: 31 }, to: { name: "main" } });
});

it("says the line in words, its pull request's state and the operator's merge included, so no state is only a colour (B55, B56)", () => {
  const words = t();
  expect(branchLineWords(branchLine(node({ pr: 3105, pr_state: "mergeable", pr_more: 1, merge: { by: "operator", state: "due" } }), "main")!, words)).toBe("#3105 +1 feat/refund-db → main · 머지 가능 · 머지는 내가 · 머지할 차례");
  expect(branchLineWords(branchLine(node({ pr: 3106, pr_state: "pending", base_pr: 3105 }), "main")!, words)).toBe("#3106 feat/refund-db → PR 3105 · CI 실행 중이거나 리뷰 대기");
  expect(branchLineWords(branchLine(node({ status: "planned", delivery: "direct", branch: null }), "main")!, words)).toBe("PR 없이 → main");
  expect(branchLineWords(branchLine(node({ delivery: "direct" }), "main")!, words)).toBe("feat/refund-db → main · PR 없이");
  expect(branchLineWords(branchLine(node({ kind: "external", delivery: null, branch: null, external: { branch: "vendor/sdk" } }), null)!, words)).toBe("↗ vendor/sdk");
});

it("gives every general node its preset's line: the name until done, then the result line or a verdict of any preset, the icon by where it stands (PRD node-presets D-08, B1, B2, B5)", () => {
  const general = (preset: "research" | "verify" | "deploy" | null) => node({ kind: "general", preset, delivery: null, branch: null });
  const [research, verify, deploy, none] = [general("research"), general("verify"), general("deploy"), general(null)];
  expect([research, verify, deploy, none].map((one) => branchLine(one, "main"))).toEqual([null, null, null, null]);
  expect(resultLine(node())).toBeNull();
  // Planned grey, worked blue: the preset's name whatever line its worker last said.
  expect(resultLine({ ...research, status: "planned" })).toEqual({ preset: "research", tone: "planned", shows: "kind" });
  expect(resultLine({ ...deploy, line: "signing the package" })).toEqual({ preset: "deploy", tone: "working", shows: "kind" });
  // Done green with the result line itself, whatever the preset, none included (B1, B5).
  expect(resultLine({ ...research, status: "done", line: "the reopen order" })).toEqual({ preset: "research", tone: "done", shows: "conclusion", line: "the reopen order" });
  expect(resultLine({ ...deploy, status: "done", line: "1.2.8 out" })).toEqual({ preset: "deploy", tone: "done", shows: "conclusion", line: "1.2.8 out" });
  expect(resultLine({ ...none, status: "done", line: "three pages reworded" })).toEqual({ preset: null, tone: "done", shows: "conclusion", line: "three pages reworded" });
  // A verdict wins over the line on any preset, red when it failed (B2).
  expect(resultLine({ ...verify, status: "done", line: "two failed", verdict: "fail", checks: { passed: 5, total: 7 } })).toEqual({ preset: "verify", tone: "failed", shows: "verdict", verdict: "fail", checks: { passed: 5, total: 7 } });
  expect(resultLine({ ...research, status: "done", line: "why", verdict: "pass", checks: null })).toEqual({ preset: "research", tone: "done", shows: "verdict", verdict: "pass", checks: null });
  // A finished node with nothing kept says its preset, as before it finished.
  expect(resultLine({ ...none, status: "done" })).toEqual({ preset: null, tone: "done", shows: "kind" });
  const words = t();
  expect([
    resultLineWords(resultLine({ ...verify, status: "done", verdict: "pass", checks: { passed: 7, total: 7 } })!, words),
    ...[research, verify, deploy, none].map((one) => resultLineWords(resultLine({ ...one, status: "planned" })!, words)),
  ]).toEqual(["통과 7/7", "리서치", "검증", "배포·출시", "일반"]);
});
