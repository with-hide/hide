// Invented phone list for the gallery (PRD mobile-companion-ui B1-B9): the
// doop final board's F1 list and F2 device sheet, drawn from production parts.
// Names, places and counts are fixtures, never operator data.

import { HOSTS_KEY } from "../mobile/hosts";
import type { LookContent } from "../mobile/lookContent";
import type { AgentGroup, AgentKey, GroupRow, LookFile, LookRow, PhoneAgent, PhoneDevice, PhoneSleepingSession } from "../mobile/protocol";
import type { Detail } from "../mobile/store";

export const PHONE_ROOM = "a".repeat(43);
export const PHONE_MAC = "호연의 MacBook Pro";

const MINUTE = 60_000;

function agent(pane: string, title: string, ago: number, over: Partial<PhoneAgent> = {}): PhoneAgent {
  return {
    device_id: "local", pane_id: pane, root_pane_id: pane, group: "working", mark: "going", wait: null, descendants_working: 0,
    agent_kind: "claude", title, place: null, device_label: null, changed_at_unix_ms: Date.now() - ago * MINUTE,
    line: null, status_code: "working", demand: "none", emphasized: false, holds_notification: false, ...over,
  };
}

const mini = { device_id: "mini", device_label: "Mac mini" } as const;
const asking = (text: string): PhoneAgent["line"] => ({ text, mode: "request", tone: "warning" });
const row = (value: PhoneAgent): GroupRow => ({ kind: "agent", ...value });

function sleeping(index: number): PhoneSleepingSession {
  return {
    sleep_id: `sleep-gallery-${index}`, device_id: "local", agent_kind: "claude", title: `지난주 정리 ${index}`, group: "seen",
    phase: "sleeping", since_unix_ms: Date.now() - (400 + index) * MINUTE, wake_available: true, checking: false, attention: false,
  };
}

const FOLDED_TITLES = ["강의 영상 자막 점검", "주간 회고 정리", "카톡 수집기 로그 확인", "뉴스레터 초안", "테스트 서버 정리", "릴리스 체크리스트"];

