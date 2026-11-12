// Invented missions for the gallery's phone scenes (PRD mobile-companion-ui
// B28, B31, B33-B36; doop final board F8 and F11): a release that waits on a
// decision, the steps around it, and the other groups of the missions tab.
// Names, numbers and times are fixtures, never operator data.

import type { PhoneMission, PhoneMissionNode } from "../mobile/protocol";

export const RELEASE_MISSION = "mission-release";
/** The mission of general steps of each preset and a waiting step (PRD node-presets B10, B12). */
export const PRESETS_MISSION = "mission-presets";
/** The branch the release mission's results go to (PRD mission-destination D-52, B59): its steps' branch lines end there and its head names it. */
const RELEASE_BRANCH = "release/1.2";

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;

function step(key: string, title: string, status: PhoneMissionNode["status"], over: Partial<PhoneMissionNode> = {}): PhoneMissionNode {
  return {
    key, title, kind: "change", depends_on: [], status, blocked: null, delivery: "pr", pr: null, pr_state: null, base_pr: null, stage: null,
    attention: null, landing: null, agent_runtime: "running", worker_agent: "claude", agent: null, escalation: null, ...over,
  };
}

const merged = (key: string, title: string, pr: number, worker: string): PhoneMissionNode =>
  step(key, title, "done", { pr, pr_state: "merged", worker_agent: worker, agent_runtime: "ended" });

function mission(id: string, title: string, over: Partial<PhoneMission> = {}): PhoneMission {
  return {
    id, title, device_id: "local", device_label: null, status: "running", decision: null, question: null, answer: null, answer_failed: null,
    landing_order: [], created_at_unix_ms: Date.now() - 3 * HOUR, started_at_unix_ms: Date.now() - 3 * HOUR, closed_at_unix_ms: null, stale: false,
    conductor: { present: true, takes_answers: true, name: "Conductor", agent_kind: "claude", agent: { device_id: "local", pane_id: "gallery-conductor" } },
    nodes: [], ...over,
  };
}

/** The F11 mission: eight steps, four done, two at work, one stopped on the decision the Conductor asks and one waiting on two of them, with `views` the Views linked to it to look at. */
function release(views: boolean): PhoneMission {
  const started = Date.now() - (2 * 60 + 14) * MINUTE;
  return mission(RELEASE_MISSION, "1.2.0 릴리스 준비", {
    destination: RELEASE_BRANCH,
    created_at_unix_ms: started,
    started_at_unix_ms: started,
    decision: "CI 인증서가 만료됐어요. 이번 릴리스는 서명 없이 내보낼까요?",
    question: { key: 3, why: null, options: ["서명 없이 진행", "릴리스 미루기"], recommend: null, node: "sign", asked_at_unix_ms: Date.now() - 6 * MINUTE },
    decision_views: views
      ? [
          { device_id: "local", display_id: "gallery-sign-report", view: "gallery-v1", kind: "browser", label: "서명 없이 내보낼 때 바뀌는 것", unseen: true, by: "conductor", pane_id: "gallery-conductor" },
          { device_id: "local", display_id: "gallery-sign-log", view: "gallery-v2", kind: "file", label: "ci-sign.log", by: "worker" },
        ]
      : undefined,
    nodes: [
      merged("notes", "릴리스 노트 초안", 2085, "claude"),
      merged("pin", "agy 고정 버전 올리기", 2082, "codex"),
      merged("bump", "버전 번호 올리기", 2080, "claude"),
      merged("log", "체인지로그 확인", 2079, "codex"),
      // The plan gives the operator this merge: a faint orange person stands before its destination until the pull request is ready.
      step("holder", "holder 버전 폴더 정리", "working", { pr: 2091, pr_state: "pending", merge: { by: "operator", why: "holder 경로는 직접 확인", state: "ahead", prs: [2091] }, worker_agent: "codex", stage: 2, agent: { device_id: "local", pane_id: "gallery-holder" } }),
      step("rehearsal", "마이그레이션 리허설", "working", { depends_on: ["holder"], pr: 2093, pr_state: "pending", base_pr: 2091, worker_agent: "claude", stage: 1, agent: { device_id: "local", pane_id: "gallery-rehearsal" } }),
      step("sign", "Windows 서명 단계", "working", { depends_on: ["bump"], worker_agent: "claude", stage: 0 }),
      step("verify", "릴리스 빌드 확인", "planned", { kind: "general", preset: "verify", delivery: null, depends_on: ["rehearsal", "sign"], worker_agent: null, agent_runtime: null }),
    ],
  });
}

