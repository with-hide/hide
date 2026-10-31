// The real mission page and Settings › Conductor over invented values (PRD
// software-factory-conductor, mission-map-git-criteria), one state per
// `state` parameter: plan, running (every card look: working, review with its
// checkers, done with a merge left, in main, outside, reopened by a defect),
// rejected (running, with the search fix's check failed and one reviewer's
// rejection, its short reason first), decision (a decision, a failed node, a stuck worker and an outside PR
// closed unmerged in the operator's turn), decision-sent (its answer waiting
// for the Conductor's turn), decision-unreadable (its answer waiting on a
// Conductor session Hide cannot read), decision-unconfirmed (its answer typed without
// Herdr confirming it), stack (the merge order), no-conductor, stale, failed
// (the record unreadable), wide (a row wider than the slot, shrunk to fit),
// done, cancelled, no-pr (a finished mission with no pull request), exempt
// (a finished mission from before finish lines, its old sentence on each
// card beside the worker), settings.
// Every map state draws its Conductor row (asleep once the mission closed,
// none for no-conductor, which offers 이어받기 there), the summary beside the
// map while no node is picked, and the checkers as satellites beside their
// cards; done leads with its result tiles and pull requests (PRD mission-tool
// B12-B21). `pick=<node>` opens that node's panel, `width` narrows the slot
// and `scale` sets the text size. The
// Conductor pane's head with its mission chip and pinned popover (PRD
// mission-pane-chip, mission-plan-visibility) is `pane-running`,
// `pane-decision`, `pane-first-plan`, `pane-done` and `pane-cancelled`.
// Nothing leaves this scene.
import { useEffect, useLayoutEffect, useMemo } from "react";
import { AgentMark } from "../AgentMark";
import { createActions } from "../actions";
import { TooltipProvider } from "../components/ui/tooltip";
import { useFollowInterfaceLanguage } from "../i18n/translator";
import { requireInterfaceLanguage } from "../i18n/locale";
import { MissionMap } from "../MissionMap";
import { PaneWorkChip } from "../PaneMissionChip";
import { ConductorTab } from "../settings/ConductorTab";
import type { AgentRow, Mission, MissionChecker, MissionCriterion, MissionEvent, MissionLanding, MissionNode, MissionWorker, SnapshotRest } from "../snapshot";
import { useShellStore } from "../store";
import { useUiStore } from "../ui";

/** The Conductor's pane, as its agent row and the mission name it. */
const CONDUCTOR_PANE = "w1:p1";
/** A pane state reads the map state it draws small. */
const PANE_STATES: Record<string, string> = { "pane-running": "running", "pane-decision": "decision", "pane-first-plan": "plan", "pane-done": "done", "pane-cancelled": "cancelled" };

/** The report a finished mission's Conductor left, invented for the boards, in Markdown as a Conductor writes it. */
const REPORT = "# 이슈 정리 보고: 네 개 끝, 화면 PR까지 머지\n\n## 남은 일\n- [ ] 알림 문구 다듬기는 다음 미션으로\n\n## 검증\n- 격리 앱에서 **6개 항목** 통과\n- 머지 순서는 API, 화면";

const MINUTE = 60_000;
/** An issue or pull request number as the boards write it, built so no literal reads as a color. */
const no = (n: number) => `#${n}`;

function node(key: string, title: string, extra: Partial<MissionNode>): MissionNode {
  return { key, title, kind: "change", depends_on: [], status: "planned", blocked: null, delivery: "pr", worker: null, agent_id: null, pane_id: null, machine: null, pr: null, base_pr: null, branch: null, pr_state: null, pr_url: null, stage: null, attempts: 0, line: null, change: null, review: null, criteria: [], criteria_met: 0, criteria_skipped: 0, external: null, unread: false, reopened: 0, started_at_unix_ms: null, finished_at_unix_ms: null, commit: null, landing: null, verdict: null, checks: null, ...extra };
}

