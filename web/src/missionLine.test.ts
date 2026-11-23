// A mission node card's bottom line (PRD mission-destination D-52-D-54,
// B55-B57, B60): the branch line of a node that changes code and the result
// line of one that does not, as the desktop and the phone both read them.
import { expect, it } from "vitest";
import { initializeInterfaceI18n } from "./i18n/instance";
import { branchLine, branchLineWords, lineNode, resultLine, resultLineWords, waitCard, waitCardWords, waitConditionWords, waitFootWords, type LineNode } from "./missionLine";
import type { MissionNode } from "./snapshot";

function node(extra: Partial<LineNode> = {}): LineNode {
  return { kind: "change", status: "working", pr: null, pr_state: null, landing: null, delivery: "pr", base_pr: null, branch: "feat/refund-db", ...extra };
}

const t = () => initializeInterfaceI18n("ko").t;

it("draws a change by pull request as its PR list mark, number, branch and destination, a grey branch before the pull request (B55)", () => {
  expect(branchLine(node(), "main")).toEqual({ mark: "branch", pr: null, branch: { name: "feat/refund-db" }, plan: null, direct: false, to: { kind: "branch", name: "main", elsewhere: false }, person: null });
  // Each state is the one the PR list draws; several pull requests show the newest and +N; one GitHub has not answered yet is a plain PR mark.
  for (const state of ["pending", "failed", "mergeable", "draft", "merged", "closed"] as const) {
    expect(branchLine(node({ pr: 3105, pr_state: state }), "main")?.mark).toBe(state);
  }
  expect(branchLine(node({ pr: 3105, pr_state: null, pr_more: 2 }), "main")).toMatchObject({ mark: "pull", pr: { number: 3105, more: 2 } });
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
  expect(branchLine(node({ delivery: "direct" }), "main")).toEqual({ mark: "branch", pr: null, branch: { name: "feat/refund-db" }, plan: null, direct: true, to: { kind: "branch", name: "main", elsewhere: false }, person: null });
  expect(branchLine(node({ delivery: "direct", status: "done", landing: { state: "landed", proof: { by: "in_main", commit: "abc" } } }), "main")?.mark).toBe("merged");
  expect(branchLine(node({ delivery: "direct", status: "done", landing: { state: "left", action: "join" } }), "main")?.mark).toBe("branch");
});

it("puts the operator's merge before the destination as where it stands, and nothing for the Conductor's (D-53, B56)", () => {
  const pr = { pr: 3105, pr_state: "mergeable" as const };
  expect(["ahead", "due", "merged"].map((state) => branchLine(node({ ...pr, merge: { by: "operator", state: state as "ahead" } }), "main")?.person)).toEqual(["ahead", "due", "merged"]);
  expect(branchLine(node({ ...pr, merge: { by: "conductor", state: "ahead" } }), "main")?.person).toBeNull();
});

it("draws no branch line for a waiting node, which draws what it waits for instead (option A, PRD core-wait D-15)", () => {
  const wait = { kind: "wait" as const, delivery: null, branch: null };
  expect(branchLine(node({ ...wait, external: { pr: 1232 }, outside: { base: "main", state: "pending" } }), "main")).toBeNull();
  expect(branchLine(node({ ...wait, status: "done", wait_target: { branch: "vendor/sdk" }, external: { branch: "vendor/sdk" } }), "main")).toBeNull();
  expect(branchLine(node({ ...wait, wait_target: { ci: 1232 } }), "main")).toBeNull();
});