/** The five groups the board shows, with the folds the caps make: 내 차례 3, 작업 중 6 (5 listed), 확인함 24 (3 listed, 9 asleep among the rest). */
export function phoneGroups(): AgentGroup[] {
  const parent = agent("a1", "리뷰 코멘트 반영하기", 3, {
    group: "needs_you", mark: "turn", status_code: "idle", demand: "question", emphasized: true, holds_notification: true, place: "herdr-ide · prd/review",
    line: asking("리뷰 코멘트 3건을 모두 반영할까요?"),
    descendants: { asking: 1, working: 1, done: 1 }, children_open: true,
    children: [
      agent("a1c1", "Windows 경로 테스트 고치기", 1, { group: "working", mark: "turn", status_code: "idle", demand: "question", emphasized: true, line: asking("main에서 새로 시작할까요?") }),
      agent("a1c2", "verify-web 돌리기", 6, { line: { text: "테스트 412 / 980 통과", mode: "kept", tone: "subtle" } }),
      agent("a1c3", "리뷰 답글 문서 정리", 9, { group: "done", mark: "done", status_code: "idle", line: { text: "완료 · 바뀐 파일 2개", mode: "news", tone: "news" } }),
    ],
  });
  const holder = agent("w2", "홀더 메모리 측정", 4, {
    mark: "waiting", wait: "children", descendants_working: 2, descendants: { working: 2, asking: 0, done: 0 },
    line: { text: "", mode: "waiting", tone: "subtle" },
    children: [agent("w2c1", "RSS 기록 모으기", 4), agent("w2c2", "footprint 비교 표", 5)],
  });
  const needsYou: GroupRow[] = [
    row(parent),
    // A person's turn, its place with Copy beside it and Done under it (PRD ask-human B19).
    row(agent("a2", "모닥불 입문반 강의 정산", 5, { ...mini, group: "needs_you", mark: "turn", status_code: "idle", demand: "question", emphasized: true, holds_notification: true, line: asking("환불 2건도 포함할까요?"), human_turn: { id: "a2.1", kind: "auth", place: "https://github.com/login/device?user_code=TEST-1234", pane_id: "a2", title: "모닥불 입문반 강의 정산", device_id: "mini", since_unix_ms: null, own: true } })),
    row(agent("a3", "설치 가이드 다듬기", 11, { group: "needs_you", mark: "turn", status_code: "idle", demand: "question", emphasized: true, holds_notification: true, place: "herdr-ide · main", line: asking("INSTALL.md의 순서를 바꿔도 될까요?") })),
  ];
  const working: GroupRow[] = [
    row(agent("w1", "Slack 스레드 시행착오 점검", 2, { ...mini, place: "modakbul · main" })),
    row(holder),
    row(agent("w3", "Day2 강의자료 준비", 18, { place: "creator · master" })),
    row(agent("w4", "릴리스 노트 초안 검토", 22, { place: "herdr-ide · main" })),
    row(agent("w5", "사이트 배포 점검", 31, { place: "withhide · main" })),
  ];
  const seenRow = (pane: string, title: string, ago: number, over: Partial<PhoneAgent> = {}) =>
    row(agent(pane, title, ago, { group: "seen", mark: "quiet", status_code: "idle", ...over }));
  const seen: GroupRow[] = [
    seenRow("s1", "hide 배치 PR 처리와 보고서", 60, { ...mini, place: "herdr-ide · main" }),
    seenRow("s2", "리눅스 서버 카톡 수집기 이전", 120, { place: "atorni · main" }),
    seenRow("s3", "분기 결산 메모", 180, { place: "notes" }),
  ];
  const seenMore: GroupRow[] = [
    ...Array.from({ length: 12 }, (_, at) => seenRow(`s${at + 4}`, FOLDED_TITLES[at % FOLDED_TITLES.length]!, 240 + at * 20, { place: "notes" })),
    ...Array.from({ length: 9 }, (_, at): GroupRow => ({ kind: "sleeping", ...sleeping(at + 1) })),
  ];
  return [
    { group: "needs_you", count: 3, rows: needsYou },
    { group: "working", count: 6, rows: working, more: [row(agent("w6", "벤치마크 결과 정리", 40, { place: "herdr-ide · perf" }))] },
    { group: "seen", count: 24, rows: seen, more: seenMore, more_asleep: 9 },
  ];
}

/** Each device's counts of top rows, counted from the list as the core counts them (B9), so the sheet cannot disagree with the rows. */
export function phoneDevices(groups: readonly AgentGroup[]): PhoneDevice[] {
  const tally = (group: string, device: string) =>
    groups
      .filter((candidate) => candidate.group === group)
      .flatMap((candidate) => [...candidate.rows, ...(candidate.more ?? [])])
      .filter((top) => top.kind === "agent" && top.device_id === device).length;
  return [
    { device_id: "local", label: null, local: true, connected: true, needs_you: tally("needs_you", "local"), working: tally("working", "local") },
    { device_id: "mini", label: "Mac mini", connected: true, needs_you: tally("needs_you", "mini"), working: tally("working", "mini") },
    { device_id: "old", label: "Linux 서버", connected: false, needs_you: 0, working: 0 },
  ];
}

/** The one paired Mac the hosted phone's picker lists, written before the sheet reads it. */
export function seedPairedMac(): void {
  try {
    localStorage.setItem(HOSTS_KEY, JSON.stringify([{ room: PHONE_ROOM, name: PHONE_MAC, noiseKey: "b".repeat(43), credential: "gallery", protocol: 2, lastSeen: null }]));
  } catch {
    // The picker then says storage is unavailable, which is the state it draws.
  }
}

/** The scenes of the 볼 것 tab and its viewers (PRD mobile-companion-ui B10-B19, B23, B26, B27), in the order the board draws them. */
export type PhoneLookScene = "phone-looks" | "phone-look-changes" | "phone-look-doc" | "phone-look-image" | "phone-look-shot" | "phone-look-shot-refreshing" | "phone-look-page" | "phone-look-other" | "phone-look-large" | "phone-look-html-large" | "phone-look-failed" | "phone-look-closed" | "phone-look-device" | "phone-look-loading";

