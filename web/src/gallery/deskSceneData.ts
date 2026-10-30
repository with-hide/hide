// Invented Desk values for the gallery's Desk scenes (PRD
// session-docs-and-assistant, Desk 1차): agents, a conversation, jobs in every
// state, threads as ACP updates and outputs. Names, counts and paths are
// made up; nothing here is read from a machine.
import type { AcpSessionUpdate, DeskAgent, DeskItem, DeskJob, DeskMention, DeskMission, DeskOutput, ThreadHidePart } from "../desk";
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

export const AGENTS: DeskAgent[] = [
  { id: "claude", label: "Claude", state: "ready", models: [{ id: "opus", label: "Opus 5.5" }, { id: "sonnet", label: "Sonnet 5.5" }], efforts: ["low", "medium", "high"] },
  { id: "codex", label: "Codex", state: "ready", models: [{ id: "gpt", label: "GPT-6" }, { id: "gpt-mini", label: "GPT-6 mini" }], efforts: ["low", "medium", "high"] },
  { id: "opencode", label: "OpenCode", state: "login_needed", login_command: "opencode auth login", models: [{ id: "default", label: "기본" }], efforts: ["medium"] },
  { id: "pi", label: "Pi", state: "no_ask_mode", models: [{ id: "default", label: "기본" }], efforts: ["medium"] },
];

export const AGENTS_PREPARING: DeskAgent[] = AGENTS.map((agent) => (agent.id === "codex" ? { ...agent, state: "preparing" } : agent));

const output = (id: string, kind: DeskOutput["kind"], label: string, seen: boolean, thumb: string | null = null): DeskOutput => ({ id, kind, label, seen, thumb });

export const OUTPUTS = {
  compareLight: output("o-compare-light", "image", "compare-light.png", false, PICTURES.light),
  compareDark: output("o-compare-dark", "image", "compare-dark.png", false, PICTURES.dark),
  notes: output("o-notes", "doc", "notes.md", true),
  beforeAfter: output("o-before-after", "doc", "before-after.md", false),
  galleryPage: output("o-gallery-page", "page", "gallery · desk", true, PICTURES.page),
  pr: output("o-pr-41", "pr", "PR #41 자막 수정", true),
  reelCover: output("o-reel-cover", "image", "cover.png", true, PICTURES.light),
};

export const MISSION: DeskMission = {
  id: "mission-desk", title: "Desk 1차", status: "running", done: 2, total: 4, tone: "working", deciding: false,
  nodes: [
    { key: "1a", title: "1a Desk 방 · 리뷰 대기", tone: "review" },
    { key: "1b", title: "1b 스레드 화면 · 작업 중", tone: "working" },
    { key: "1d", title: "1d lane · 1b 기다림", tone: "waiting" },
    { key: "docs", title: "문서 정리 · 대기", tone: "waiting" },
  ],
};

const base = (now: number) => (id: string, title: string, extra: Partial<DeskJob>): DeskJob => ({
  id, title, agent_id: `agent-${id}`, pane_id: `w1:${id}`,
  project: { workspace_id: "hide", label: "hide" },
  location: { kind: "checkout", label: "hide" },
  state: "working", pull_requests: [], outputs: [], joined: false, disconnected: false,
  agent_kind: "claude", changed_at_unix_ms: now - 5 * MINUTE, read: false,
  ...extra,
});