/** Where a node is on its way to main, as the core would publish it (`herdr-core/src/runtime/mission_landing.rs`). */
const MERGE_LEFT: MissionLanding = { state: "left", action: "merge" };
const MERGED: MissionLanding = { state: "landed", proof: { by: "merged" } };
const OUTSIDE_LANDED: MissionLanding = { state: "landed", proof: { by: "outside" } };
const inMain = (commit: string): MissionLanding => ({ state: "landed", proof: { by: "in_main", commit } });

const opus = (effort: string | null) => ({ agent: "claude", model: "Opus", effort, effort_auto: true });
const astra = (effort: string | null) => ({ agent: "codex", model: "Astra", effort, effort_auto: true });
const sonnet = { agent: "claude", model: "Sonnet", effort: "low", effort_auto: false };
/** The screen node's pane, whose recorded launch runs another model than the plan named (the card shows the pane's). */
const SCREEN_PANE = "w1:p7";
/** The pane of the reviewer the core bound in the running state, whose recorded launch the strip shows. */
const REVIEW_PANE = "w1:p8";
/** The pane of the worker the core raised in the decision state. */
const FIX_PANE = "w1:p5";

/** One finish line, and a node's lines with the counts the core derives from them. */
const line = (key: string, kind: MissionCriterion["kind"], state: MissionCriterion["state"], extra: Partial<MissionCriterion> = {}): MissionCriterion => ({ key, kind, state, ...extra });
const own = (index: number, text: string, state: MissionCriterion["state"] = "pass", extra: Partial<MissionCriterion> = {}) => line(`c${index}`, "own", state, { text, ...extra });
const ci = (state: MissionCriterion["state"]) => line("ci", "ci", state);
function finish(...lines: MissionCriterion[]) {
  const counted = (states: MissionCriterion["state"][]) => lines.filter((one) => states.includes(one.state)).length;
  return { criteria: lines, criteria_met: counted(["pass", "skipped"]), criteria_skipped: counted(["skipped"]) };
}
const checker = (worker: MissionWorker, state: MissionCriterion["state"], agent_id?: string): MissionChecker => ({ worker, state, ...(agent_id ? { agent_id } : {}) });

