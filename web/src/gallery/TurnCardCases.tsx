// The turn card's later half over fixed, invented wire values shaped like
// three real sessions: a Conductor whose request is buried by mission
// letters, a delegation that hit a block, and a child's question its parent
// decided; the fourth opens the Conductor's whole answer.
import { useEffect } from "react";
import { TurnCard } from "../PaneTurnCard";
import { TooltipProvider } from "../components/ui/tooltip";
import type { AgentRow, PaneHeader, SnapshotRest } from "../snapshot";
import { useShellStore } from "../store";

const AT = new Date(2026, 10, 9, 19, 18).getTime();
const min = (minutes: number) => AT + minutes * 60_000;
const named = (name: string) => ({ kind: "named" as const, name });

const WHOLE = `네, 실제로 생길 수 있습니다. 다만 흔한 경우는 아닙니다. Claude는 처음 실행될 때 자기 폴더(~/.claude)를 만듭니다. 그런데 프로필로만 써 온 사람은 기본 폴더가 아니라 프로필 폴더만 갖게 됩니다.

| 상황 | 예 |
| --- | --- |
| 새 기기를 프로필로만 씀 | 처음부터 "회사" 프로필로만 로그인함 |
| 계정을 일부러 전부 나눔 | 기본 계정은 쓰지 않음 |
| 기본 폴더를 지움 | ~/.claude를 삭제함 |

같은 원인으로 반대 방향 문제도 있습니다.

- 기본 계정이 준비 안 됨: 프로필은 준비됐는데도 시작이 거절됩니다.
- 기본 계정만 준비됨: 프로필 쪽 훅이 빠져 있어도 그냥 시작됩니다.`;

type Case = { id: string; label: string; header: PaneHeader; request: NonNullable<AgentRow["request"]>; opened?: boolean };

const CONDUCTOR: Case = {
  id: "conductor",
  label: "미션 지휘 · 편지 많음 · 묻힌 질문",
  header: { working: true, notice: null, band: null, progress: "reopen은 자리 대기, windows는 Windows CI 확인 대기", request: { text: "프로필만 있는 상황 질문", images: 0 } },
  request: {
    verb: "working", verb_since_unix_ms: AT, pull_requests: [], recent_turns: [],
    request: { text: "기본 Claude 계정 폴더 없이 프로필 계정만 있는 상황이 있을 수 있는거야? 어떤 상황이지", cut: false, images: 0, at_unix_ms: AT, sender: { kind: "operator" } },
    request_summary: "기본 Claude 계정 폴더 없이 프로필 계정만 있는 상황이 있을 수 있는거야? 어떤 상황이지",
    later_by: named("m41-review-windows"),
    answer: { offset: 1, end: "done", line: "프로필만 쓰는 경우 실제로 있음, 시작 점검이 기본 계정만 봐서 생김", head: "네, 실제로 생길 수 있습니다. 다만 흔한 경우는 아닙니다. Claude는 처음 실행될 때 자기 폴더(~/.claude)를 만듭니다.\n같은 원인으로 반대 방향 문제도 있습니다.", whole: true, at_unix_ms: AT + 40_000 },
    letters: {
      total: 5, progress: 10,
      recent: [
        { direction: "sent", peer: named("m41-together-x"), line: "디스크 여유 5.3GB입니다. 판정 끝난 실행의 상태 폴더·사본을 지워 주세요", at_unix_ms: min(4) },
        { direction: "received", peer: named("m41-review-windows"), mark: "report", line: "로컬 732160b86 증분 리뷰 FAIL: 일반 1건", at_unix_ms: min(5) },
        { direction: "sent", peer: named("m41-windows-x"), mark: "waiting_answer", line: "리뷰어 732160b86 증분 FAIL 1건 전달", at_unix_ms: min(5) + 20_000 },
        { direction: "received", peer: named("m41-review-windows"), mark: "report", line: "로컬 551e5c8c4 추가 delta 승인", at_unix_ms: min(6) + 30_000 },
        { direction: "received", peer: named("m41-review-windows"), mark: "reply", line: "방향 승인합니다", at_unix_ms: min(7) },
      ],
    },
    asked: [{ line: "닫은 탭 결함 어떻게 할지 정해 주세요", at_unix_ms: min(6), current: false }],
  },
};

