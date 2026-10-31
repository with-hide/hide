// Invented Desk values for the gallery's Desk scenes (PRD
// session-docs-and-assistant, Desk 1차): agents, conversations, jobs in every
// state, threads as ACP updates and outputs. Names, counts and paths are
// made up; nothing here is read from a machine.
import type { AcpSessionUpdate, DeskAgent, DeskItem, DeskJob, DeskMention, DeskMission, DeskOutput, DeskSessionRef, DeskSteps, SessionViewKind, ThreadHidePart } from "../desk";
import type { DeskSessionChip, RowMark } from "../snapshot";
import type { MentionProject, MentionSession } from "../components/desk/desk-mention-menu";
import { planFixture } from "./PlanScene";

const MINUTE = 60_000;

/** A picture of a capture: a filled card, in named colours so no literal reaches the source rules. */
function picture(ground: string, card: string, line: string): string {
  const svg = `<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 500 200'><rect width='500' height='200' fill='${ground}'/><rect x='24' y='24' width='300' height='152' rx='10' fill='${card}'/><rect x='44' y='48' width='180' height='14' rx='4' fill='${line}'/><rect x='44' y='76' width='240' height='10' rx='4' fill='${line}' opacity='0.6'/><rect x='44' y='96' width='200' height='10' rx='4' fill='${line}' opacity='0.6'/><rect x='348' y='24' width='128' height='152' rx='10' fill='${card}'/></svg>`;
  return `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`;
}

export const PICTURES = {
  light: picture("whitesmoke", "white", "lightsteelblue"),
  dark: picture("black", "darkslategray", "slategray"),
  page: picture("aliceblue", "lightcyan", "cadetblue"),
};

const EFFORTS = [{ id: "low", label: "low" }, { id: "medium", label: "medium" }, { id: "high", label: "high" }];

export const AGENTS: DeskAgent[] = [
  { id: "claude", label: "Claude", state: "ready", detail: null, login_command: "claude /login", models: [{ id: "opus", label: "Opus 5.5" }, { id: "sonnet", label: "Sonnet 5.5" }], efforts: EFFORTS },
  { id: "codex", label: "Codex", state: "ready", detail: null, login_command: "hide desk login codex", models: [{ id: "gpt", label: "GPT-6" }, { id: "gpt-mini", label: "GPT-6 mini" }], efforts: EFFORTS },
  { id: "pi", label: "Pi", state: "no_ask_mode", detail: null, login_command: "pi login", models: [], efforts: [] },
];

export const AGENTS_PREPARING: DeskAgent[] = AGENTS.map((agent) => (agent.id === "codex" ? { ...agent, state: "preparing" } : agent));

const output = (id: string, kind: DeskOutput["kind"], label: string, seen: boolean, thumb: string | null = null): DeskOutput => ({ id, kind, label, seen, thumb, pr_state: null, device_id: "local", checkout: "/repo/herdr-ide", path: kind === "page" || kind === "pr" ? null : `/repo/herdr-ide/${label}`, url: kind === "page" ? `http://localhost:5173/${label}` : null });

export const OUTPUTS = {
  light1: output("o-light-1", "image", "light-1.png", true, PICTURES.light),
  light2: output("o-light-2", "image", "light-2.png", true, PICTURES.light),
  light3: output("o-light-3", "image", "light-3.png", false, PICTURES.light),
  compareLight: output("o-compare-light", "image", "compare-light.png", false, PICTURES.light),
  compareDark: output("o-compare-dark", "image", "compare-dark.png", false, PICTURES.dark),
  notes: output("o-notes", "doc", "notes.md", true),
  galleryPage: output("o-gallery-page", "page", "gallery · desk", true, PICTURES.page),
  pr: { ...output("o-pr-41", "pr", "PR #41 자막 수정", true), pr_state: "merged" },
};

export const MISSION: DeskMission = { id: "mission-desk", title: "Desk 1차", status: "running", done: 2, total: 6, deciding: false };

/** Sessions as the chips inside a line name them. */
export const SESSIONS = {
  recordNow: { name: "record-now", mark: "going", agent_kind: "codex" } satisfies DeskSessionRef,
  gallery: { name: "갤러리 캡처 비교", mark: "turn", agent_kind: "claude" } satisfies DeskSessionRef,
  review: { name: "캡처 차이 리뷰", mark: "going", agent_kind: "codex" } satisfies DeskSessionRef,
  room: { name: "1a-desk-room", mark: "done", agent_kind: "codex" } satisfies DeskSessionRef,
};