/** The eight nodes of the boards (cmm1-cmm7): four starts, two that wait, one check together. */
function plan(state: string, now: number): MissionNode[] {
  const ago = (minutes: number) => now - minutes * MINUTE;
  const planned = state === "plan";
  const done = (minutes: number, length: number) => ({ status: "done" as const, started_at_unix_ms: ago(minutes), finished_at_unix_ms: ago(minutes - length) });
  const working = (minutes: number) => ({ status: "working" as const, started_at_unix_ms: ago(minutes) });
  if (state === "wide") {
    // One base, then six parts side by side, each with its own review: a row wider than the slot.
    const parts = ["목록", "검색", "설정", "알림", "단축키", "도움말"];
    return [
      node("base", "공용 바탕", { worker: opus("high"), ...done(40, 20), pr: 1601, pr_state: "mergeable", landing: MERGE_LEFT, stage: 3 }),
      node("base-check", "공용 바탕 리뷰", { kind: "verify", delivery: null, depends_on: ["base"], worker: astra("medium"), ...done(20, 6), line: "필수 수정 없음" }),
      ...parts.flatMap((title, index) => [
        node(`part-${index}`, `${title} 화면`, { depends_on: ["base-check"], worker: opus("high"), ...(index < 3 ? working(12 - index * 3) : {}), branch: index < 3 ? `part-${index + 1}` : null, stage: index < 3 ? 1 : null }),
        node(`part-${index}-check`, `${title} 화면 리뷰`, { kind: "verify", delivery: null, depends_on: [`part-${index}`], worker: astra("medium") }),
      ]),
    ];
  }
  if (state === "no-pr") {
    // A release with no pull request: research, two direct changes on main and a check (PRD mission-plan-visibility B22, B23).
    return [
      node("rules", "열린 PR 정리 기준 정하기", { kind: "research", delivery: null, worker: opus("high"), ...done(180, 30), line: "19개 중 12개 닫기" }),
      node("bump", "1.0.0 버전 커밋", { delivery: "direct", depends_on: ["rules"], worker: astra("medium"), ...done(150, 20), branch: "release/1.0.0", stage: 4, commit: "c216793eb4a1" }),
      node("notes", "릴리스 노트", { delivery: "direct", depends_on: ["rules"], worker: sonnet, ...done(150, 40), branch: "release/notes", stage: 4, landing: inMain("8d1b0c4e92f7") }),
      node("agents", "Mac 에이전트 확인", { kind: "verify", delivery: null, depends_on: ["bump"], worker: astra("medium"), ...done(110, 50), line: "7개 항목 통과", verdict: "pass", checks: { passed: 7, total: 7 } }),
      node("order", "배포 순서 확인", { kind: "research", delivery: null, depends_on: ["notes"], worker: opus("medium"), ...done(100, 20), line: "private 먼저, site 뒤" }),
    ];
  }
  if (state === "done" || state === "cancelled") {
    const cancelled = state === "cancelled";
    // Each worker and checker the core bound, so the result tiles count them.
    const reviewed = (who: MissionWorker, id: string, state: MissionCriterion["state"] = "pass") => line("review", "review", state, { checkers: [checker(who, state, id)] });
    return [
      node("cause", `${no(1487)} 원인 조사`, { kind: "research", delivery: null, worker: opus("xhigh"), agent_id: "agent-cause", ...done(300, 40), line: "reopen 때 셀 초기화가 먼저 돎", ...finish(own(1, "원인을 재현 단계와 함께 적는다")) }),
      node("api", "API 추가", { worker: astra("high"), agent_id: "agent-api", ...done(300, 120), pr: 1503, pr_state: "merged", landing: MERGED, stage: 4, ...finish(own(1, "목록 API가 페이지를 나눠 준다"), reviewed(opus("high"), "reviewer-api"), ci("pass")) }),
      node("fix-1481", `이슈 ${no(1481)} 수정`, { delivery: "direct", worker: astra("high"), agent_id: "agent-1481", ...done(290, 60), branch: "fix-1481", stage: 4, landing: inMain("4f1c9a27be03"), ...finish(own(1, "빈 목록에서도 필터가 남는다"), line("check", "check", "pass")) }),
      node("fix-1487", `${no(1487)} 수정`, { depends_on: ["cause"], worker: astra("medium"), agent_id: "agent-1487", ...done(250, 90), pr: 1506, pr_state: "merged", landing: MERGED, stage: 4, attempts: 2, ...finish(own(1, "reopen 뒤에도 셀이 남는다"), own(2, "실제 세션 50개 replay", "skipped", { skip: { reason: "기록된 세션이 31개뿐" } }), ci("pass")) }),
      cancelled
        ? node("screen", "API 쓰는 화면: 목록·검색·필터를 새 페이지 API로 옮기기", { depends_on: ["api"], worker: astra("high"), agent_id: "agent-screen", status: "working", started_at_unix_ms: ago(170), pr: 1504, base_pr: 1503, pr_state: "pending", stage: 2, ...finish(own(1, "새 API로 목록을 그린다", "open"), reviewed(opus("high"), "reviewer-screen", "open")) })
        : node("screen", "API 쓰는 화면: 목록·검색·필터를 새 페이지 API로 옮기기", { depends_on: ["api"], worker: astra("high"), agent_id: "agent-screen", ...done(170, 110), pr: 1504, base_pr: 1503, pr_state: "merged", landing: MERGED, stage: 4, ...finish(own(1, "새 API로 목록을 그린다"), reviewed(opus("high"), "reviewer-screen"), ci("pass")) }),
      node("together", "함께 확인", { kind: "verify", delivery: null, depends_on: ["screen", "fix-1487"], worker: sonnet, ...(cancelled ? {} : { ...done(55, 25), agent_id: "agent-together" }), line: cancelled ? null : "6개 항목 통과", ...finish(line("verify", "verify", cancelled ? "open" : "pass", { rule: "all", checkers: [checker(sonnet, cancelled ? "open" : "pass", cancelled ? undefined : "checker-1"), checker(astra("medium"), cancelled ? "open" : "pass", cancelled ? undefined : "checker-2")] })) }),
    ];
  }
  if (planned) {
    return [
      node("cause", `${no(1487)} 원인 조사`, { kind: "research", delivery: null, worker: opus(null) }),
      node("fix-1481", `이슈 ${no(1481)} 수정`, { worker: astra(null) }),
      node("fix-1483", `이슈 ${no(1483)} 수정`, { worker: astra(null) }),
      node("api", "API 추가", { worker: astra(null) }),
      node("fix-1487", `${no(1487)} 수정`, { depends_on: ["cause"], worker: astra(null) }),
      node("screen", "API 쓰는 화면", { depends_on: ["api"], worker: opus(null) }),
      node("together", "함께 확인", { kind: "verify", delivery: null, depends_on: ["screen", "api"], worker: sonnet }),
    ];
  }
  const stack = state === "stack" || state === "no-conductor";
  const decision = state === "decision";
  const review = (minutes: number) => ({ status: "review" as const, started_at_unix_ms: ago(minutes) });
  return [
    node("cause", `${no(1487)} 원인 조사`, { kind: "research", delivery: null, worker: opus("xhigh"), ...done(44, 24), line: "reopen 때 셀 초기화가 먼저 돎", ...finish(own(1, "원인을 재현 단계와 함께 적는다")) }),
    node("fix-1481", `이슈 ${no(1481)} 수정`, { worker: astra("high"), ...done(40, 14), pr: 1501, pr_state: "mergeable", landing: MERGE_LEFT, stage: 3, ...finish(own(1, "빈 목록에서도 필터가 남는다"), line("check", "check", "pass"), ci("pass")) }),
    node("fix-1490", `이슈 ${no(1490)} 수정`, {
      worker: astra("medium"), pr: 1505, stage: 3, change: `${no(1483)}은 ${no(1490)}과 같은 원인이라 합침`,
      ...(stack ? { ...done(40, 9), pr_state: "mergeable" as const, landing: MERGE_LEFT } : decision ? { status: "failed" as const, started_at_unix_ms: ago(30), pr_state: "failed" as const, attempts: 2, line: "CI 실패 · 시도 2/2" } : { ...review(30), pr_state: "pending" as const }),
      ...finish(own(1, "검색어를 지우면 처음 목록으로 돌아간다"), line("check", "check", "pass"), line("review", "review", stack ? "pass" : "open", { rule: "all", checkers: [checker(opus("high"), "pass", "agent-21"), { ...checker(sonnet, stack ? "pass" : "open", stack || decision ? undefined : "agent-22"), ...(stack || decision ? {} : { pane_id: REVIEW_PANE }) }] }), ci(decision ? "fail" : "pass")),
    }),
    node("api", "API 추가", { worker: astra("high"), ...done(40, 17), pr: 1503, pr_state: state === "no-conductor" ? "merged" : stack ? "mergeable" : "merged", landing: stack && state !== "no-conductor" ? MERGE_LEFT : MERGED, stage: 4, ...finish(own(1, "목록 API가 페이지를 나눠 준다"), ci("pass")) }),
    node("fix-1487", `${no(1487)} 수정`, {
      depends_on: ["cause"], worker: astra("medium"), pane_id: FIX_PANE, pr: 1506, stage: 3, reopened: stack ? 0 : 1,
      ...(stack ? { ...done(30, 12), pr_state: "mergeable" as const, landing: MERGE_LEFT } : { ...working(5), pr_state: "pending" as const, line: "replay 19/31" }),
      ...finish(own(1, "reopen 뒤에도 셀이 남는다"), own(2, "실제 세션 50개 replay", "skipped", { skip: { reason: "기록된 세션이 31개뿐" } }), ...(stack ? [] : [line("d1", "defect", "open", { text: "다시 열면 첫 셀만 비어 있다", found_by: "check-1487" })]), ci(stack ? "pass" : "open")),
    }),
    node("check-1487", `${no(1487)} 실제 앱 확인`, { kind: "verify", delivery: null, depends_on: ["fix-1487"], worker: sonnet, ...(stack ? done(16, 6) : {}), line: stack ? "5개 항목 통과" : null, ...finish(line("verify", "verify", stack ? "pass" : "open", { rule: "all", checkers: [checker(sonnet, stack ? "pass" : "open")] })) }),
    node("outside", "바깥: isolated 앱 실행기 정리", { kind: "external", delivery: null, external: { pr: 1232 }, status: decision ? "failed" : stack ? "done" : "waiting", pr: 1232, landing: stack ? OUTSIDE_LANDED : null }),
    decision
      ? node("screen", "API 쓰는 화면", { depends_on: ["api", "outside"], worker: astra("high"), status: "held", started_at_unix_ms: ago(21), pr: 1504, base_pr: 1503, pr_state: "pending", stage: 3, review: "리뷰 전 · CI 통과 뒤 시작", ...finish(own(1, "새 API로 목록을 그린다", "open"), ci("open")) })
      : node("screen", "API 쓰는 화면", { depends_on: ["api", "outside"], worker: astra("high"), pane_id: stack ? null : SCREEN_PANE, ...(stack ? done(33, 33) : working(12)), status: stack ? "done" : "working", pr: 1504, base_pr: 1503, pr_state: state === "no-conductor" ? "pending" : stack ? "mergeable" : "pending", stage: state === "no-conductor" ? 2 : 3, landing: stack ? MERGE_LEFT : null, line: state === "no-conductor" ? "옮기기 필요" : null, ...finish(own(1, "새 API로 목록을 그린다", stack ? "pass" : "open"), ci(stack ? "pass" : "open")) }),
    node("together", "함께 확인", { kind: "verify", delivery: null, depends_on: ["screen", "fix-1487"], worker: sonnet, ...(stack ? done(8, 4) : {}), line: stack ? "합친 변경 ✓" : null, ...finish(line("verify", "verify", stack ? "pass" : "open", { rule: "majority", checkers: [checker(sonnet, stack ? "pass" : "open"), checker(astra("medium"), stack ? "pass" : "open"), checker(opus("high"), stack ? "pass" : "open")] })) }),
  ];
}

