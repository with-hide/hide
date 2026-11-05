// Invented phone list for the gallery (PRD mobile-companion-ui B1-B9): the
// doop final board's F1 list and F2 device sheet, drawn from production parts.
// Names, places and counts are fixtures, never operator data.

import { HOSTS_KEY } from "../mobile/hosts";
import type { AgentGroup, GroupRow, PhoneAgent, PhoneDevice, PhoneSleepingSession } from "../mobile/protocol";

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
    row(agent("a2", "모닥불 입문반 강의 정산", 5, { ...mini, group: "needs_you", mark: "turn", status_code: "idle", demand: "question", emphasized: true, holds_notification: true, line: asking("환불 2건도 포함할까요?") })),
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