/** The mark the sidebar row would carry for a state: hided sends it with the job; the gallery invents it the same way. */
function markOf(job: DeskJob): RowMark | null {
  switch (job.state) {
    case "starting":
      return "waiting";
    case "working":
      return "going";
    case "asking":
      return "turn";
    case "done":
      return job.read ? "quiet" : "done";
    case "sleeping":
      return "asleep";
    default:
      return null;
  }
}

/** A job as the wire sends it; `mark` follows its state unless given. */
export function deskJob(job: DeskJob, extra: Partial<DeskJob>): DeskJob {
  const next = { ...job, ...extra };
  return { ...next, mark: extra.mark !== undefined ? extra.mark : markOf(next) };
}

/** The panes of the sessions Desk writes to. */
const PANES: Record<string, string> = { "record-now": "w1:record-now", "갤러리 캡처 비교": "w1:gallery" };

const base = (now: number) => (id: string, title: string, extra: Partial<DeskJob>): DeskJob => deskJob({
  id, title, agent_id: `agent-${id}`, pane_id: `w1:${id}`, agent_kind: "claude",
  project: { workspace_id: "herdr-ide", label: "herdr-ide" },
  location: { kind: "checkout", label: "herdr-ide" },
  state: "working", queue_position: null, now: null, plan: null, question: null, summary: null,
  pull_requests: [], outputs: [], joined: false, reason: null, failure: null, ask: null, mark: null, mark_read: false,
  machine: null, disconnected: false, changed_at_unix_ms: now - 5 * MINUTE, read: false, mission: null, workers: [],
}, extra);

const MENTION_BRANCH = { kind: "worktree", label: "desk/mention-button" } as const;

/** The jobs of the room's story (scenes 1, 2, 2-1 and 4-7). */
export function storyJobs(now: number): Record<string, DeskJob> {
  const job = base(now);
  const jobs: DeskJob[] = [
    job("mention", "mention 버튼 정리", { location: MENTION_BRANCH, now: "테스트 돌리는 중", plan: { done: 6, total: 14 }, pull_requests: [{ number: 1903, state: "pending" }], changed_at_unix_ms: now - 12 * MINUTE }),
    job("gallery", "갤러리 캡처 비교", { state: "asking", ask: "question", question: "다크 모드도 찍을까요?", outputs: [OUTPUTS.compareLight, OUTPUTS.light1, OUTPUTS.light2], changed_at_unix_ms: now - 9 * MINUTE }),
    job("release", "release 노트 초안", { state: "done", summary: "초안을 notes.md에 썼어요", outputs: [OUTPUTS.notes], changed_at_unix_ms: now - 62 * MINUTE }),
    job("record", "record-now", { state: "asking", ask: "approval", question: "rm -rf web/dist", agent_kind: "codex", changed_at_unix_ms: now - 2 * MINUTE }),
    job("conductor", "Desk 1차", { now: "Conductor · ui 다듬기 외 1", mission: MISSION, workers: ["w1:1a", "w1:1b"], changed_at_unix_ms: now - 22 * MINUTE }),
    job("1a", "1a Desk 방", { state: "done", read: true, location: { kind: "worktree", label: "feat/desk-host" }, summary: "리뷰 기다림", pull_requests: [{ number: 1905, state: "pending" }], agent_kind: "codex" }),
    job("1b", "1b 스레드 화면", { location: { kind: "worktree", label: "feat/desk-thread" }, now: "ThreadView 턴 묶기", changed_at_unix_ms: now - 6 * MINUTE }),
    job("reel", "reel 자막 수정", { project: { workspace_id: "creator", label: "creator" }, location: { kind: "checkout", label: "creator" }, machine: "mini", disconnected: true, now: "자막 타이밍 맞추는 중", agent_kind: "codex", changed_at_unix_ms: now - 31 * MINUTE }),
  ];
  return Object.fromEntries(jobs.map((one) => [one.id, one]));
}