/** The jobs of the room's main story (scenes 2-9). */
export function storyJobs(now: number): Record<string, DeskJob> {
  const job = base(now);
  const jobs: DeskJob[] = [
    job("mention", "mention 버튼 정리", { location: { kind: "worktree", label: "desk/mention-button" }, now: "버튼 정리 → 테스트 돌리는 중", plan: { done: 6, total: 14 }, pull_requests: [{ number: 1903, state: "pending" }], outputs: [OUTPUTS.compareLight, OUTPUTS.compareDark, OUTPUTS.beforeAfter], changed_at_unix_ms: now - 12 * MINUTE }),
    job("gallery", "갤러리 캡처 비교", { state: "asking", question: "다크 모드도 찍을까요?", outputs: [OUTPUTS.compareLight], changed_at_unix_ms: now - 3 * MINUTE }),
    job("release", "release 노트 초안", { state: "done", summary: "초안을 notes.md에 썼어요", outputs: [OUTPUTS.notes], agent_kind: "codex", changed_at_unix_ms: now - 62 * MINUTE }),
    job("reel", "reel 자막 수정", { state: "done", read: true, project: { workspace_id: "studio", label: "studio" }, location: { kind: "checkout", label: "studio" }, summary: "자막 싱크를 고치고 PR을 올렸어요", pull_requests: [{ number: 41, state: "merged" }], outputs: [OUTPUTS.reelCover, OUTPUTS.pr], agent_kind: "codex", changed_at_unix_ms: now - 125 * MINUTE }),
    job("conductor", "Desk 1차", { now: "Conductor · 1a 리뷰 중, 1b 작업 중", mission: MISSION, workers: ["1a", "1b"], changed_at_unix_ms: now - 22 * MINUTE }),
    job("1a", "1a Desk 방", { state: "done", read: true, location: { kind: "worktree", label: "feat/desk-host" }, summary: "PR 리뷰 대기", pull_requests: [{ number: 1905, state: "pending" }], agent_kind: "codex", changed_at_unix_ms: now - 30 * MINUTE }),
    job("1b", "1b 스레드 화면", { location: { kind: "worktree", label: "feat/desk-thread" }, now: "스레드 화면 그리기 → 테스트", plan: { done: 3, total: 9 }, changed_at_unix_ms: now - 4 * MINUTE }),
  ];
  return Object.fromEntries(jobs.map((one) => [one.id, one]));
}

/** One card per state the cards draw (B16-B24, B36-B39). */
export function everyStateJobs(now: number): DeskJob[] {
  const job = base(now);
  return [
    job("s-queued", "문서 정리", { state: "queued", queue_position: 2 }),
    job("s-starting", "릴리스 체크리스트", { state: "starting", location: { kind: "worktree", label: "desk/release-checklist" } }),
    job("s-working", "mention 버튼 정리", { location: { kind: "worktree", label: "desk/mention-button" }, now: "버튼 정리 → 테스트 돌리는 중", plan: { done: 6, total: 14 }, pull_requests: [{ number: 1903, state: "pending" }], outputs: [OUTPUTS.compareLight, OUTPUTS.beforeAfter] }),
    job("s-asking", "갤러리 캡처 비교", { state: "asking", question: "다크 모드도 찍을까요? 찍는다면 라이트와 같은 크기로 잘라 둘지도 알려 주세요", outputs: [OUTPUTS.compareLight] }),
    job("s-done", "release 노트 초안", { state: "done", summary: "초안을 notes.md에 썼어요. 바뀐 화면 세 개와 고친 버그 두 개를 정리했어요", pull_requests: [{ number: 1910, state: "mergeable" }], outputs: [OUTPUTS.notes, OUTPUTS.galleryPage], agent_kind: "codex" }),
    job("s-failed", "로그 정리", { state: "failed", reason: "spawn_failed" }),
    job("s-exited", "의존성 올리기", { state: "failed", reason: "exited", location: { kind: "worktree", label: "desk/bump-deps" } }),
    job("s-cancelled", "아이콘 크기 맞추기", { state: "cancelled" }),
    job("s-closed", "설정 화면 문구", { state: "closed" }),
    job("s-moved", "단축키 표 정리", { state: "moved_out" }),
    job("s-archived", "README 고치기", { state: "archived", summary: "README의 설치 절을 고쳤어요" }),
    job("s-joined", "PaneHeader 아이콘", { joined: true, now: "아이콘을 14로 줄이는 중", plan: { done: 2, total: 3 } }),
    job("s-mini", "mini에서 e2e 돌리기", { machine: "mini", disconnected: true, now: "e2e 돌리는 중", plan: { done: 4, total: 7 } }),
    job("s-mini-failed", "mini에서 캡처", { state: "failed", reason: "device_disconnected", machine: "mini", disconnected: true }),
    job("s-conductor", "Desk 1차", { now: "Conductor · 1a 리뷰 중, 1b 작업 중", mission: MISSION }),
  ];
}

export const MENTIONS: DeskMention[] = [
  { kind: "session", name: "record-now", id: "agent-record-now" },
  { kind: "session", name: "1a-desk-room", id: "agent-1a" },
  { kind: "project", name: "hide", id: "hide" },
  { kind: "project", name: "studio", id: "studio" },
];

const at = (now: number, minutes: number) => now - minutes * MINUTE;