it("names no branch or destination it was not sent: no branch, no `→` without a destination, no bold without one to differ from (B59, B60)", () => {
  // A mission from before destinations and the phone's frame, which carries no branch.
  expect(branchLine(node({ branch: null, pr: 31, pr_state: "pending" }), null)).toMatchObject({ pr: { number: 31 }, branch: null, to: null });
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
  expect(resultLine({ ...research, status: "done", line: "the reopen order" })).toEqual({ preset: "research", tone: "done", shows: "line", line: "the reopen order" });
  expect(resultLine({ ...deploy, status: "done", line: "1.2.8 out" })).toEqual({ preset: "deploy", tone: "done", shows: "line", line: "1.2.8 out" });
  expect(resultLine({ ...none, status: "done", line: "three pages reworded" })).toEqual({ preset: null, tone: "done", shows: "line", line: "three pages reworded" });
  // A verdict wins over the line on any preset, red when it failed (B2).
  expect(resultLine({ ...verify, status: "done", line: "two failed", verdict: "fail", checks: { passed: 5, total: 7 } })).toEqual({ preset: "verify", tone: "failed", shows: "verdict", verdict: "fail", checks: { passed: 5, total: 7 }, line: "two failed" });
  expect(resultLine({ ...research, status: "done", line: "why", verdict: "pass", checks: null })).toEqual({ preset: "research", tone: "done", shows: "verdict", verdict: "pass", checks: null, line: "why" });
  // A finished node with nothing kept says its preset, as before it finished.
  expect(resultLine({ ...none, status: "done" })).toEqual({ preset: null, tone: "done", shows: "kind" });
  const words = t();
  expect([
    resultLineWords(resultLine({ ...verify, status: "done", verdict: "pass", checks: { passed: 7, total: 7 } })!, words),
    ...[research, verify, deploy, none].map((one) => resultLineWords(resultLine({ ...one, status: "planned" })!, words)),
  ]).toEqual(["통과 7/7", "리서치", "검증", "배포·출시", "일반"]);
});

// Option A of the operator's decision on with-hide/hide pull request 2848 (PRD core-wait D-15-D-17, B4-B8): a waiting node's
// card says what it waits for in one sentence, and its foot carries only what it has.
const HOUR = 3_600_000;
const NOW = 1_000 * HOUR;
const waiting = (extra: Partial<LineNode>) => node({ kind: "wait", delivery: null, branch: null, wait_until_unix_ms: NOW + 6 * 24 * HOUR, ...extra });
const mission91 = { wait_target: { mission: "mission-91" }, wait_progress: { done: 2, total: 7 } };
const gitlabWrite = { wait_target: { node: { mission: "mission-91", node: "gitlab-write" } } };

it("says what a waiting node waits for in one sentence, whatever it waits on and however it ended", () => {
  const words = t();
  const said = (extra: Partial<LineNode>) => waitConditionWords(waitCard(waiting(extra), "external", NOW)!.aim, words);
  expect(said({ wait_target: { pr: 2528 }, external: { pr: 2528 } })).toBe(`#${2528} 머지되면`);
  // A pull request is named as its code host writes it, the core's reference.
  expect(said({ wait_target: { ci: 1232 }, wait_label: "!1232" })).toBe("!1232 CI 통과하면");
  expect(said({ wait_target: { branch: "release/1.3" }, outside: { base: "main" } })).toBe("main에 release/1.3 들어오면");
  expect(said({ wait_target: { branch: "release/1.3" } })).toBe("기본 브랜치에 release/1.3 들어오면");
  expect(said(mission91)).toBe("mission-91 끝나면");
  expect(said(gitlabWrite)).toBe("mission-91 · gitlab-write 끝나면");
  // A waiting node from a core before waits names only the pull request or branch it watched.
  expect(said({ external: { pr: 9 } })).toBe("#9 머지되면");
  // The sentence stays once it is met or failed; the foot says how it ended.
  expect(said({ wait_target: { pr: 2528 }, status: "done" })).toBe(`#${2528} 머지되면`);
  expect(said({ wait_target: { pr: 2528 }, wait_outcome: "closed_unmerged" })).toBe(`#${2528} 머지되면`);
  // No card for another kind of node, nor for a waiting one whose target the frame does not carry.
  expect(waitCard(node({ pr: 7 }), "external", NOW)).toBeNull();
  expect(waitCard(waiting({}), "external", NOW)).toBeNull();
});