/** The card scene: every state a card draws, each with the one chip that matters in it (B16-B24, B36-B39). */
export function everyStateJobs(now: number): { label: string; job: DeskJob }[] {
  const job = base(now);
  return [
    { label: "작업 중 · 계획 칩", job: job("c-working", "mention 버튼 정리", { location: MENTION_BRANCH, now: "테스트 돌리는 중", plan: { done: 6, total: 14 } }) },
    { label: "물어봄 · 결과물 칩", job: job("c-asking", "갤러리 캡처 비교", { state: "asking", ask: "question", question: "다크 모드도 찍을까요?", outputs: [OUTPUTS.compareLight, OUTPUTS.light1, OUTPUTS.light2] }) },
    { label: "허락 기다림 (터미널의 권한 질문)", job: job("c-permission", "record-now", { state: "asking", ask: "approval", question: "rm -rf web/dist", agent_kind: "codex" }) },
    { label: "끝남 · PR", job: job("c-done", "pane header @ hover", { state: "done", location: { kind: "worktree", label: "desk/pane-mention" }, summary: "PR을 열었고 CI 통과", pull_requests: [{ number: 1910, state: "mergeable" }] }) },
    { label: "미션 (Conductor)", job: job("c-conductor", "Desk 1차", { now: "Conductor · 지금 ui 다듬기 외 1", mission: MISSION }) },
    { label: "미션 판단", job: job("c-decision", "Desk 1차", { state: "asking", ask: "decision", question: "1b를 공용 부품 머지 전에 시작할까요?", mission: { ...MISSION, deciding: true } }) },
    { label: "계획 검토 (hide-plan)", job: job("c-review", "문서 정리", { state: "asking", ask: "plan_review", question: "5단계", plan: { done: 0, total: 5 }, agent_kind: "codex" }) },
    { label: "다른 기기", job: job("c-mini", "reel 자막 수정", { project: { workspace_id: "creator", label: "creator" }, machine: "mini", now: "자막 타이밍 맞추는 중", agent_kind: "codex" }) },
    { label: "다른 기기, 연결 끊김 (흐림)", job: job("c-mini-off", "reel 자막 수정", { project: { workspace_id: "creator", label: "creator" }, machine: "mini", disconnected: true, now: "12분 전까지의 기록", agent_kind: "codex" }) },
    { label: "나도 참여 중", job: job("c-joined", "mention 버튼 정리", { location: MENTION_BRANCH, joined: true, now: "아이콘 14로 줄이는 중" }) },
    { label: "대기", job: job("c-queued", "로그 정리", { state: "queued", queue_position: 2 }) },
    { label: "실패", job: job("c-failed", "릴스 썸네일 3안", { state: "failed", reason: "spawn_failed", project: { workspace_id: "creator", label: "creator" }, agent_kind: "codex" }) },
    { label: "끝까지 못 감", job: job("c-exited", "의존성 올리기", { state: "failed", reason: "exited", location: { kind: "worktree", label: "desk/bump-deps" } }) },
    { label: "취소됨", job: job("c-cancelled", "아이콘 크기 맞추기", { state: "cancelled" }) },
    { label: "잠듦", job: job("c-asleep", "문서 정리", { state: "sleeping", agent_kind: "codex" }) },
    { label: "보관됨 (흐림)", job: job("c-archived", "로그 정리", { state: "archived" }) },
  ];
}

export const MENTIONS: DeskMention[] = [
  { kind: "session", name: "record-now", id: "agent-record-now", mark: "going", agent_kind: "codex" },
  { kind: "session", name: "1a-desk-room", id: "agent-1a", mark: "done", agent_kind: "codex" },
  { kind: "project", name: "herdr-ide", id: "~/projects/herdr-ide" },
  { kind: "project", name: "creator", id: "~/creator" },
];

const at = (now: number, minutes: number) => now - minutes * MINUTE;

const NO_STEPS: DeskSteps = { sessions: 0, files: 0, letters: 0, starts: 0, other: 0 };

/** A letter Desk sent: handed work (`request`) or another letter, with its first line. */
const letter = (id: number, session: DeskSessionRef, kind: string, line: string, read: boolean): DeskItem => ({ kind: "handed", id, agent_id: `agent-${id}`, pane_id: PANES[session.name]!, name: session.name, project: "herdr-ide", letter_kind: kind, line, letter_id: `letter-${id}`, read, agent_kind: session.agent_kind, mark: session.mark });

/** A session a reply names, as the wire sends its chip. */
const chip = (session: DeskSessionRef): DeskSessionChip => ({ ...session, pane_id: PANES[session.name]! });

