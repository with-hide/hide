// The real mission page and Settings › Mission over invented values (PRD
// software-factory-conductor, mission-map-git-criteria), one state per
// `state` parameter: plan, running (every card look: working, review with its
// checkers, done with a merge left, in main, outside, reopened by a defect),
// rejected (running, with the search fix's check failed and one reviewer's
// rejection, its short reason first), decision (a decision, a failed node whose worker ended, raised to the
// operator with its PR's CI failure beside the choices, a stuck worker and an outside PR closed unmerged in the
// operator's turn; its choices radio rows, one long enough to wrap, the recommended one tagged), decision-views (a decision about
// screens with the Views linked to it to look at: one seen, one not looked at, one closed that opens again), decision-sent (its answer waiting
// for the Conductor's turn; `decision-hold-working`, `-typing`, `-recent`, `-approval` and `-forced` say what it waits for, the first three with
// Send now), decision-unreadable (its answer waiting on a
// Conductor session Hide cannot read), decision-unconfirmed (its answer typed without
// the engine confirming it), stack (the merge order), no-conductor, stale, failed
// (the record unreadable), wide (a row wider than the slot, shrunk to fit),
// done, cancelled, no-pr (a finished mission with no pull request), exempt
// (a finished mission from before finish lines, its old sentence on each
// card beside the worker), settings.
// Every map state draws its Conductor row (asleep once the mission closed,
// none for no-conductor, which offers 이어받기 there), the summary beside the
// map while no node is picked, and the checkers as satellites beside their
// cards; done leads with its result tiles and pull requests (PRD mission-tool
// B12-B21). `hint=role` focuses the first checker beside its card so its role hint is open. `pick=<node>` opens that node's panel, `width` narrows the slot
// and `scale` sets the text size. The
// Conductor pane's head with its mission chip and pinned popover (PRD
// mission-pane-chip, mission-plan-visibility) is `pane-running`,
// `pane-decision`, `pane-first-plan`, `pane-done` and `pane-cancelled`.
// Nothing leaves this scene.
import { useEffect, useLayoutEffect, useMemo } from "react";
import { AgentMark } from "../AgentMark";
import { createActions } from "../actions";
import { TooltipProvider } from "../components/ui/tooltip";
import { useInterfaceTranslation } from "../i18n/client";
import { useFollowInterfaceLanguage } from "../i18n/translator";
import { requireInterfaceLanguage } from "../i18n/locale";
import { MissionScreen } from "../MissionScreen";
import { PaneWorkChip } from "../PaneMissionChip";
import { ConductorTab } from "../settings/ConductorTab";
import type { AgentRow, Device, Mission, MissionAnswerHold, MissionChecker, MissionCriterion, MissionEvent, MissionGate, MissionGateState, MissionLanding, MissionMapFlow, MissionNode, MissionWorker, PlanView, SnapshotRest } from "../snapshot";
import { useShellStore } from "../store";
import { MISSION_ENTRY, useUiStore } from "../ui";
import { withCoreWaits } from "./missionWaits";

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
  return { key, title, kind: "change", depends_on: [], status: "planned", blocked: null, delivery: "pr", worker: null, agent_id: null, pane_id: null, machine: null, pr: null, base_pr: null, branch: null, pr_state: null, pr_url: null, stage: null, attempts: 0, line: null, change: null, review: null, criteria: [], criteria_met: 0, criteria_skipped: 0, steps: [], brief_bytes: null, external: null, unread: false, reopened: 0, started_at_unix_ms: null, finished_at_unix_ms: null, commit: null, landing: null, verdict: null, checks: null, attention: null, hide: null, ...extra, prs: extra.prs ?? (extra.pr != null ? [{ number: extra.pr, state: extra.pr_state ?? null, url: extra.pr_url ?? null }] : []) };
}

/** Where a node is on its way to main, as the core would publish it (`hide-core/src/runtime/mission_landing.rs`). */
const MERGE_LEFT: MissionLanding = { state: "left", action: "merge" };
const MERGED: MissionLanding = { state: "landed", proof: { by: "merged" } };
const OUTSIDE_LANDED: MissionLanding = { state: "landed", proof: { by: "outside" } };
const inMain = (commit: string): MissionLanding => ({ state: "landed", proof: { by: "in_main", commit } });