export const PHONE_LOOK_SCENES: readonly PhoneLookScene[] = ["phone-looks", "phone-look-changes", "phone-look-doc", "phone-look-image", "phone-look-shot", "phone-look-shot-refreshing", "phone-look-page", "phone-look-other", "phone-look-large", "phone-look-html-large", "phone-look-failed", "phone-look-closed", "phone-look-device", "phone-look-loading"];

const LOOK_AGENT = "l1";

/** The one agent the scenes open: its row carries the eye mark (B11). */
export function phoneLookGroups(): AgentGroup[] {
  const parent = agent(LOOK_AGENT, "PR 2090 리뷰 반영", 3, {
    group: "needs_you", mark: "look", status_code: "idle", emphasized: true, place: "hide · fix/review-2090",
    line: { text: "경로 구분자 수정", mode: "request", tone: "warning" },
  });
  return [{ group: "needs_you", count: 1, rows: [row(parent)] }];
}

const KEY: AgentKey = { device_id: "local", pane_id: LOOK_AGENT };

const INSTALL_GUIDE = `# Linux에 설치하기

Hide는 Ubuntu 24.04 이상과 Fedora 40 이상에서 돌아가요.

## 1. 받기

\`\`\`sh
curl -fsSL https://withhide.dev/install.sh | sh
\`\`\`

## 2. 폰 연결

설정 › 모바일에서 **폰에서 Hide 열기**를 켜고 QR을 폰 카메라로 찍으세요. 로그인이나 계정은 필요 없어요.

## 3. 지우기

1. \`hide uninstall\`을 실행해요.
2. 남은 상태 폴더 \`~/.hide\`를 지워요.

| 시스템 | 지원 |
| --- | --- |
| Ubuntu 24.04 | 예 |
| Fedora 40 | 예 |
`;

/** A chart drawn as an image, so the viewer has pixels to show without a file. */
const CHART = `data:image/svg+xml;utf8,${encodeURIComponent(
  '<svg xmlns="http://www.w3.org/2000/svg" width="2400" height="1400" viewBox="0 0 240 140"><rect width="240" height="140" fill="white"/><text x="10" y="14" font-size="8" font-family="sans-serif" fill="black">홀더 메모리 (MB) · 탭 50개 열고 닫기 10회</text><g stroke="lightgray" stroke-width=".5"><path d="M10 40H230M10 70H230M10 100H230"/></g><path d="M10 100C60 70 120 66 230 36" fill="none" stroke="orangered" stroke-width="1.5"/><path d="M10 104C80 90 150 92 230 90" fill="none" stroke="royalblue" stroke-width="1.5"/></svg>',
)}`;

/** A landing page as the desktop app showed it, drawn as an image so the viewer has pixels without a browser. */
const PAGE_SHOT = `data:image/svg+xml;utf8,${encodeURIComponent(
  '<svg xmlns="http://www.w3.org/2000/svg" width="1200" height="1500" viewBox="0 0 400 500"><rect width="400" height="500" fill="ivory"/><text x="20" y="34" font-size="16" font-weight="700" font-family="sans-serif" fill="black">hide</text><text x="380" y="34" font-size="11" text-anchor="end" font-family="sans-serif" fill="black">문서 · 다운로드</text><text x="20" y="110" font-size="26" font-weight="700" font-family="sans-serif" fill="black">에이전트 여럿을</text><text x="20" y="144" font-size="26" font-weight="700" font-family="sans-serif" fill="black">한 화면에서</text><text x="20" y="180" font-size="12" font-family="sans-serif" fill="dimgray">Claude Code, Codex, OpenCode를 나란히 돌리고,</text><text x="20" y="198" font-size="12" font-family="sans-serif" fill="dimgray">누가 나를 기다리는지 한눈에 봐요.</text><rect x="20" y="220" width="104" height="36" rx="18" fill="black"/><text x="72" y="243" font-size="12" text-anchor="middle" font-family="sans-serif" fill="white">Mac용 받기</text><rect x="20" y="290" width="360" height="170" rx="10" fill="black"/><g fill="darkslategray"><rect x="32" y="304" width="190" height="10" rx="3"/><rect x="32" y="324" width="270" height="10" rx="3"/><rect x="32" y="364" width="220" height="10" rx="3"/><rect x="32" y="384" width="170" height="10" rx="3"/></g><rect x="32" y="344" width="126" height="10" rx="3" fill="orange"/></svg>',
)}`;