/** Scene 2: a hand-off that started three threads, a task handed to a session already on it, and Desk thinking. */
export function handoffItems(now: number): DeskItem[] {
  return [
    { kind: "user", id: 1, text: "@project(~/projects/herdr-ide herdr-ide) mention 버튼 정리하고 PR까지, 갤러리 캡처 라이트·다크 비교, release 노트 초안도 맡겨줘", at_ms: at(now, 15), mentions: [MENTIONS[2]!] },
    { kind: "reply", mentions: [], agent: "claude", id: 2, text: "스레드 세 개를 띄웠어요. mention 버튼은 PR로 낼 일이라 새 worktree에서 해요.", at_ms: at(now, 15), state: "done", thought_ms: null, steps: NO_STEPS },
    { kind: "job", id: 3, job_id: "mention" },
    { kind: "job", id: 4, job_id: "gallery" },
    { kind: "job", id: 5, job_id: "release" },
    { kind: "user", id: 6, text: "@session(agent-record-now record-now) 갤러리 숫자 줄인 거 다크에서도 확인해줘", at_ms: at(now, 6), mentions: [MENTIONS[0]!] },
    letter(7, SESSIONS.recordNow, "request", "갤러리 숫자 줄인 거 다크에서도 확인해줘", true),
    { kind: "user", id: 8, text: "mention 버튼 PR 올라갔어?", at_ms: at(now, 0), mentions: [] },
  ];
}

/** Scene 2-1: asking how things go. Desk answers in words; only its own thread waiting on the operator comes as a card. */
export function statusItems(now: number): DeskItem[] {
  return [
    { kind: "user", id: 11, text: "지금 작업들 잘 되고 있어?", at_ms: at(now, 8), mentions: [] },
    {
      kind: "reply", mentions: [chip(SESSIONS.recordNow), chip(SESSIONS.gallery)], agent: "claude", id: 12, at_ms: at(now, 8), state: "done", thought_ms: 5_000, steps: { ...NO_STEPS, sessions: 9 },
      text: "9개 중 **2개가 답을 기다리고**, 하나는 꼬일 수 있어요.\n\n- **꼬일 수 있어요** · @session(w1:record-now record-now)와 @session(w1:gallery 갤러리 캡처 비교)가 둘 다 `scripts/capture-gallery.mjs`를 고쳐요. 맞춰 줄까요?\n- **미션 판단** · Desk 1차가 1b를 공용 부품 머지 전에 시작할지 물어요.\n- **잘 돌고 있어요** · mention 버튼 정리(6/14), reel 자막 수정(mini) 외 4개. PR 1905는 초록이라 머지만 남았어요.",
    },
    { kind: "job", id: 13, job_id: "gallery" },
    { kind: "user", id: 14, text: "응 맞춰줘", at_ms: at(now, 4), mentions: [] },
    letter(15, SESSIONS.recordNow, "fyi", "다크 출력은 dark/로 맞춰요", true),
    letter(16, SESSIONS.gallery, "fyi", "record-now가 같은 파일을 고쳐요, 먼저 받아서 하세요", false),
    { kind: "reply", mentions: [], agent: "claude", id: 17, text: "양쪽에 알렸어요. 갤러리 캡처 비교는 아직 질문에 답을 기다리고 있어요.", at_ms: at(now, 4), state: "done", thought_ms: null, steps: NO_STEPS },
  ];
}

/** The agent menu scene: a conversation that changed agents and then model. */
export function switchedItems(now: number): DeskItem[] {
  return [
    { kind: "user", id: 21, text: "오늘 끝난 거 정리해줘", at_ms: at(now, 30), mentions: [] },
    { kind: "reply", mentions: [], agent: "claude", id: 22, text: "세 개 끝났어요: release 노트, 갤러리 비교, reel 자막. 갤러리 비교는 다크 캡처가 남았어요.", at_ms: at(now, 29), state: "done", thought_ms: null, steps: NO_STEPS },
    { kind: "switch", id: 23, from: "claude", to: "codex", carried: "all" },
    { kind: "user", id: 24, text: "그럼 다크 캡처는 이어서 맡겨줘", at_ms: at(now, 20), mentions: [] },
    { kind: "reply", mentions: [], agent: "codex", id: 25, text: "갤러리 비교 스레드에 다크 모드 캡처를 이어서 하라고 전했어요.", at_ms: at(now, 19), state: "done", thought_ms: null, steps: NO_STEPS },
    { kind: "model", id: 26, model: "gpt-mini", effort: "medium" },
  ];
}

/** The @ menu scenes: a mention typed into the composer. */
export function mentionItems(now: number): DeskItem[] {
  return [
    { kind: "user", id: 31, text: "@session(agent-record-now record-now) 갤러리 숫자 줄인 거 다크에서도 확인해줘", at_ms: at(now, 6), mentions: [MENTIONS[0]!] },
    letter(32, SESSIONS.recordNow, "request", "갤러리 숫자 줄인 거 다크에서도 확인해줘", false),
  ];
}