const opus = (effort: string | null) => ({ agent: "claude", model: "Opus", effort, effort_auto: true });
const astra = (effort: string | null) => ({ agent: "codex", model: "Astra", effort, effort_auto: true });
const sonnet = { agent: "claude", model: "Sonnet", effort: "low", effort_auto: false };
/** A worker's own plan as the core keeps it for its node: `done` steps finished, the next one at work. */
function workerPlan(session: string, titles: string[], done: number): PlanView {
  return {
    pane_id: "", session: { device_id: "mac", agent: "codex", session_id: session },
    steps: titles.map((title, index) => ({ key: `s${index}`, title, depends_on: index > 0 ? [`s${index - 1}`] : [], status: index < done ? "done" as const : index === done ? "working" as const : "planned" as const, line: null, output: index === titles.length - 1 ? "pr" as const : null, changed: false, change: null, waiting: index > done, delegate: false, evidence: null, delegation: null })),
    review: "approved", ask: null,
    progress: { current: done < titles.length ? `s${done}` : null, next: done + 1 < titles.length ? `s${done + 1}` : null, done, total: titles.length, finished: done === titles.length },
    created_at_unix_ms: 1, updated_at_unix_ms: 2,
  };
}

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
        node(`part-${index}`, `${title} 화면`, { depends_on: ["base-check"], worker: opus("high"), ...(index < 3 ? working(12 - index * 3) : {}), branch: index < 3 ? `feat/mission-part-${index + 1}-screen-layout` : null, stage: index < 3 ? 1 : null }),
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
  const review = (minutes: number) => ({ status: "working" as const, started_at_unix_ms: ago(minutes) });
  return [
    node("cause", `${no(1487)} 원인 조사`, { kind: "research", delivery: null, worker: opus("xhigh"), ...done(44, 24), line: "reopen 때 셀 초기화가 먼저 돎", ...finish(own(1, "원인을 재현 단계와 함께 적는다")) }),
    node("fix-1481", `이슈 ${no(1481)} 수정`, { worker: astra("high"), agent_id: "agent-1481", plan: workerPlan("worker-1481", ["빈 목록 재현", "필터 상태 보존", "구현 PR"], 3), launched: { agent_id: "agent-1481", agent: "codex", model: "Astra 2", effort: "high" }, ...done(40, 14), pr: 1501, pr_state: "mergeable", landing: MERGE_LEFT, stage: 3, hide: { action: "done", at_unix_ms: ago(26) }, ...finish(own(1, "빈 목록에서도 필터가 남는다"), line("check", "check", "pass"), ci("pass")) }),
    node("fix-1490", `이슈 ${no(1490)} 수정`, {
      worker: astra("medium"), pr: 1505, pr_url: "https://github.com/with-hide/hide/pull/1505", stage: 3, change: `${no(1483)}은 ${no(1490)}과 같은 원인이라 합침`,
      ...(stack ? { ...done(40, 9), pr_state: "mergeable" as const, landing: MERGE_LEFT } : decision ? { status: "working" as const, started_at_unix_ms: ago(30), pr_state: "failed" as const, attempts: 2, line: "CI 실패 · 시도 2/2", hide: { action: "woke" as const, pr: 1505, why: "ci_failed" as const, at_unix_ms: ago(3) }, agent_runtime: "ended" as const, attention: { reason: "worker_ended" as const, pr: null, to: "operator" as const, since_unix_ms: ago(2), on_row: false, item: false, choices: ["leave" as const, "drop" as const] } } : { ...review(30), pr_state: "pending" as const }),
      ...finish(own(1, "검색어를 지우면 처음 목록으로 돌아간다"), line("check", "check", "pass"), line("review", "review", stack ? "pass" : "open", { rule: "all", tally: { passed: stack ? 2 : 1, total: 2 }, checkers: [checker(opus("high"), "pass", "agent-21"), { ...checker(sonnet, stack ? "pass" : "open", stack || decision ? undefined : "agent-22"), ...(stack || decision ? {} : { pane_id: REVIEW_PANE }) }] }), ci(decision ? "fail" : "pass")),
    }),
    node("api", "API 추가", { worker: astra("high"), ...done(40, 17), pr: 1503, pr_state: state === "no-conductor" ? "merged" : stack ? "mergeable" : "merged", landing: stack && state !== "no-conductor" ? MERGE_LEFT : MERGED, stage: 4, ...finish(own(1, "목록 API가 페이지를 나눠 준다"), ci("pass")),
      // Merged with a line left unrecorded and no Conductor to ask: it waits on the operator (PRD mission-node-simple B13, B14).
      ...(state === "no-conductor" ? { status: "working" as const, finished_at_unix_ms: null, landing: null, criteria_met: 0, criteria: [own(1, "목록 API가 페이지를 나눠 준다", "open"), ci("pass")], attention: { reason: "merged_unproven" as const, pr: 1503, to: "operator" as const, since_unix_ms: ago(6), on_row: false, item: true, choices: ["done" as const] } } : {}) }),
    node("fix-1487", `${no(1487)} 수정`, {
      depends_on: ["cause"], worker: astra("medium"), pane_id: FIX_PANE, pr: 1506, pr_url: "https://github.com/with-hide/hide/pull/1506", stage: 3, reopened: stack ? 0 : 1, branch: "fix/1487-reopen-cells",
      plan: workerPlan("worker-1487", ["reopen 재현 테스트", "셀 초기화 순서 고치기", "실제 세션 replay", "전체 검사", "구현 PR"], stack ? 5 : 2),
      ...(stack ? { ...done(30, 12), pr_state: "mergeable" as const, landing: MERGE_LEFT } : { ...working(5), pr_state: "pending" as const, line: "replay 19/31" }),
      ...finish(own(1, "reopen 뒤에도 셀이 남는다"), own(2, "실제 세션 50개 replay", "skipped", { skip: { reason: "기록된 세션이 31개뿐" } }), ...(stack ? [] : [line("d1", "defect", "open", { text: "다시 열면 첫 셀만 비어 있다", found_by: "check-1487" })]), ci(stack ? "pass" : "open")),
    }),
    node("check-1487", `${no(1487)} 실제 앱 확인`, { kind: "verify", delivery: null, depends_on: ["fix-1487"], worker: sonnet, ...(stack ? done(16, 6) : {}), line: stack ? "5개 항목 통과" : null, ...finish(line("verify", "verify", stack ? "pass" : "open", { rule: "all", checkers: [checker(sonnet, stack ? "pass" : "open")] })) }),
    node("outside", "바깥: isolated 앱 실행기 정리", { kind: "external", delivery: null, external: { pr: 1232 }, status: decision ? "working" : stack ? "done" : "planned", pr: 1232, landing: stack ? OUTSIDE_LANDED : null, attention: decision ? { reason: "pr_closed", pr: 1232, to: "conductor", since_unix_ms: ago(4), on_row: false, item: false, choices: ["leave" as const, "drop" as const] } : null }),
    decision
      ? node("screen", "API 쓰는 화면", { depends_on: ["api", "outside"], worker: astra("high"), status: "working", started_at_unix_ms: ago(21), pr: 1504, base_pr: 1503, pr_state: "pending", stage: 3, review: "리뷰 전 · CI 통과 뒤 시작", ...finish(own(1, "새 API로 목록을 그린다", "open"), ci("open")) })
      : node("screen", "API 쓰는 화면", { depends_on: ["api", "outside"], worker: astra("high"), agent_id: "agent-screen", launched: { agent_id: "agent-screen", agent: "codex", model: "Sol", effort: null }, pane_id: stack ? null : SCREEN_PANE, ...(stack ? done(33, 33) : working(12)), status: stack ? "done" : "working", pr: 1504, base_pr: 1503, pr_state: state === "no-conductor" ? "pending" : stack ? "mergeable" : "pending", stage: state === "no-conductor" ? 2 : 3, landing: stack ? MERGE_LEFT : null, line: state === "no-conductor" ? "옮기기 필요" : null, ...finish(own(1, "새 API로 목록을 그린다", stack ? "pass" : "open"), ci(stack ? "pass" : "open")) }),
    node("together", "함께 확인", { kind: "verify", delivery: null, depends_on: ["screen", "fix-1487"], worker: sonnet, ...(stack ? done(8, 4) : {}), line: stack ? "합친 변경 ✓" : null, ...finish(line("verify", "verify", stack ? "pass" : "open", { rule: "majority", tally: { passed: stack ? 3 : 0, total: 3 }, checkers: [checker(sonnet, stack ? "pass" : "open"), checker(astra("medium"), stack ? "pass" : "open"), checker(opus("high"), stack ? "pass" : "open")] })) }),
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

/**
 * The warned state (hide-private issue 552): the rejected search fix
 * failed its review twice in a row, and the reopen fix went ninety-five
 * minutes with no push; Hide told the Conductor about each.
 */
function warned(one: MissionNode, now: number): MissionNode {
  if (one.key === "fix-1490") return { ...one, review_fails: 2, warnings: [{ kind: "review_fails", from_unix_ms: now - 12 * MINUTE, since_unix_ms: now - 12 * MINUTE, told_unix_ms: now - 12 * MINUTE }] };
  if (one.key === "fix-1487") return { ...one, pushed_at_unix_ms: now - 95 * MINUTE, warnings: [{ kind: "no_push", from_unix_ms: now - 95 * MINUTE, since_unix_ms: now - 5 * MINUTE, told_unix_ms: now - 5 * MINUTE }] };
  return one;
}

/**
 * The stalled state (hide-private issues 678-680): eleven letters wait for
 * the Conductor behind a draft in its input, the search fix's worker rests
 * while its review is open, the screen's worker on the Mac mini rests on its
 * CI, the API change has no worker left, and the reopen fix's worker works.
 */
function stalled(one: MissionNode): MissionNode {
  switch (one.key) {
    case "fix-1490":
      return { ...one, work: { state: "waiting", on: { kind: "review" } } };
    case "screen":
      return { ...one, pane_id: "remote:mini:pane:h15:p3", work: { state: "waiting", on: { kind: "ci" } } };
    case "fix-1487":
      return { ...one, work: { state: "working" } };
    case "api":
      return { ...one, status: "working", finished_at_unix_ms: null, landing: null, work: { state: "no_worker" } };
    default:
      return one;
  }
}

/**
 * The steps a Conductor wrote with the plan, by node (hide-private missions.md):
 * a node ahead of its worker shows them as dashed cells, a node whose worker has
 * a plan shows that plan, and the reopen fix's worker changed one of its steps.
 */
const PLANNED_STEPS: Record<string, string[]> = {
  cause: ["재현 단계 찾기", "초기화 순서 읽기", "원인을 줄 번호와 함께 적기"],
  "fix-1481": ["빈 목록 재현", "필터 상태 보존", "구현 PR"],
  "fix-1483": ["재현 테스트", "원인 고치기", "구현 PR"],
  "fix-1490": ["검색 입력 흐름 읽기", "지우면 처음 목록으로", "테스트와 구현 PR"],
  api: ["페이지 나누기 설계", "API 구현", "테스트와 구현 PR"],
  "fix-1487": ["reopen 재현 테스트", "셀 초기화 순서 고치기", "실제 세션 50개 replay", "전체 검사", "구현 PR"],
  screen: ["새 API 연결", "목록·검색·필터 옮기기", "화면 확인과 구현 PR"],
  "check-1487": ["실제 앱에서 reopen 확인", "결과 기록"],
  together: ["합친 변경 로드 확인", "두 화면 같이 확인"],
};

/** The brief the reopen fix's worker was started on, invented for the boards. */
const BRIEF = `${no(1487)} fix 노드입니다. reopen 뒤에 셀이 비는 문제를 고칩니다.\n\n브랜치 fix/1487-reopen-cells, 워크트리에서만 커밋하세요.\n원인 노드 결론: 셀 초기화가 reopen 보다 먼저 돎.\n\n할 일\n1. reopen 재현 테스트\n2. 셀 초기화 순서 고치기\n3. 실제 세션 50개 replay\n4. 전체 검사\n5. 구현 PR\n\n끝나면 report 편지로 PR 번호와 근거를 보내세요.`;

/** Gives each node the Conductor's steps, and the size of its brief once a worker was bound. */
function withSteps(nodes: MissionNode[]): MissionNode[] {
  return nodes.map((one) => {
    const steps = PLANNED_STEPS[one.key];
    if (!steps || one.kind === "external") return one;
    // The worker of the reopen fix changed its third step from the plan.
    return { ...one, steps, brief_bytes: one.agent_id || one.pane_id ? BRIEF.length : null };
  });
}

/** How many minutes before now each state's mission started; the rest started 44 minutes ago. */
const STARTED_AGO: Record<string, number> = { stack: 72, done: 300, cancelled: 300, "no-pr": 180 };

/**
 * The events a mission of these nodes would have recorded (PRD
 * mission-plan-visibility D-15, B18): its plan, each node's start and end,
 * its merges, what Hide did and raised, the decision and the cancel.
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
      events.push({ at_unix_ms: failedAt + MINUTE, kind: "retried", node: one.key, count: 2 });
      events.push({ at_unix_ms: failedAt + MINUTE, kind: "node", node: one.key, status: "working" });
    } else events.push({ at_unix_ms: begun, kind: "node", node: one.key, status: "working" });
    if (one.pr != null) events.push({ at_unix_ms: begun + (end - begun) * 0.4, kind: "pr_opened", node: one.key, pr: one.pr });
    if (one.hide) events.push({ at_unix_ms: one.hide.at_unix_ms, kind: "hide", node: one.key, hide: one.hide.action, ...(one.hide.pr != null ? { pr: one.hide.pr } : {}), ...(one.hide.why ? { why: one.hide.why } : {}) });
    if (one.attention) events.push({ at_unix_ms: one.attention.since_unix_ms, kind: one.attention.to === "operator" ? "raised_to_operator" : "raised", node: one.key, reason: one.attention.reason, ...(one.attention.pr != null ? { pr: one.attention.pr } : {}) });
    if (one.finished_at_unix_ms != null && one.status === "done") {
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

/**
 * A node's gate as the core draws it (PRD mission-human-gate): its `human`
 * finish line, tagged `볼 것`, and the derived view the map reads.
 */
function gated(one: MissionNode, state: MissionGateState, look: string, extra: Partial<MissionGate> = {}): void {
  const gate: MissionGate = { state, look, ask: null, since_unix_ms: null, long_wait_at_unix_ms: null, via: null, said: null, ...extra };
  const line: MissionCriterion = { key: "human", kind: "human", text: look, state: state === "passed" ? "pass" : "open", ...(gate.via ? { via: gate.via, evidence: gate.said ?? undefined } : {}), ...(gate.ask != null ? { asked: gate.ask } : {}) };
  one.criteria = [...one.criteria.filter((criterion) => criterion.kind !== "human"), line];
  one.criteria_met = one.criteria.filter((criterion) => criterion.state === "pass" || criterion.state === "skipped").length;
  one.gate = gate;
}

const OLD_CRITERION = "목록 화면이 새 API로 페이지를 넘기고, 필터를 바꿔도 처음 쪽으로 돌아가며, 빈 목록에서도 안내가 남는다";

export function missionFixture(shown: string, now: number): { mission: Mission; rest: SnapshotRest } {
  // `close-refused`: the no-Conductor mission with every node's work finished, whose close Hide could not make because three nodes' check lines are empty.
  if (shown === "close-refused") return closeRefused(missionFixture("no-conductor", now), now);
  // An answer sent from the banner draws the decision with that answer on it.
  const held = shown.startsWith("decision-hold-") ? (shown.slice("decision-hold-".length) as MissionAnswerHold) : undefined;
  const answered = shown === "decision-sent" || shown === "decision-unreadable" || shown === "decision-unconfirmed" || held !== undefined;
  // A decision about screens to look at, with the Views linked to it.
  const looking = shown === "decision-views";
  // `gate-plan`: the plan before approval names where the operator looks; `gate-tobe`: a reached gate is the decision the Conductor asks;
  // `gate-all`: planned gates passed and ahead beside an unplanned decision; `gate-nocond`: a reached gate no Conductor holds, waiting 30 hours.
  const gating = shown === "gate-tobe";
  const adhoc = shown === "gate-all";
  // The stalled state (hide-private issues 678-680) runs a worker on the Mac mini.
  const remote = shown === "remote" || shown === "remote-long" || shown === "stalled";
  const state = answered || looking ? "decision" : shown === "gate-plan" ? "plan" : shown === "gate-nocond" ? "no-conductor" : shown === "exempt" ? "done" : shown === "rejected" || shown === "warned" || gating || adhoc || remote ? "running" : shown;
  // A mission from before finish lines keeps each node's one old sentence (B23); a rejected one has a reviewer say no.
  const nodes = shown === "exempt" ? plan(state, now).map((one) => ({ ...one, ...finish(own(1, OLD_CRITERION, "pass")) })) : shown === "rejected" ? plan(state, now).map(rejectedReview) : shown === "warned" ? plan(state, now).map((one) => warned(rejectedReview(one), now)) : plan(state, now);
  nodes.splice(0, nodes.length, ...withSteps(nodes).map((one) => (shown === "stalled" ? stalled(one) : one)));
  const at = (key: string) => nodes.find((one) => one.key === key)!;
  if (shown === "gate-plan") {
    gated(at("cause"), "ahead", "원인 결론");
    gated(at("screen"), "ahead", "화면 시안 2장");
    gated(at("together"), "ahead", "머지 전 합친 화면");
  }
  if (gating || adhoc) {
    const screen = at("screen");
    Object.assign(screen, { status: "working" as const, pane_id: null, pr_state: "mergeable" as const, line: "시안 2장 완성", landing: null });
    gated(at("cause"), "passed", "원인 결론", { via: "pane", said: "ㄱㄱ 좋아" });
    if (gating) gated(screen, "open", "화면 시안 2장", { ask: 4, since_unix_ms: now - 3 * MINUTE, long_wait_at_unix_ms: now - 3 * MINUTE + 24 * 60 * MINUTE });
    gated(at("together"), "ahead", "머지 전 합친 화면");
    if (adhoc) gated(at("fix-1487"), "ahead", "실제 앱에서 reopen 화면");
  }
  if (shown === "gate-nocond") {
    // No Conductor holds the mission and the node's worker has no row: the reached gate is the mission's own Needs You item.
    const since = now - 30 * 60 * MINUTE;
    const screen = at("screen");
    Object.assign(screen, { status: "working" as const, line: "시안 2장 완성", pr_state: "mergeable" as const, attention: { reason: "human_gate" as const, pr: null, to: "operator" as const, since_unix_ms: since, on_row: false, item: true, choices: [] } });
    screen.criteria = screen.criteria.map((criterion) => ({ ...criterion, state: "pass" as const }));
    gated(screen, "open", "화면 시안 2장", { since_unix_ms: since, long_wait_at_unix_ms: since + 24 * 60 * MINUTE });
    gated(at("cause"), "passed", "원인 결론", { via: "card", said: "계속 진행" });
    gated(at("together"), "ahead", "합친 화면");
    for (const one of nodes) if (one.attention?.reason !== "human_gate") one.attention = null;
  }
  if (remote) {
    for (const one of nodes) {
      one.machine = one.key === "screen" || (shown === "remote-long" && one.key === "fix-1487") ? "mini" : one.pane_id ? "local" : null;
      for (const check of one.criteria.flatMap((criterion) => criterion.checkers ?? [])) {
        if (check.pane_id) check.machine = "mini";
      }
    }
  }
  const mission: Mission = withCoreWaits({
    id: "mission-1", title: "이슈 정리", workspace_id: "hide", project: "/projects/herdr-ide", checkout_path: "/projects/herdr-ide", device_id: "local",
    status: state === "plan" ? "planning" : state === "stack" ? "merge_wait" : state === "done" || state === "no-pr" ? "done" : state === "cancelled" ? "cancelled" : "running",
    scope: "pr", cap: 4, exempt: shown === "exempt", strength: "normal",
    conductor: { pane_id: CONDUCTOR_PANE, name: "이슈 정리 · Conductor", agent_kind: "claude", session: { device_id: "local", agent: "claude", session_id: "scene-conductor" }, present: state !== "no-conductor", takes_answers: true },
    nodes, merge_order: ["api", "fix-1481", "fix-1490", "fix-1487", "screen"],
    // The order the core gives what is left: the merge order first, then the plan's (`mission_landing::landing_order`).
    landing_order: state === "wide" ? ["base"] : state === "stack" ? ["api", "fix-1481", "fix-1490", "fix-1487", "screen"] : state === "no-conductor" ? ["fix-1481", "fix-1490", "fix-1487", "screen"] : ["no-pr", "done", "cancelled", "plan"].includes(state) ? [] : ["fix-1481"],
    // The screen's pull request stacks on the API's until that one merges.
    stack_waiting: ["plan", "wide", "no-pr", "done"].includes(state) ? 0 : 1,
    check: "cargo test && pnpm test", github: "octocat",
    decision: gating ? "화면 시안 2장 확인하고 알려 주세요" : adhoc ? "화면 PR을 어디에 올릴까요?" : shown === "gate-nocond" ? null : looking ? "화면 PR: 고치기 전·후 화면 12장을 보고 머지해도 될까요?" : state === "decision" ? "화면 PR을 어디에 올릴까요?" : state === "no-conductor" ? `${no(1503)}이 머지되어 ${no(1504)}를 옮겨야 해요` : null,
    question: gating
      ? { key: 4, why: null, options: ["계속 진행", "고칠 점 있음"], recommend: null, node: "screen", asked_at_unix_ms: now - 3 * MINUTE }
      : adhoc
      ? { key: 5, why: `API PR ${no(1503)}이 아직 리뷰 중이라 그 위에 쌓으면 리베이스가 생길 수 있어요`, options: [`${no(1503)} 위에 쌓기`, `${no(1503)} 머지 뒤 main에서`], recommend: 1, node: "screen", asked_at_unix_ms: now - 2 * MINUTE }
      : looking
      ? { key: 3, why: "화면이 바뀌는 PR이라 머지 전에 직접 봐야 해요", options: ["머지해도 됨", "고칠 점 있음"], recommend: null, node: "screen", asked_at_unix_ms: now - 8 * MINUTE }
      : state === "decision"
      ? { key: 2, why: `API PR ${no(1503)}이 아직 리뷰 중이라 그 위에 쌓으면 리베이스가 생길 수 있어요`, options: [`${no(1503)} 위에 쌓기`, `${no(1503)} 머지 뒤 main에서`, "화면 PR을 둘로 나누기: 목록 화면은 지금 올리고, 검색 화면은 API가 머지된 뒤 따로 올려요"], recommend: 1, node: "screen", asked_at_unix_ms: now - 8 * MINUTE }
      : null,
    answer: answered ? { key: 2, text: `${no(1503)} 위에 쌓기`, option: 1, state: shown === "decision-unconfirmed" ? "unconfirmed" : "waiting", ...(held ? { hold: held } : {}) } : null,
    answer_failed: shown === "decision-unreadable" ? "session_unreadable" : null,
    decision_views: gating ? [
      { device_id: "local", checkout_path: "/repo/hide.worktrees/screen", display_id: "d7", view: "v7", kind: "file" as const, label: "시안 A · 목록 화면.png", unseen: true, by: "worker" as const },
      { device_id: "local", checkout_path: "/repo/hide.worktrees/screen", display_id: "d8", view: "v8", kind: "file" as const, label: "시안 B · 검색 화면.png", unseen: true, by: "worker" as const },
    ] : looking ? [
      { device_id: "local", checkout_path: "/repo/hide.worktrees/screen", display_id: "d7", view: "v7", kind: "browser" as const, label: "전후 비교 · screens.html", by: "conductor" as const },
      { device_id: "local", checkout_path: "/repo/hide.worktrees/screen", display_id: "d8", view: "v8", kind: "file" as const, label: "리뷰 요약.md", unseen: true, by: "worker" as const },
      { device_id: "local", checkout_path: "/repo/hide.worktrees/screen", display_id: "d9", view: "v9", kind: "file" as const, label: "이전 시안.png", closed: true, by: "worker" as const },
    ] : undefined,
    delay: null, report: state === "done" ? REPORT : state === "no-pr" ? "1.0.0 릴리스 끝. private 배포, 사이트 반영, floor 올림." : null,
    created_at_unix_ms: now - 320 * MINUTE,
    started_at_unix_ms: state === "plan" ? null : now - (STARTED_AGO[state] ?? 44) * MINUTE,
    closed_at_unix_ms: state === "stack" ? now : state === "done" ? now - 30 * MINUTE : state === "cancelled" ? now - 60 * MINUTE : state === "no-pr" ? now - 20 * MINUTE : null,
    running: state === "decision" ? 1 : state === "running" || state === "wide" ? 4 : 0,
    // A device that disconnected leaves its last values, dimmed (B20).
    stale: state === "stale", github_stale: state === "stale", github_read_at_unix_ms: state === "stale" ? now - 25 * MINUTE : now,
    conductor_letters: shown === "stalled" ? { count: 11, reason: "draft", since_unix_ms: now - 40 * MINUTE } : null,
  });
  const rest = {
    missions: state === "failed" ? { items: [], failed: "unreadable" } : { items: [mission] },
    mission_events: { mission: mission.id, events: history(mission, now), dropped: 0 },
    mission_brief: { mission: mission.id, node: "fix-1487", text: BRIEF },
    navigator: { siblings_shown: 5, devices: [{ id: "local", kind: "local", label: remote ? "Studio Mac" : "This Mac", kit: { agents: [
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
    ], default_strength: "normal", clean_merged: true } },
  } as unknown as SnapshotRest;
  if (remote) {
    rest.navigator!.devices!.push({ id: "mini", kind: "remote", label: shown === "remote-long" ? "맥 미니 · 빌드와 검증을 맡는 작업실의 데스크톱" : "Mac mini" } as Device);
    for (const session of rest.overview_graph!.sessions) {
      if (session.pane_id === SCREEN_PANE || session.pane_id === REVIEW_PANE) session.device_id = "mini";
    }
  }
  return { mission, rest };
}

/** The no-Conductor mission with every node done and its close refused for three empty check lines, raised on the first node and waiting on the operator. */
function closeRefused({ mission, rest }: { mission: Mission; rest: SnapshotRest }, now: number): { mission: Mission; rest: SnapshotRest } {
  mission.nodes.forEach((one, index) => {
    Object.assign(one, { status: "done", finished_at_unix_ms: now - (30 - index) * MINUTE, attention: null, hide: null, landing: null, warnings: [], agent_runtime: null });
    one.criteria = one.criteria.map((criterion) => (criterion.state === "open" && criterion.kind !== "check" ? { ...criterion, state: "pass" as const } : criterion));
    one.criteria_met = one.criteria.filter((criterion) => criterion.state === "pass" || criterion.state === "skipped").length;
  });
  const first = mission.nodes[0]!;
  first.title = "카톡 통화 기록";
  first.attention = { reason: "close_refused", pr: null, to: "operator", since_unix_ms: now - 6 * MINUTE, on_row: false, item: true, choices: ["done"], unmet_nodes: 3 };
  mission.decision = null;
  return { mission, rest };
}

/**
 * `rest` with the no-Conductor mission waiting on the operator, counted on
 * `deviceId`'s scope as the core counts a mission no row carries (PRD
 * mission-node-simple B14): named in `mission_asks` and in its Needs You number.
 */
export function withMissionAsk(rest: SnapshotRest, deviceId: string, now: number, shown = "no-conductor"): SnapshotRest {
  const { mission, rest: fixture } = missionFixture(shown, now);
  const devices = rest.navigator?.devices?.map((device) => device.id !== deviceId ? device : {
    ...device,
    agent_scope: { ...device.agent_scope, mission_asks: [mission.id], overview_needs_you: device.agent_scope.overview_needs_you + 1, groups: { ...device.agent_scope.groups, needs_you: device.agent_scope.groups.needs_you + 1 } },
  });
  return { ...rest, missions: fixture.missions, navigator: { ...rest.navigator!, devices: devices ?? [] } };
}

/** The page's title as the Settings sheet draws it above the tab. */
function SettingsPageTitle() {
  const { t } = useInterfaceTranslation();
  return (
    <div className="mb-lg flex min-w-0 items-center gap-sm">
      <h2 className="truncate text-headline font-semibold">{t("settings.tabs.conductor")}</h2>
    </div>
  );
}

export function MissionScene({ params }: { params: URLSearchParams }) {
  useFollowInterfaceLanguage(requireInterfaceLanguage(params.get("lang") ?? "ko"));
  const theme = params.get("theme") === "light" ? "light" : "dark";
  const state = params.get("state") ?? "running";
  const pane = PANE_STATES[state];
  const fixture = useMemo(() => missionFixture(pane ?? state, Date.now()), [pane, state]);
  // The map's ↓ and → turn it here as the core's echo would; `flow=across` opens it turned.
  const actions = useMemo(() => createActions((event) => {
    const flow = event.kind === "conductor_settings_set" ? (event.payload as { map_flow?: MissionMapFlow }).map_flow : undefined;
    if (flow) useShellStore.setState((shell) => ({ rest: shell.rest ? withMapFlow(shell.rest, flow) : shell.rest }));
    return true;
  }), []);
  const flow = params.get("flow") === "across" ? "across" : "down";
  const pick = params.get("pick");
  useLayoutEffect(() => {
    // The decision state's fix worker is one the core raised: its own line says why it stopped.
    const raised = { pane_id: FIX_PANE, device_id: "local", escalation: { cause: "child_blocked", letter_id: null, since_unix_ms: Date.now() - 3 * MINUTE, human_notice: true }, request: { verb: "blocked", verb_since_unix_ms: Date.now() - 3 * MINUTE, line: "로그인이 만료되어 테스트를 못 돌림", end: "blocked", request: null, later_by: null, pull_requests: [] } };
    // The Conductor's own row: at work while its mission runs, asleep once it closed, none when no Conductor holds it.
    const closed = ["done", "cancelled", "no-pr", "exempt"].includes(state);
    const conductor = { pane_id: CONDUCTOR_PANE, device_id: "local", mark: closed ? "asleep" : state === "decision" ? "quiet" : "going" };
    const agents = (pane ? [{ pane_id: CONDUCTOR_PANE, device_id: "local" }] : state === "no-conductor" || state === "failed" ? [] : state === "decision" ? [conductor, raised] : [conductor]) as unknown as AgentRow[];
    // The mission screen shows the mission, so its head wears the kind mark and the `?` as the app's does.
    const rest = withMapFlow(fixture.rest, flow);
    useShellStore.setState({ rest, agents, connection: "live" });
    // The popover pinned, as a press, the Show task steps chord or a first plan watched leaves it.
    const device = rest.missions?.items.find((row) => row.id === "mission-1")?.device_id ?? null;
    useUiStore.setState({ planPane: pane ? CONDUCTOR_PANE : null, screen: { kind: "missions" }, missions: { ...MISSION_ENTRY, deviceId: device, selected: "mission-1" } });
    document.documentElement.classList.toggle("dark", theme === "dark");
    document.documentElement.classList.toggle("light", theme === "light");
    document.documentElement.style.setProperty("--interface-scale", params.get("scale") ?? "1");
  }, [fixture, pane, state, theme, params, flow]);
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
  // `hint=role`: the first checker beside a card takes focus, which opens its hover, the role's line under the agent and its state.
  const roleHint = params.get("hint") === "role";
  useEffect(() => {
    if (!roleHint) return;
    let frame = 0;
    let left = 120;
    const focus = () => {
      const mark = document.querySelector<HTMLElement>("[data-node-satellite]");
      if (mark) mark.focus();
      else if ((left -= 1) > 0) frame = requestAnimationFrame(focus);
    };
    frame = requestAnimationFrame(focus);
    return () => cancelAnimationFrame(frame);
  }, [roleHint, fixture]);
  const width = Number(params.get("width") ?? (state === "settings" ? 900 : 1080));
  return (
    <TooltipProvider>
      <div style={{ width }} className="flex h-full min-w-0 flex-col bg-background text-foreground" data-gallery-scene="mission" data-mission-scene={state}>
        {state === "settings" ? (
          <div className="p-xl"><SettingsPageTitle /><ConductorTab actions={actions} /></div>
        ) : pane ? (
          <div className="flex h-(--size-pane-header) min-w-0 items-center gap-xs border-b border-border bg-muted px-sm text-caption" data-mission-scene-pane="true">
            <AgentMark kind="claude" />
            <span className="truncate">Conductor</span>
            <PaneWorkChip paneId={CONDUCTOR_PANE} actions={actions} />
          </div>
        ) : (
          <MissionScreen actions={actions} />
        )}
      </div>
    </TooltipProvider>
  );
}

function withMapFlow(rest: SnapshotRest, flow: MissionMapFlow): SnapshotRest {
  const settings = rest.ui_state?.conductor_settings;
  return settings ? { ...rest, ui_state: { ...rest.ui_state!, conductor_settings: { ...settings, map_flow: flow } } } : rest;
}