/**
 * The rejected state's search fix (B32): its check failed with a short reason,
 * and of its two reviewers one rejected it, the one line it wrote the operator
 * before its long report, which folds.
 */
function rejectedReview(one: MissionNode): MissionNode {
  if (one.key !== "fix-1490") return one;
  const report = "Blocking: clearing the query restores the first page, but the scroll position and the selected row are lost. SearchList.tsx resets `selected` in the effect on `query` change (line 88), so the row the operator had picked is gone after clearing. Also the empty-query branch skips the debounce, which makes the list flash twice. Suggest keeping the selection keyed by id and moving the reset into the submit handler.";
  return {
    ...one,
    ...finish(
      own(1, "검색어를 지우면 처음 목록으로 돌아간다"),
      line("check", "check", "fail", { summary: "검색 테스트 2개 실패", evidence: "pnpm test: SearchList.test.tsx > keeps the selection after clearing (expected row 3, got none); restores the first page (timeout 5000 ms)" }),
      line("review", "review", "fail", { rule: "all", checkers: [{ ...checker(opus("high"), "fail", "agent-21"), summary: "검색어를 지우면 고른 줄과 스크롤 위치가 사라져요", evidence: report }, { ...checker(sonnet, "open", "agent-22"), pane_id: REVIEW_PANE }] }),
      ci("pass"),
    ),
  };
}