/** The mission scene: the mission hand-off. */
export function missionItems(now: number): DeskItem[] {
  return [
    { kind: "user", id: 41, text: "Desk 1차를 미션으로 돌려줘", at_ms: at(now, 25), mentions: [] },
    { kind: "reply", mentions: [], agent: "claude", id: 42, text: "Conductor 스레드를 띄워 미션으로 돌려요. 노드 여섯 개, PR 모드예요.", at_ms: at(now, 25), state: "done", thought_ms: null, steps: NO_STEPS },
    { kind: "job", id: 43, job_id: "conductor" },
  ];
}

/** The lines of the card scene that change no state, and Desk's own approval card asked and answered. */
export function cardSceneItems(): DeskItem[] {
  return [
    { kind: "switch", id: 51, from: "claude", to: "codex", carried: "all" },
    { kind: "notice", id: 52, notice: "blocked", agent: "claude", detail: null },
    { kind: "approval", id: 53, card: "command", command: "git worktree add ../desk-mention -b desk/mention-button", folder: "~/projects/herdr-ide", reason: "PR로 낼 일이라 새 worktree에서 하려고 해요.", state: "waiting" },
    { kind: "approval", id: 54, card: "command", command: "git worktree add ../desk-mention -b desk/mention-button", folder: "~/projects/herdr-ide", reason: null, state: "allowed" },
  ];
}

/** The chat's other states (B5-B8, B11, B13-B15): approvals in each answer and every notice. */
export function stateItems(now: number): DeskItem[] {
  return [
    { kind: "user", id: 61, text: "mention 버튼 브랜치 정리하고 테스트 다시 돌려줘", at_ms: at(now, 9), mentions: [] },
    { kind: "approval", id: 62, card: "edit", command: "edit web/src/PaneHeader.tsx", folder: "~/projects/herdr-ide", reason: "hover 조건을 고쳐야 해요", state: "allowed" },
    { kind: "approval", id: 63, card: "command", command: "rm -rf web/dist", folder: "~/projects/herdr-ide", reason: "빌드 결과를 지우고 새로 만들게요", state: "rejected" },
    { kind: "approval", id: 64, card: "command", command: "pnpm --dir web test", folder: "~/projects/herdr-ide", reason: "테스트를 돌려 볼게요", state: "closed" },
    { kind: "model", id: 65, model: "sonnet", effort: "low" },
    { kind: "switch", id: 66, from: "claude", to: "codex", carried: "trimmed" },
    { kind: "switch", id: 67, from: "codex", to: "claude", carried: "none" },
    { kind: "notice", id: 68, notice: "preparing", agent: "codex", detail: null },
    { kind: "notice", id: 69, notice: "install_failed", agent: "codex", detail: "네트워크에 연결할 수 없어요" },
    { kind: "notice", id: 70, notice: "login_needed", agent: "codex", detail: "hide desk login codex" },
    { kind: "notice", id: 71, notice: "host_stopped", agent: "claude", detail: null },
    { kind: "notice", id: 72, notice: "resume_failed", agent: "claude", detail: null },
    { kind: "notice", id: 73, notice: "no_ask_mode", agent: "pi", detail: null },
    { kind: "notice", id: 74, notice: "turn_failed", agent: "claude", detail: null },
    { kind: "reply", mentions: [], agent: "claude", id: 75, text: "정리하고 테스트를 다시 돌렸어요. 실패 하나는", at_ms: at(now, 1), state: "stopped", thought_ms: null, steps: NO_STEPS },
    { kind: "user", id: 76, text: "지금 돌고 있는 일들 어디까지 왔어요?", at_ms: at(now, 0), mentions: [] },
  ];
}

const text = (sessionUpdate: "user_message_chunk" | "agent_message_chunk" | "agent_thought_chunk", words: string): AcpSessionUpdate => ({ sessionUpdate, content: { type: "text", text: words } });
const call = (id: string, title: string, kind: "read" | "edit" | "execute", status: "completed" | "in_progress", extra: Record<string, unknown> = {}): AcpSessionUpdate => ({ sessionUpdate: "tool_call", toolCallId: id, title, kind, status, ...extra });
const hideCall = (id: string, command: string): AcpSessionUpdate => call(id, "Bash", "execute", "completed", { rawInput: { command } });