/** General steps of each preset at work, done and planned, a deploy step's start gate ahead and asking, and a waiting step (PRD node-presets B10, B12): the phone draws each preset's icon and name, never a result line or a verdict (mission-57 B59). */
function presets(): PhoneMission {
  const gate = (state: "ahead" | "open") => ({ state, look: "배포해도 될지", when: "start" as const, ask: state === "open" ? 4 : null, since_unix_ms: state === "open" ? Date.now() - 2 * MINUTE : null, long_wait_at_unix_ms: null, via: null, said: null });
  const general = (key: string, title: string, status: PhoneMissionNode["status"], preset: PhoneMissionNode["preset"], over: Partial<PhoneMissionNode> = {}) =>
    step(key, title, status, { kind: "general", preset, delivery: null, worker_agent: status === "planned" ? null : "claude", agent_runtime: status === "planned" ? null : status === "done" ? "ended" : "running", ...over });
  return mission(PRESETS_MISSION, "1.2.8 출시", {
    destination: "main",
    nodes: [
      general("cause", "재시도 잠금 원인 찾기", "done", "research"),
      general("survey", "열린 PR 정리 기준", "working", "research"),
      general("check", "합친 변경 검증", "done", "verify", { depends_on: ["cause"], worker_agent: "codex" }),
      general("copy", "사이트 문구 정리", "working", null),
      general("ship", "1.2.8 배포", "planned", "deploy", { depends_on: ["check"], gate: gate("open") }),
      general("store", "앱 스토어 제출", "planned", "deploy", { depends_on: ["survey"], gate: gate("ahead") }),
      step("login", "로그인 수정 기다림", "working", { kind: "external", delivery: null, pr: null, worker_agent: null, agent_runtime: null, external_pr: 1232, external_base: "main", external_state: "pending" }),
    ],
  });
}

/** The five places the missions tab draws: one asking, one at work, one on another Mac, one waiting to merge, and finished ones past the three shown. */
export function phoneMissions({ views = false, withPresets = false }: { views?: boolean; withPresets?: boolean } = {}): PhoneMission[] {
  return [
    ...(withPresets ? [presets()] : []),
    release(views),
    mission("mission-ui", "모바일 UI 개선", {
      created_at_unix_ms: Date.now() - 50 * MINUTE,
      started_at_unix_ms: Date.now() - 50 * MINUTE,
      nodes: [
        merged("a", "하단 탭 만들기", 2101, "claude"),
        step("b", "미션 탭 그리기", "working", { pr: 2102, stage: 2, agent: { device_id: "local", pane_id: "gallery-ui-b" } }),
        step("c", "PR 탭 그리기", "working", { pr: 2103, worker_agent: "codex", stage: 1, agent: { device_id: "local", pane_id: "gallery-ui-c" } }),
        step("d", "문서 고치기", "planned", { depends_on: ["b"], worker_agent: null, agent_runtime: null }),
      ],
    }),
    mission("mission-lecture", "강의 정산 자동화", {
      device_id: "mini",
      device_label: "Mac mini",
      created_at_unix_ms: Date.now() - 95 * MINUTE,
      started_at_unix_ms: Date.now() - 95 * MINUTE,
      nodes: [
        merged("a", "정산표 가져오기", 311, "codex"),
        step("b", "환불 건 합치기", "working", { pr: 312, stage: 2, agent: { device_id: "mini", pane_id: "gallery-lecture-b" } }),
        step("c", "메일 초안 만들기", "planned", { depends_on: ["b"], worker_agent: null, agent_runtime: null }),
      ],
    }),
    mission("mission-site", "사이트 문구 정리", {
      status: "merge_wait",
      created_at_unix_ms: Date.now() - 5 * HOUR,
      started_at_unix_ms: Date.now() - 5 * HOUR,
      nodes: [
        step("a", "랜딩 문구 다듬기", "done", { pr: 2088, pr_state: "pending", agent_runtime: "ended" }),
        step("b", "가격 표 고치기", "done", { pr: 2089, pr_state: "pending", worker_agent: "codex", agent_runtime: "ended", base_pr: 2088 }),
      ],
    }),
    ...[
      ["mission-closed-1", "설치 가이드 다듬기", "done", 1],
      ["mission-closed-2", "리눅스 CI 정리", "done", 2],
      ["mission-closed-3", "알림 문구 실험", "cancelled", 3],
      ["mission-closed-4", "지난 릴리스 회고", "done", 4],
    ].map(([id, title, status, days]) =>
      mission(id as string, title as string, {
        status: status as PhoneMission["status"],
        created_at_unix_ms: Date.now() - (days as number) * 24 * HOUR - 2 * HOUR,
        started_at_unix_ms: Date.now() - (days as number) * 24 * HOUR - 2 * HOUR,
        closed_at_unix_ms: Date.now() - (days as number) * 24 * HOUR,
        nodes: [merged("a", "작업", 2000 + (days as number), "claude")],
      }),
    ),
  ];
}