/** How many minutes before now each state's mission started; the rest started 44 minutes ago. */
const STARTED_AGO: Record<string, number> = { stack: 72, done: 300, cancelled: 300, "no-pr": 180 };

/**
 * The events a mission of these nodes would have recorded (PRD
 * mission-plan-visibility D-15, B18): its plan, each node's start, review
 * and end, its merges, the decision and the cancel.
 */
function history(mission: Mission, now: number): MissionEvent[] {
  const start = mission.started_at_unix_ms;
  if (start == null) return [];
  const events: MissionEvent[] = [{ at_unix_ms: start, kind: "planned", count: mission.nodes.length }];
  for (const one of mission.nodes) {
    if (one.started_at_unix_ms == null) continue;
    const begun = one.started_at_unix_ms;
    const end = one.finished_at_unix_ms ?? now;
    if (one.attempts >= 2) {
      const failedAt = begun + (end - begun) * 0.35;
      events.push({ at_unix_ms: begun, kind: "node", node: one.key, status: "working" });
      events.push({ at_unix_ms: failedAt, kind: "node", node: one.key, status: "failed", line: "CI 실패" });
      events.push({ at_unix_ms: failedAt + MINUTE, kind: "retried", node: one.key, count: 2 });
      events.push({ at_unix_ms: failedAt + MINUTE, kind: "node", node: one.key, status: "working" });
    } else events.push({ at_unix_ms: begun, kind: "node", node: one.key, status: "working" });
    if (one.pr != null) events.push({ at_unix_ms: begun + (end - begun) * 0.4, kind: "pr_opened", node: one.key, pr: one.pr });
    if (one.pr != null && one.finished_at_unix_ms != null) events.push({ at_unix_ms: begun + (end - begun) * 0.7, kind: "node", node: one.key, status: "review" });
    if (one.status === "review") events.push({ at_unix_ms: now - 6 * MINUTE, kind: "node", node: one.key, status: "review" });
    if (one.status === "held") events.push({ at_unix_ms: now - 8 * MINUTE, kind: "node", node: one.key, status: "held", line: one.line ?? undefined });
    if (one.status === "failed") events.push({ at_unix_ms: now - 4 * MINUTE, kind: "node", node: one.key, status: "failed", line: one.line ?? undefined });
    if (one.finished_at_unix_ms != null && (one.status === "done" || one.status === "waiting")) {
      events.push({ at_unix_ms: one.finished_at_unix_ms, kind: "node", node: one.key, status: one.status });
      if (one.pr_state === "merged") events.push({ at_unix_ms: one.finished_at_unix_ms, kind: "pr_merged", node: one.key, pr: one.pr! });
      if (one.commit) events.push({ at_unix_ms: one.finished_at_unix_ms, kind: "landed", node: one.key, line: one.commit });
    }
  }
  if (mission.question) events.push({ at_unix_ms: mission.question.asked_at_unix_ms, kind: "asked", node: mission.question.node ?? undefined, line: mission.decision ?? undefined, options: mission.question.options });
  if (mission.status === "cancelled" && mission.closed_at_unix_ms != null) events.push({ at_unix_ms: mission.closed_at_unix_ms, kind: "cancelled" });
  if (mission.report && mission.closed_at_unix_ms != null) events.push({ at_unix_ms: mission.closed_at_unix_ms, kind: "reported" });
  return events.sort((a, b) => a.at_unix_ms - b.at_unix_ms);
}