/** The gallery thread's first turn: the task Desk handed it, the light captures and its question. */
const GALLERY_FIRST_TURN: AcpSessionUpdate[] = [
  text("user_message_chunk", "gallery 캡처를 라이트·다크로 찍어서 나란히 비교해줘. 차이는 notes.md에 정리."),
  text("agent_thought_chunk", "capture-gallery.mjs가 테마를 하나만 받는다. 라이트부터 찍고 다크를 물어본다."),
  call("g-read", "scripts/capture-gallery.mjs", "read", "completed"),
  call("g-edit-1", "scripts/capture-gallery.mjs", "edit", "completed", { content: [{ type: "diff", path: "scripts/capture-gallery.mjs", oldText: "const out = dir", newText: "const out = `${dir}/light`" }] }),
  call("g-edit-2", "scripts/capture-gallery.css", "edit", "completed", { content: [{ type: "diff", path: "scripts/capture-gallery.css", oldText: "a", newText: "b" }] }),
  call("g-edit-3", "notes.md", "edit", "completed", { content: [{ type: "diff", path: "notes.md", oldText: null, newText: "# 갤러리 비교" }] }),
  call("g-run-1", "Bash", "execute", "completed", { rawInput: { command: "pnpm --dir web build" } }),
  call("g-run-2", "Bash", "execute", "completed", { rawInput: { command: "node scripts/capture-gallery.mjs --theme light" } }),
  call("g-run-3", "Bash", "execute", "completed", { rawInput: { command: "ls agents/runs/compare/light" } }),
  call("g-run-4", "Bash", "execute", "completed", { rawInput: { command: "du -sh agents/runs/compare" } }),
  hideCall("g-view-1", "hide file open agents/runs/compare/light/1.png"),
  hideCall("g-view-2", "hide file open agents/runs/compare/light/2.png"),
  hideCall("g-view-3", "hide file open agents/runs/compare/light/3.png"),
  text("agent_message_chunk", "라이트 캡처 6장을 찍었어요. 다크도 찍을까요?"),
];

const GALLERY_FIRST_PARTS: Record<string, ThreadHidePart> = {
  "g-view-1": { kind: "view", output: OUTPUTS.light1 },
  "g-view-2": { kind: "view", output: OUTPUTS.light2 },
  "g-view-3": { kind: "view", output: OUTPUTS.light3 },
};

/** Scene 4: the gallery thread at work on its second turn, its `hide` calls read into Hide's parts. */
export function galleryThread(): { updates: AcpSessionUpdate[]; parts: Record<string, ThreadHidePart> } {
  return {
    updates: [
      ...GALLERY_FIRST_TURN,
      text("user_message_chunk", "응 다크도"),
      call("g2-read", "capture-gallery.mjs", "read", "completed"),
      call("g2-edit", "scripts/capture-gallery.mjs", "edit", "completed", { content: [{ type: "diff", path: "scripts/capture-gallery.mjs", oldText: "const themes = [\"light\"]\nfor (const theme of themes) {", newText: "const themes = [\"light\", \"dark\"]\nconst out = `${dir}/${theme}`\nfor (const theme of themes) {" }] }),
      hideCall("g2-letter", "hide request send record-now --intent dark-path --body …"),
      hideCall("g2-spawn", "hide agent spawn --parent here …"),
      call("g2-run", "Bash", "execute", "in_progress", { rawInput: { command: "node scripts/capture-gallery.mjs --theme dark" }, _meta: { hide: { at_ms: Date.now() - 12_000 } } }),
    ],
    parts: {
      ...GALLERY_FIRST_PARTS,
      "g2-letter": { kind: "letter", direction: "sent", peer: SESSIONS.recordNow, body: "다크 캡처 경로가 바뀌어요" },
      "g2-spawn": { kind: "spawn", session: SESSIONS.review },
    },
  };
}

/** Scene 7: the gallery thread done, its outputs at the end. */
export const GALLERY_DONE_THREAD: { updates: AcpSessionUpdate[]; parts: Record<string, ThreadHidePart> } = {
  updates: [
    ...GALLERY_FIRST_TURN,
    text("user_message_chunk", "응 다크도"),
    call("g3-edit", "scripts/capture-gallery.mjs", "edit", "completed", { content: [{ type: "diff", path: "scripts/capture-gallery.mjs", oldText: "a", newText: "b" }] }),
    call("g3-run", "Bash", "execute", "completed", { rawInput: { command: "node scripts/capture-gallery.mjs --theme dark" } }),
    text("agent_message_chunk", "라이트와 다크를 나란히 비교했어요. 차이는 세 곳이고 notes.md에 정리했어요."),
  ],
  parts: GALLERY_FIRST_PARTS,
};