export function phoneLooks(): LookRow[] {
  return [
    { id: "v1", kind: "changes", name: "경로 구분자 수정", unseen: true, at: Date.now() - 3 * MINUTE },
    { id: "v2", kind: "image", name: "memory-holder.png", unseen: true, at: Date.now() - 5 * MINUTE },
    { id: "v3", kind: "page", name: "랜딩 미리보기", unseen: true, at: Date.now() - 8 * MINUTE, url: "https://withhide.dev/preview" },
    { id: "v7", kind: "html", name: "설정 화면 목업.html", unseen: true, at: Date.now() - 9 * MINUTE },
    { id: "v4", kind: "doc", name: "INSTALL.md", unseen: false, at: Date.now() - 60 * MINUTE },
  ];
}

const FILES: LookFile[] = [
  { path: "hide-platform/src/path.rs", status: "modified", added: 6, removed: 2, binary: false },
  { path: "hided/src/mobile/wire.rs", status: "modified", added: 71, removed: 8, binary: false },
  { path: "hide-platform/tests/it/path.rs", status: "untracked", added: null, removed: null, binary: false },
  { path: "docs/diagram.pdf", status: "untracked", added: null, removed: null, binary: true },
];

const PAGE_ADDRESS = "https://withhide.dev/preview";
const VIEWED: Record<Exclude<PhoneLookScene, "phone-looks">, { id: string; content: LookContent; refreshing?: boolean }> = {
  "phone-look-changes": { id: "v1", content: { state: "changes", files: FILES, count: 4, added: 77, removed: 10 } },
  "phone-look-doc": { id: "v4", content: { state: "text", markdown: true, text: INSTALL_GUIDE, truncated: false } },
  "phone-look-image": { id: "v2", content: { state: "image", url: CHART, size: 319_488 } },
  "phone-look-shot": { id: "v3", content: { state: "shot", url: PAGE_SHOT, size: 412_160, address: PAGE_ADDRESS, takenAt: Date.now() - 20_000, takenBy: "local" } },
  "phone-look-shot-refreshing": { id: "v3", refreshing: true, content: { state: "shot", url: PAGE_SHOT, size: 412_160, address: PAGE_ADDRESS, takenAt: Date.now() - 95_000, takenBy: "local" } },
  "phone-look-page": { id: "v3", content: { state: "page", url: PAGE_ADDRESS } },
  "phone-look-other": { id: "v5", content: { state: "other" } },
  "phone-look-large": { id: "v6", content: { state: "too_large" } },
  "phone-look-html-large": { id: "v7", content: { state: "too_large" } },
  "phone-look-failed": { id: "v1", content: { state: "failed" } },
  "phone-look-closed": { id: "v1", content: { state: "closed" } },
  "phone-look-device": { id: "v1", content: { state: "device_unreachable" } },
  "phone-look-loading": { id: "v2", content: { state: "loading" } },
};

/** The detail the scene seeds: the 볼 것 list, or one of its Views opened with the content the state names. */
export function phoneLookDetail(scene: PhoneLookScene): Detail {
  const rows = phoneLooks();
  const extra: LookRow[] = [
    { id: "v5", kind: "other", name: "architecture.pdf", unseen: true, at: Date.now() - 12 * MINUTE },
    { id: "v6", kind: "image", name: "screen-recording-poster.png", unseen: true, at: Date.now() - 14 * MINUTE },
  ];
  const base: Detail = { key: KEY, view: "conversation", tab: "looks", rows: null, moreAskedAt: null, conversation: { messages: [], before: null }, olderAskedAt: null, looks: { rows, omitted: 0 }, look: null };
  if (scene === "phone-looks") return base;
  const viewed = VIEWED[scene];
  const row = [...rows, ...extra].find((candidate) => candidate.id === viewed.id)!;
  return { ...base, looks: { rows: [...rows, ...extra], omitted: 0 }, look: { row, requestId: `gallery-${scene}`, content: viewed.content, shown: true, refreshing: viewed.refreshing ?? false } };
}