const OLD_CRITERION = "목록 화면이 새 API로 페이지를 넘기고, 필터를 바꿔도 처음 쪽으로 돌아가며, 빈 목록에서도 안내가 남는다";

export function missionFixture(shown: string, now: number): { mission: Mission; rest: SnapshotRest } {
  // An answer sent from the banner draws the decision with that answer on it.
  const answered = shown === "decision-sent" || shown === "decision-unreadable" || shown === "decision-unconfirmed";
  const state = answered ? "decision" : shown === "exempt" ? "done" : shown === "rejected" ? "running" : shown;
  // A mission from before finish lines keeps each node's one old sentence (B23); a rejected one has a reviewer say no.
  const nodes = shown === "exempt" ? plan(state, now).map((one) => ({ ...one, ...finish(own(1, OLD_CRITERION, "pass")) })) : shown === "rejected" ? plan(state, now).map(rejectedReview) : plan(state, now);
  const mission: Mission = {
    id: "mission-1", title: "이슈 정리", workspace_id: "hide", project: "/projects/herdr-ide", checkout_path: "/projects/herdr-ide", device_id: "local",
    status: state === "plan" ? "planning" : state === "stack" ? "merge_wait" : state === "done" || state === "no-pr" ? "done" : state === "cancelled" ? "cancelled" : "running",
    scope: "pr", cap: 4, exempt: shown === "exempt", strength: "normal", other_vendor: true,
    conductor: { pane_id: CONDUCTOR_PANE, name: "이슈 정리 · Conductor", agent_kind: "claude", session: { device_id: "local", agent: "claude", session_id: "scene-conductor" }, present: state !== "no-conductor", takes_answers: true },
    nodes, merge_order: ["api", "fix-1481", "fix-1490", "fix-1487", "screen"],
    // The order the core gives what is left: the merge order first, then the plan's (`mission_landing::landing_order`).
    landing_order: state === "wide" ? ["base"] : state === "stack" ? ["api", "fix-1481", "fix-1490", "fix-1487", "screen"] : state === "no-conductor" ? ["fix-1481", "fix-1490", "fix-1487", "screen"] : ["no-pr", "done", "cancelled", "plan"].includes(state) ? [] : ["fix-1481"],
    check: "cargo test && pnpm test", github: "octocat",
    decision: state === "decision" ? "화면 PR을 어디에 올릴까요?" : state === "no-conductor" ? `${no(1503)}이 머지되어 ${no(1504)}를 옮겨야 해요` : null,
    question: state === "decision"
      ? { key: 2, why: `API PR ${no(1503)}이 아직 리뷰 중이라 그 위에 쌓으면 리베이스가 생길 수 있어요`, options: [`${no(1503)} 위에 쌓기`, `${no(1503)} 머지 뒤 main에서`, "화면 PR을 둘로 나누기"], recommend: 1, node: "screen", asked_at_unix_ms: now - 8 * MINUTE }
      : null,
    answer: answered ? { key: 2, text: `${no(1503)} 위에 쌓기`, option: 1, state: shown === "decision-unconfirmed" ? "unconfirmed" : "waiting" } : null,
    answer_failed: shown === "decision-unreadable" ? "session_unreadable" : null,
    delay: null, report: state === "done" ? REPORT : state === "no-pr" ? "1.0.0 릴리스 끝. private 배포, 사이트 반영, floor 올림." : null,
    created_at_unix_ms: now - 320 * MINUTE,
    started_at_unix_ms: state === "plan" ? null : now - (STARTED_AGO[state] ?? 44) * MINUTE,
    closed_at_unix_ms: state === "stack" ? now : state === "done" ? now - 30 * MINUTE : state === "cancelled" ? now - 60 * MINUTE : state === "no-pr" ? now - 20 * MINUTE : null,
    running: state === "decision" ? 1 : state === "running" || state === "wide" ? 4 : 0,
    // A device that disconnected leaves its last values, dimmed (B20).
    stale: state === "stale", github_stale: state === "stale", github_read_at_unix_ms: state === "stale" ? now - 25 * MINUTE : now,
  };
  const rest = {
    missions: state === "failed" ? { items: [], failed: "unreadable" } : { items: [mission] },
    mission_events: { mission: mission.id, events: history(mission, now), dropped: 0 },
    navigator: { devices: [{ id: "local", kind: "local", label: "This Mac", kit: { agents: [
      { id: "claude-code", label: "Claude Code", availability: "available", enabled: true },
      { id: "codex", label: "Codex", availability: "available", enabled: true },
      { id: "pi", label: "Pi", availability: "available", enabled: true },
      { id: "grok", label: "Grok", availability: "not_installed", enabled: false },
    ] } }], workspaces: [] },
    overview_graph: { available: true, limit: 2000, retained_count: 0, truncated: false, retention_ms: 86400000, letters: [], sessions: [{ device_id: "local", pane_id: CONDUCTOR_PANE, session_digest: "scene-conductor", launch_model: "Opus 5.5", launch_effort: "high" }, { device_id: "local", pane_id: SCREEN_PANE, session_digest: "scene-screen", launch_model: "Sol", launch_effort: null }, { device_id: "local", pane_id: REVIEW_PANE, session_digest: "scene-review", launch_model: "Sonnet 5.5", launch_effort: "medium" }] },
    status: { remote: [], background_ai: { enabled: true, providers: [
      { agent: "claude-code", state: "ready", models: ["Opus 5.5", "Sonnet 5.5", "Haiku 5.5"] },
      { agent: "codex", state: "ready", models: ["GPT-6-Astra", "GPT-5.6-Sol"] },
    ] } },
    ui_state: { conductor_settings: { concurrency: 4, default_scope: "pr", candidates: [
      { agent: "claude", model: "Opus 5.5", effort: null, purpose: "화면·설계가 섞인 작업" },
      { agent: "codex", model: "GPT-6-Astra", effort: null, purpose: "구현, 테스트 고치기" },
      { agent: "claude", model: "Sonnet 5.5", effort: "low", purpose: "리뷰, 문서" },
      { agent: "pi", effort: null, purpose: "짧은 조사" },
      { agent: "grok", effort: null, purpose: "" },
    ], default_strength: "normal", other_vendor: true } },
  } as unknown as SnapshotRest;
  return { mission, rest };
}