/** Scene 5: record-now, asking to run a command. */
export const RECORD_THREAD: AcpSessionUpdate[] = [
  text("user_message_chunk", "갤러리 숫자 표시를 줄여 주세요"),
  call("r-edit-1", "web/src/gallery/Gallery.tsx", "edit", "completed", { content: [{ type: "diff", path: "web/src/gallery/Gallery.tsx", oldText: "{count} items", newText: "{count}" }] }),
  call("r-run-1", "Bash", "execute", "completed", { rawInput: { command: "pnpm --dir web test gallery" } }),
  text("agent_message_chunk", "갤러리 숫자 표시를 줄였어요. 다크 모드에서 다시 확인하려면 빌드를 지우고 새로 돌려야 해요."),
];

/** The Hide parts scene: a Conductor's thread, each `hide` call it made read into Hide's part. */
export function conductorThread(jobs: Record<string, DeskJob>): { updates: AcpSessionUpdate[]; parts: Record<string, ThreadHidePart> } {
  const plan = planFixture("running", Date.now());
  return {
    updates: [
      text("user_message_chunk", "Desk 1차를 미션으로 돌려줘"),
      text("agent_message_chunk", "계획을 올리고 @1a-desk-room에 첫 층을 맡길게요."),
      hideCall("h-plan", "hide plan set …"),
      hideCall("h-spawn", "hide agent spawn --parent here …"),
      hideCall("h-letter", "hide request send 1a-desk-room …"),
      hideCall("h-view", "hide browser open …"),
      hideCall("h-mission", "hide mission show …"),
      hideCall("h-pr", "gh pr view 1905"),
      { sessionUpdate: "user_message_chunk", content: { type: "text", text: "반영했어요. CI 기다리는 중\n실패하면 다시 알릴게요." }, _meta: { hide: { origin: "letter", from: "1a-desk-room" } } },
      text("agent_message_chunk", "1b가 끝나면 lane 노드를 띄울게요."),
    ],
    parts: {
      "h-plan": { kind: "plan", plan },
      "h-spawn": { kind: "spawn", session: { name: jobs["1b"]!.title, mark: "going", agent_kind: jobs["1b"]!.agent_kind } },
      "h-letter": { kind: "letter", direction: "sent", peer: SESSIONS.room, body: "리뷰 지적 2개 반영해서 다시 올려줘" },
      "h-view": { kind: "view", output: OUTPUTS.galleryPage },
      "h-mission": { kind: "mission", mission: MISSION },
      "h-pr": { kind: "pr", number: 1905, state: "pending" },
    },
  };
}

/** The mission scene: the worker thread beside the map. */
export const WORKER_THREAD: AcpSessionUpdate[] = [
  { sessionUpdate: "user_message_chunk", content: { type: "text", text: "1b 스레드 화면을 맡아 주세요. 공용 부품은 shared PR에 있어요." }, _meta: { hide: { origin: "letter" } } },
  call("w-edit", "web/src/desk/ThreadView.tsx", "edit", "in_progress"),
];

/** A session of the `@` menu, under `parent` when it has one, its last request `minutes` ago. */
const menuSession = (now: number, id: string, name: string, agent_kind: string, mark: MentionSession["mark"], line: [project: string, branch: string | null, detail: string], minutes: number, parent: string | null = null): MentionSession => ({
  id, name, agent_kind, mark, detail: line[2], parent, project: line[0], branch: line[1], last_prompt_at_unix_ms: at(now, minutes),
});