/** Scene 1-1: a conversation that changed agents and then model. */
export function switchedItems(now: number): DeskItem[] {
  return [
    { kind: "user", id: "i1", text: "오늘 끝난 거 정리해줘", at: at(now, 30), mentions: [] },
    { kind: "reply", id: "i2", text: "세 개 끝났어요: release 노트, 갤러리 비교, reel 자막. 갤러리 비교는 다크 캡처가 남았어요.", at: at(now, 29), state: "done" },
    { kind: "switch", id: "i3", from: "Claude", to: "Codex", carried: true },
    { kind: "user", id: "i4", text: "그럼 다크 캡처는 이어서 맡겨줘", at: at(now, 20), mentions: [] },
    { kind: "reply", id: "i5", text: "갤러리 비교 스레드에 다크 모드 캡처를 이어서 하라고 전했어요.", at: at(now, 19), state: "done" },
    { kind: "model", id: "i6", model: "GPT-6 mini", effort: "medium" },
  ];
}

/** Scene 2: a hand-off that started three threads. */
export function handoffItems(now: number): DeskItem[] {
  return [
    { kind: "user", id: "h1", text: "@hide mention 버튼 정리, 갤러리 캡처 비교, release 노트 초안 맡겨줘", at: at(now, 15), mentions: [MENTIONS[2]!] },
    { kind: "reply", id: "h2", text: "스레드 세 개를 띄웠어요. mention 버튼은 PR로 낼 일이라 새 워크트리에서 해요.", at: at(now, 15), state: "done" },
    { kind: "job", id: "h3", job_id: "mention" },
    { kind: "job", id: "h4", job_id: "gallery" },
    { kind: "job", id: "h5", job_id: "release" },
  ];
}

/** Scenes 3-1 and 3-2: a task handed to a session already on it. */
export function mentionItems(now: number): DeskItem[] {
  return [
    { kind: "user", id: "m1", text: "@record-now 갤러리 숫자 줄인 거 다크에서도 확인해줘", at: at(now, 6), mentions: [MENTIONS[0]!] },
    { kind: "reply", id: "m2", text: "record-now에 편지로 맡겼어요.", at: at(now, 6), state: "done" },
    { kind: "handed", id: "m3", agent_id: "agent-record-now", name: "record-now", project: "hide" },
    { kind: "user", id: "m4", text: "@studio 릴스 썸네일 3안 만들어줘", at: at(now, 2), mentions: [MENTIONS[3]!] },
  ];
}

/** Scene 9: the mission hand-off. */
export function missionItems(now: number): DeskItem[] {
  return [
    { kind: "user", id: "x1", text: "Desk 1차를 미션으로 돌려줘", at: at(now, 25), mentions: [] },
    { kind: "reply", id: "x2", text: "Conductor 스레드를 띄워 미션으로 돌려요. 노드 네 개, PR 모드예요.", at: at(now, 25), state: "done" },
    { kind: "job", id: "x3", job_id: "conductor" },
  ];
}

/** The chat's other states (B5-B8, B11, B13-B15): approvals in each answer and every notice. */
export function stateItems(now: number): DeskItem[] {
  return [
    { kind: "user", id: "a1", text: "mention 버튼 브랜치 정리하고 테스트 다시 돌려줘", at: at(now, 9), mentions: [] },
    { kind: "approval", id: "a2", title: "셸 명령", command: "git -C ~/projects/hide/.worktrees/desk-mention-button rebase --abort && bash scripts/verify-web.sh --filter PaneHeader", folder: "~/projects/hide/.worktrees/desk-mention-button", reason: "멈춘 리베이스를 정리해야 테스트를 다시 돌릴 수 있어요", state: "waiting" },
    { kind: "approval", id: "a3", title: "파일 수정", command: "edit web/src/PaneHeader.tsx", folder: "~/projects/hide", reason: "hover 조건을 고쳐야 해요", state: "allowed" },
    { kind: "approval", id: "a4", title: "셸 명령", command: "rm -rf web/dist", folder: "~/projects/hide", reason: "빌드 결과를 지우고 새로 만들게요", state: "rejected" },
    { kind: "approval", id: "a5", title: "셸 명령", command: "pnpm --dir web test", folder: "~/projects/hide", reason: "테스트를 돌려 볼게요", state: "closed" },
    { kind: "model", id: "a6", model: "Sonnet 5.5", effort: "low" },
    { kind: "switch", id: "a7", from: "Claude", to: "Codex", carried: false },
    { kind: "notice", id: "n1", notice: "preparing", agent: "Codex" },
    { kind: "notice", id: "n2", notice: "install_failed", agent: "Codex", detail: "네트워크에 연결할 수 없어요", action: "reinstall" },
    { kind: "notice", id: "n3", notice: "login_needed", agent: "OpenCode", detail: "opencode auth login" },
    { kind: "notice", id: "n4", notice: "blocked", action: "retry" },
    { kind: "notice", id: "n5", notice: "host_stopped", action: "restart" },
    { kind: "notice", id: "n6", notice: "resume_failed" },
    { kind: "reply", id: "a8", text: "정리하고 테스트를 다시 돌렸어요. 실패 하나는", at: at(now, 1), state: "stopped" },
    { kind: "user", id: "a9", text: "지금 돌고 있는 일들 어디까지 왔어요?", at: at(now, 0), mentions: [] },
  ];
}