export function MissionScene({ params }: { params: URLSearchParams }) {
  useFollowInterfaceLanguage(requireInterfaceLanguage(params.get("lang") ?? "ko"));
  const theme = params.get("theme") === "light" ? "light" : "dark";
  const state = params.get("state") ?? "running";
  const pane = PANE_STATES[state];
  const fixture = useMemo(() => missionFixture(pane ?? state, Date.now()), [pane, state]);
  const actions = useMemo(() => createActions(() => true), []);
  const pick = params.get("pick");
  useLayoutEffect(() => {
    // The decision state's fix worker is one the core raised: its own line says why it stopped.
    const raised = { pane_id: FIX_PANE, device_id: "local", escalation: { cause: "child_blocked", letter_id: null, since_unix_ms: Date.now() - 3 * MINUTE, human_notice: true }, request: { verb: "blocked", verb_since_unix_ms: Date.now() - 3 * MINUTE, line: "로그인이 만료되어 테스트를 못 돌림", end: "blocked", request: null, later_by: null, pull_requests: [] } };
    // The Conductor's own row: at work while its mission runs, asleep once it closed, none when no Conductor holds it.
    const closed = ["done", "cancelled", "no-pr", "exempt"].includes(state);
    const conductor = { pane_id: CONDUCTOR_PANE, device_id: "local", mark: closed ? "asleep" : state === "decision" ? "quiet" : "going" };
    const agents = (pane ? [{ pane_id: CONDUCTOR_PANE, device_id: "local" }] : state === "no-conductor" || state === "failed" ? [] : state === "decision" ? [conductor, raised] : [conductor]) as unknown as AgentRow[];
    useShellStore.setState({ rest: fixture.rest, agents, connection: "live" });
    // The popover pinned, as a press, the Show plan chord or a first plan watched leaves it.
    useUiStore.setState({ planPane: pane ? CONDUCTOR_PANE : null });
    document.documentElement.classList.toggle("dark", theme === "dark");
    document.documentElement.classList.toggle("light", theme === "light");
    document.documentElement.style.setProperty("--interface-scale", params.get("scale") ?? "1");
  }, [fixture, pane, state, theme, params]);
  // A picked node is a press on its card, as the operator makes it, once the layout drew the card.
  useEffect(() => {
    if (!pick) return;
    let frame = 0;
    let left = 120;
    const press = () => {
      const card = document.querySelector<HTMLElement>(`[data-mission-node="${pick}"]`);
      if (card) card.click();
      else if ((left -= 1) > 0) frame = requestAnimationFrame(press);
    };
    frame = requestAnimationFrame(press);
    return () => cancelAnimationFrame(frame);
  }, [pick, fixture]);
  const width = Number(params.get("width") ?? (state === "settings" ? 900 : 1080));
  return (
    <TooltipProvider>
      <div style={{ width }} className="flex h-full min-w-0 flex-col bg-background text-foreground" data-gallery-scene="mission" data-mission-scene={state}>
        {state === "settings" ? (
          <div className="p-xl"><ConductorTab actions={actions} /></div>
        ) : pane ? (
          <div className="flex h-(--size-pane-header) min-w-0 items-center gap-xs border-b border-border bg-muted px-sm text-caption" data-mission-scene-pane="true">
            <AgentMark kind="claude" />
            <span className="truncate">Conductor</span>
            <PaneWorkChip paneId={CONDUCTOR_PANE} actions={actions} />
          </div>
        ) : (
          <MissionMap missionId="mission-1" actions={actions} />
        )}
      </div>
    </TooltipProvider>
  );
}