/** The `@` menu's sessions (D-84): parents with their children, one asking under 프로젝트 패널 인터뷰 so that parent opens from the start. */
export function menuSessions(now: number): MentionSession[] {
  return [
    menuSession(now, "agent-interview", "프로젝트 패널 인터뷰", "claude", "turn", ["herdr-ide", "main", "빈 화면 시작 방식과 PR 머지 여부를 정해 주세요"], 3),
    menuSession(now, "agent-interview-draft", "인터뷰 초안", "claude", "turn", ["herdr-ide", "main", "질문 두 개 남았어요"], 4, "agent-interview"),
    menuSession(now, "agent-holder", "m21 holder-life", "claude", "going", ["herdr-ide", "main", "Conductor · 미션 2/4"], 1),
    menuSession(now, "agent-holder-life", "pane 보관 프로그램 수명", "claude", "going", ["herdr-ide", "m21-holder-life", "테스트 고치는 중"], 1, "agent-holder"),
    menuSession(now, "agent-holder-review", "리뷰", "claude", "quiet", ["herdr-ide", "m21-holder-life", "리뷰 끝, 지적 2개"], 12, "agent-holder"),
    menuSession(now, "agent-holder-verify", "verify", "codex", "quiet", ["herdr-ide", "m21-holder-life", "CI 21개 진행, 실패 없음"], 20, "agent-holder"),
    menuSession(now, "agent-planner", "desk-planner", "claude", "going", ["herdr-ide", "desk-lane", "계획 3/5"], 2),
    menuSession(now, "agent-planner-design", "디자인 리뷰", "claude", "quiet", ["herdr-ide", "desk-lane", "proposal-v2 지적 정리"], 40, "agent-planner"),
    menuSession(now, "agent-planner-copy", "문구 정리", "codex", "going", ["herdr-ide", "desk-lane", "한국어 문구 고치는 중"], 5, "agent-planner"),
    menuSession(now, "agent-pr-review", "PR 리뷰 상태 표시", "claude", "waiting", ["herdr-ide", "pr-review-status", "CI 결과 기다리는 중"], 8),
    menuSession(now, "agent-pr-review-ci", "CI 다시 돌리기", "codex", "going", ["herdr-ide", "pr-review-status", "macOS 줄 기다리는 중"], 6, "agent-pr-review"),
    menuSession(now, "agent-carousel", "캐러셀 비교", "claude", "asleep", ["creator", "main", "세 안 렌더 끝, 고르기 기다림"], 90),
    menuSession(now, "agent-docs-review", "문서 리뷰", "codex", "quiet", ["creator", "main", "용어집 갱신 끝"], 60),
  ];
}

export const MENU_PROJECTS: MentionProject[] = [
  { id: "~/projects/herdr-ide", name: "herdr-ide", sessions: ["w1:mention", "w1:gallery", "w1:release", "w1:record", "w1:1b"] },
  { id: "~/projects/creator", name: "creator", sessions: ["w1:reel", "w1:cover"] },
];

export function enteredSessions(now: number): MentionSession[] {
  return [
    menuSession(now, "agent-record-now", "record-now", "codex", "going", ["herdr-ide", "record-now", "작업 중"], 2),
    menuSession(now, "agent-mention", "mention 버튼 정리", "claude", "going", ["herdr-ide", "mention-button", "계획 6/14"], 4),
    menuSession(now, "agent-gallery", "갤러리 캡처 비교", "claude", "turn", ["herdr-ide", "gallery-capture", "질문"], 7),
    menuSession(now, "agent-plan", "hide-plan 위임", "codex", "waiting", ["herdr-ide", "main", "대기 · 미션 1/3"], 15),
    menuSession(now, "agent-release", "release 노트 초안", "claude", "done", ["herdr-ide", "main", "끝남"], 30),
  ];
}

/** The composer-with-chips scene (D-85, B49): a session chip and a project chip in the words, and the same words sent. */
export const CHIP_SESSION = { kind: "session" as const, id: "w138:p2", name: "프로젝트 패널 결과 구현", mark: "going" as const, agent_kind: "claude" };
export const CHIP_PROJECT = { kind: "project" as const, id: "~/projects/herdr-ide", name: "herdr-ide" };
export const CHIP_TEXT = "@session(w138:p2 프로젝트 패널 결과 구현) 마무리하고, 끝나면 @project(~/projects/herdr-ide herdr-ide)에 PR 올려줘";

export function chipItems(now: number): DeskItem[] {
  return [{ kind: "user", id: 51, text: "@session(w138:p2 프로젝트 패널 결과 구현) 테스트 다시 돌려줘, @project(~/projects/herdr-ide herdr-ide) 기준으로", at_ms: at(now, 2), mentions: [CHIP_SESSION, CHIP_PROJECT] }];
}

/** The 결과물 기준 scene: a session's View list, and what Desk keeps of it. */
export const SESSION_VIEWS: { label: string; kind: SessionViewKind }[] = [
  { label: "compare.png", kind: "image" },
  { label: "localhost:5173/gallery", kind: "page" },
  { label: "notes.md", kind: "markdown" },
  { label: "PR 1902", kind: "pr" },
  { label: "capture-gallery.mjs", kind: "code" },
  { label: "PaneHeader.tsx diff", kind: "diff" },
  { label: "verify-web.log", kind: "log" },
];