const text = (sessionUpdate: "user_message_chunk" | "agent_message_chunk" | "agent_thought_chunk", words: string): AcpSessionUpdate => ({ sessionUpdate, content: { type: "text", text: words } });

/** Scene 4: the mention-button thread, as the session's record reads. */
export const MENTION_THREAD: AcpSessionUpdate[] = [
  text("user_message_chunk", "PaneHeader의 @ 버튼이 늘 보여서 거슬려요. hover에서만 보이게 정리하고 PR까지 내 주세요"),
  text("agent_thought_chunk", "showMention 상태는 pane hover 상태로 대신할 수 있다. 테스트는 PaneHeader 필터로 충분하다."),
  text("agent_message_chunk", "PaneHeader의 @ 버튼을 hover에서만 보이게 바꿀게요."),
  { sessionUpdate: "plan", entries: [
    { content: "버튼 조건 바꾸기", priority: "high", status: "completed" },
    { content: "테스트 돌리기", priority: "high", status: "in_progress" },
    { content: "PR 내기", priority: "medium", status: "pending" },
  ] },
  { sessionUpdate: "tool_call", toolCallId: "t-edit", title: "web/src/PaneHeader.tsx", kind: "edit", status: "pending", content: [{ type: "diff", path: "web/src/PaneHeader.tsx", oldText: "  <TitleLine />\n  {showMention && <MentionButton />}\n  <PaneMenu />", newText: "  <TitleLine />\n  {hovered && <MentionButton />}\n  <PaneMenu />" }], locations: [{ path: "web/src/PaneHeader.tsx" }] },
  { sessionUpdate: "tool_call_update", toolCallId: "t-edit", status: "completed" },
  { sessionUpdate: "tool_call", toolCallId: "t-read", title: "web/src/PaneHeader.test.tsx", kind: "read", status: "completed" },
  { sessionUpdate: "tool_call", toolCallId: "t-test", title: "Bash", kind: "execute", status: "in_progress", rawInput: { command: "bash scripts/verify-web.sh --filter PaneHeader" } },
  { sessionUpdate: "tool_call_update", toolCallId: "t-test", content: [{ type: "content", content: { type: "text", text: "✓ PaneHeader shows @ on hover (12 ms)\n✓ PaneHeader hides @ at rest (8 ms)\n… running 14 more" } }] },
];

/** Scene 4-1: a Conductor's thread, its `hide` calls read into Hide's parts. */
export function conductorThread(jobs: Record<string, DeskJob>): { updates: AcpSessionUpdate[]; parts: Record<string, ThreadHidePart> } {
  const plan = planFixture("running", Date.now());
  return {
    updates: [
      text("user_message_chunk", "Desk 1차를 미션으로 돌려줘"),
      text("agent_message_chunk", "계획을 올리고 @1a-desk-room에 첫 층을 맡길게요."),
      { sessionUpdate: "tool_call", toolCallId: "h-plan", title: "Bash", kind: "execute", status: "completed", rawInput: { command: "hide plan set …" } },
      { sessionUpdate: "tool_call", toolCallId: "h-spawn", title: "Bash", kind: "execute", status: "completed", rawInput: { command: "hide agent spawn …" } },
      { sessionUpdate: "tool_call", toolCallId: "h-letter", title: "Bash", kind: "execute", status: "completed", rawInput: { command: "hide request send …" } },
      { sessionUpdate: "tool_call", toolCallId: "h-view", title: "Bash", kind: "execute", status: "completed", rawInput: { command: "hide browser open …" } },
      { sessionUpdate: "tool_call", toolCallId: "h-mission", title: "Bash", kind: "execute", status: "completed", rawInput: { command: "hide mission show …" } },
      text("agent_message_chunk", "1b가 끝나면 lane 노드를 띄울게요."),
    ],
    parts: {
      "h-plan": { kind: "plan", plan },
      "h-spawn": { kind: "spawn", job: { ...jobs["1b"]!, now: "시작 중", state: "starting" } },
      "h-letter": { kind: "letter", to: "1a-desk-room", body: "리뷰 지적 2개 반영해서 다시 올려줘" },
      "h-view": { kind: "view", output: OUTPUTS.galleryPage },
      "h-mission": { kind: "mission", mission: MISSION },
    },
  };
}