it("puts in a waiting card's foot only what it has: progress, the hours under a day, met, how it failed and whose turn, a read that could not say", () => {
  const words = t();
  const foot = (extra: Partial<LineNode>) => waitFootWords(waitCard(waiting(extra), "external", NOW)!, words);
  const turn = (to: "operator" | "conductor") => ({ attention: { to } });
  expect(foot({ wait_target: { pr: 2528 } })).toEqual([]);
  expect(foot(mission91)).toEqual(["끝 2/7"]);
  // Under a day the hours left, rounded up; a day or more says nothing.
  expect(foot({ ...mission91, wait_until_unix_ms: NOW + 5 * HOUR - 60_000 })).toEqual(["끝 2/7", "5시간 남음"]);
  expect(foot({ wait_target: { ci: 1 }, wait_until_unix_ms: NOW + 60_000 })).toEqual(["1시간 남음"]);
  expect(foot({ wait_target: { ci: 1 }, wait_until_unix_ms: NOW + 24 * HOUR })).toEqual([]);
  // Met, the progress and the hours are gone.
  expect(foot({ ...mission91, status: "done", wait_until_unix_ms: NOW + HOUR })).toEqual(["끝남"]);
  expect(foot({ wait_target: { pr: 2528 }, wait_outcome: "closed_unmerged", ...turn("conductor") })).toEqual(["머지 없이 닫힘", "Conductor 차례"]);
  // A failed CI names its first failed check and counts the rest.
  expect(foot({ wait_target: { ci: 1232 }, wait_outcome: "ci_failed", wait_checks: ["rust workspace", "web e2e", "lint"], ...turn("conductor") })).toEqual(["CI 실패 · rust workspace 외 2", "Conductor 차례"]);
  expect(foot({ wait_target: { ci: 1232 }, wait_outcome: "ci_failed", wait_checks: ["rust workspace"] })).toEqual(["CI 실패 · rust workspace"]);
  expect(foot({ wait_target: { ci: 1232 }, wait_outcome: "ci_failed" })).toEqual(["CI 실패"]);
  expect(foot({ ...mission91, wait_outcome: "mission_cancelled", ...turn("conductor") })).toEqual(["기다리던 미션이 취소됨", "Conductor 차례"]);
  expect(foot({ ...gitlabWrite, wait_outcome: "node_dropped", ...turn("operator") })).toEqual(["노드가 계획에서 빠짐", "내 차례"]);
  expect(foot({ wait_target: { pr: 2528 }, wait_until_unix_ms: NOW - HOUR, wait_outcome: "expired", ...turn("conductor") })).toEqual(["기한 지남", "Conductor 차례"]);
  expect(foot({ wait_target: { ci: 1232 }, unread: true })).toEqual(["확인 못 함 · 다시 읽는 중"]);
  // The card's name reads the sentence, then its foot.
  expect(waitCardWords(waitCard(waiting({ wait_target: { ci: 1232 }, wait_until_unix_ms: NOW + 5 * HOUR - 60_000 }), "external", NOW)!, words)).toBe(`#${1232} CI 통과하면 · 5시간 남음`);
});

it("gives a waiting card the state its head icon and border draw: waiting, unread, met or failed", () => {
  const state = (extra: Partial<LineNode>) => waitCard(waiting({ wait_target: { pr: 2528 }, ...extra }), "external", NOW)?.state;
  expect([state({}), state({ unread: true }), state({ status: "done" }), state({ wait_outcome: "closed_unmerged" }), state({ unread: true, wait_outcome: "expired" })]).toEqual(["waiting", "unread", "met", "failed", "failed"]);
});

it("gives a wait no one judges any more no card: dropped from its plan, or in a cancelled mission before it was met (Opus review S1)", () => {
  const failed = waiting({ wait_target: { pr: 2528 }, wait_outcome: "closed_unmerged", attention: { to: "conductor" }, wait_until_unix_ms: NOW + HOUR });
  expect([waitCard(failed, "dropped", NOW), waitCard(failed, "cancelled", NOW), waitCard(waiting(mission91), "cancelled", NOW)]).toEqual([null, null, null]);
  // A wait met before the mission ended keeps its card: its look is done.
  expect(waitCard(waiting({ ...mission91, status: "done" }), "done", NOW)?.state).toBe("met");
});