const CASES: Case[] = [
  CONDUCTOR,
  {
    id: "delegation",
    label: "직접 작업 · 위임한 자식이 막힘",
    header: { working: true, notice: null, band: null, request: { text: "머지 후 자동 미션 판단 작업, UI 포함 PR 생성", images: 0 } },
    request: {
      verb: "working", verb_since_unix_ms: AT, pull_requests: [], recent_turns: [], later_by: null,
      request: { text: "ㅇㅇ 머지하고... 자동 미션판단 작업하고 UI 포함해서 PR까지 올려", cut: false, images: 0, at_unix_ms: min(-12), sender: { kind: "operator" } },
      request_summary: "머지 후 자동 미션 판단 작업, UI 포함 PR 생성",
      letters: {
        total: 6, progress: 3,
        recent: [
          { direction: "started", peer: named("auto-mission-impl"), at_unix_ms: min(-5) },
          { direction: "sent", peer: named("auto-mission-impl"), line: "계획 승인. 마지막 sasu verify는 돌리기 전에 허락을 받으세요", at_unix_ms: min(-4) },
          { direction: "received", peer: named("auto-mission-impl"), mark: "reply", line: "확인했습니다. 최종 sasu verify는 허락 전 실행하지 않겠습니다", at_unix_ms: min(-1) },
          { direction: "received", peer: named("auto-mission-impl"), mark: "block", line: "새 세션에 이전 자동 미션 설정이 남는 결함, 플러그인 계약 범위 확인 필요", at_unix_ms: min(11) },
          { direction: "sent", peer: named("auto-mission-impl"), line: "플러그인 계약 변경은 하지 않습니다(범위 확장 거절)", at_unix_ms: min(12) },
          { direction: "received", peer: named("auto-mission-impl"), mark: "question", line: "재시작 한계를 두 문서에 반영해 de514ae0a로 커밋했습니다", at_unix_ms: min(13) },
        ],
      },
    },
  },
  {
    id: "child-question",
    label: "자식이 묻고 내 에이전트가 대신 결정",
    header: { working: true, notice: null, band: null, request: { text: "미션/직접 선택 질문 + hide-mission 안내", images: 0 } },
    request: {
      verb: "working", verb_since_unix_ms: AT, pull_requests: [], recent_turns: [], later_by: null,
      request: { text: "미션으로 할지 직접할지 물어보게 + 처음 시작할 때 복잡한 작업은 hide-mission 안내", cut: false, images: 0, at_unix_ms: min(-92), sender: { kind: "operator" } },
      request_summary: "미션으로 할지 직접할지 물어보게 + 처음 시작할 때 복잡한 작업은 hide-mission 안내",
      letters: {
        total: 3, progress: 3,
        recent: [
          { direction: "sent", peer: named("skill-merge-impl"), line: "운영자가 범위를 바꿨고 PRD amendment 1로 봉인했습니다", at_unix_ms: min(-91) },
          { direction: "received", peer: named("skill-merge-impl"), mark: "question", line: "수정 범위에서 설정 UI 문구 결정 하나가 남았습니다", at_unix_ms: min(-84) },
          { direction: "sent", peer: named("skill-merge-impl"), mark: "reply", line: "네, 세 web 파일도 갱신하세요. mission 행은 'hide mission help'만", at_unix_ms: min(-84) + 30_000 },
        ],
      },
    },
  },
  { ...CONDUCTOR, id: "answer-opened", label: "답 전체 보기 펼침", opened: true },
];

export function TurnCardCases({ seed }: { seed: AgentRow }) {
  // After the scene's own layout effect has set the store, which it would overwrite.
  useEffect(() => {
    const rest = useShellStore.getState().rest;
    useShellStore.setState({ rest: { ...rest, turn_answer: { pane_id: "answer-opened", offset: 1, phase: "ready", text: WHOLE } } as SnapshotRest });
  }, []);
  return (
    <TooltipProvider>
      <div className="flex flex-wrap items-start gap-lg bg-background p-xl text-foreground" data-gallery-scene="pane-header" data-gallery-state="turn-card">
        {CASES.map((example) => (
          <div key={example.id} className="flex flex-col gap-xs" data-turn-card-case={example.id}>
            <div className="text-micro text-muted-foreground">{example.label}</div>
            <div className="w-(--size-turn-card) max-w-full rounded-md border border-border bg-popover text-caption text-popover-foreground shadow-lg">
              <TurnCard agent={{ ...seed, id: example.id, pane_id: example.id, request: example.request }} header={example.header} onRecord={() => undefined} onReadWhole={() => undefined} answerOpened={example.opened} />
            </div>
          </div>
        ))}
      </div>
    </TooltipProvider>
  );
}