/** Scene 7: the gallery thread, asking. */
export const GALLERY_THREAD: AcpSessionUpdate[] = [
  text("user_message_chunk", "갤러리 캡처를 라이트·다크로 비교해 주세요"),
  { sessionUpdate: "tool_call", toolCallId: "g-capture", title: "Bash", kind: "execute", status: "completed", rawInput: { command: "node scripts/capture-gallery.mjs --theme light" }, content: [{ type: "content", content: { type: "text", text: "6 captures → agents/runs/compare/light/" } }] },
  text("agent_message_chunk", "라이트 모드 캡처 6장을 찍었어요."),
  text("agent_message_chunk", "\n다크 모드도 찍을까요?"),
];

/** Scenes 5 and 6: the release-notes thread, done. */
export const RELEASE_THREAD: AcpSessionUpdate[] = [
  text("user_message_chunk", "v1.0.5 release 노트 초안을 써 주세요"),
  { sessionUpdate: "tool_call", toolCallId: "r-write", title: "notes.md", kind: "edit", status: "completed", content: [{ type: "diff", path: "notes.md", oldText: null, newText: "# v1.0.5 노트 초안\n\n- 스레드 입력칸 추가\n- 결과물은 세션의 View 목록에서 모음" }] },
  text("agent_message_chunk", "초안을 notes.md에 썼어요."),
];

/** Scene 9: the worker thread beside the map. */
export const WORKER_THREAD: AcpSessionUpdate[] = [
  { sessionUpdate: "user_message_chunk", content: { type: "text", text: "1b 스레드 화면을 맡아 주세요. 공용 부품은 shared PR에 있어요." }, _meta: { hide: { origin: "letter" } } },
  text("agent_message_chunk", "ACP 업데이트를 태스크 줄 부품으로 그리는 중이에요."),
  { sessionUpdate: "tool_call", toolCallId: "w-edit", title: "web/src/desk/ThreadView.tsx", kind: "edit", status: "in_progress" },
];

export const NOTES_MD = "# v1.0.5 노트 초안\n\n- 스레드 입력칸 추가\n- 결과물은 세션의 View 목록에서 사실로 모음\n- Desk 방의 rail에 Threads와 결과물\n";

export const MENU_SESSIONS: MentionSession[] = [
  { id: "agent-record-now", name: "record-now", agent_kind: "claude", mark: "going", detail: "hide · 작업 중" },
  { id: "agent-release", name: "release 노트 초안", agent_kind: "codex", mark: "done", detail: "hide · 끝남" },
  { id: "agent-reel-review", name: "reel 자막 리뷰", agent_kind: "codex", mark: "turn", detail: "studio · 질문" },
];

export const MENU_PROJECTS: MentionProject[] = [
  { id: "hide", name: "hide", sessions: 5 },
  { id: "studio", name: "studio", sessions: 2 },
];

export const ENTERED_SESSIONS: MentionSession[] = [
  { id: "agent-record-now", name: "record-now", agent_kind: "claude", mark: "going", detail: "작업 중" },
  { id: "agent-mention", name: "mention 버튼 정리", agent_kind: "claude", mark: "going", detail: "계획 6/14" },
  { id: "agent-gallery", name: "갤러리 캡처 비교", agent_kind: "claude", mark: "turn", detail: "질문" },
  { id: "agent-plan", name: "hide-plan 위임", agent_kind: "codex", mark: "waiting", detail: "대기 · 미션 1/3" },
  { id: "agent-release", name: "release 노트 초안", agent_kind: "codex", mark: "done", detail: "끝남" },
];

/** The 결과물 기준 scene: a session's View list, and what Desk keeps of it. */
export const SESSION_VIEWS: { label: string; kind: import("../desk").SessionViewKind }[] = [
  { label: "compare.png", kind: "image" },
  { label: "localhost:5173/gallery", kind: "page" },
  { label: "notes.md", kind: "markdown" },
  { label: "PR 1902", kind: "pr" },
  { label: "capture-gallery.mjs", kind: "code" },
  { label: "PaneHeader.tsx diff", kind: "diff" },
  { label: "verify-web.log", kind: "log" },
];
