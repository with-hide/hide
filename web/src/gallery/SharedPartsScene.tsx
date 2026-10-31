// The parts the sidebar, the session record and Desk share (PRD
// session-docs-and-assistant D-72), each in the states it draws, over invented
// values: the task row, a plan's steps, the letter marks, the PR mark, the
// mission chip and the @ chip. The sidebar row and the record draw the same
// parts in the projects-sidebar and session-record scenes. Nothing leaves it.
import { HouseIcon } from "lucide-react";
import { useLayoutEffect, useMemo, type ReactNode } from "react";
import { AgentMark } from "../AgentMark";
import { LetterTag, PaneLetterButton, RowLetterMark } from "../components/letter-mark";
import { MentionChip } from "../components/mention-chip";
import { MissionChip } from "../components/mission-chip";
import { PlanChecklist } from "../components/plan-checklist";
import { PrMark } from "../components/pr-mark";
import { MilestoneMark, SessionRow, SessionRowBody } from "../components/session-row";
import { StatusMark } from "../components/status-mark";
import { RowButton } from "../components/ui/row-button";
import { TooltipProvider } from "../components/ui/tooltip";
import { requireInterfaceLanguage } from "../i18n/locale";
import { useFollowInterfaceLanguage } from "../i18n/translator";
import type { Mission, PrState, SnapshotRest } from "../snapshot";
import { useShellStore } from "../store";
import { missionFixture } from "./MissionScene";
import { planFixture } from "./PlanScene";

const AT = new Date(2026, 9, 22, 15, 30).getTime();
const MISSION_STATES = ["running", "decision", "done", "cancelled"] as const;
const PR_STATES: PrState[] = ["pending", "failed", "mergeable", "draft", "merged", "closed"];

/** One mission per chip state, each conducted by its own session, so one store draws all four. */
function missions(): Mission[] {
  return MISSION_STATES.map((state) => {
    const { mission } = missionFixture(state, AT);
    return { ...mission, id: `mission-${state}`, conductor: { ...mission.conductor, pane_id: `conductor-${state}`, session: { ...mission.conductor.session!, session_id: `conductor-${state}` } } };
  });
}

const conductor = (state: (typeof MISSION_STATES)[number]) => ({ session: { device_id: "local", session_id: `conductor-${state}` } });
const time = (text: string) => <span className="font-mono text-caption text-muted-foreground">{text}</span>;
const marks = (mark: Parameters<typeof StatusMark>[0]["mark"], kind = "claude") => <><StatusMark mark={mark} /><AgentMark kind={kind} /></>;

function Part({ name, children }: { name: string; children: ReactNode }) {
  return (
    <section className="flex min-w-0 flex-col gap-xs border-b border-border pb-md" data-shared-part={name}>
      <h2 className="text-caption font-semibold text-subtle-foreground">{name}</h2>
      {children}
    </section>
  );
}

function State({ name, children }: { name: string; children: ReactNode }) {
  return (
    <div className="flex min-w-0 flex-col gap-xxs" data-shared-part-state={name}>
      <span className="text-micro text-muted-foreground">{name}</span>
      <div className="min-w-0">{children}</div>
    </div>
  );
}

export function SharedPartsScene({ params }: { params: URLSearchParams }) {
  useFollowInterfaceLanguage(requireInterfaceLanguage(params.get("lang") ?? "ko"));
  const theme = params.get("theme") === "light" ? "light" : "dark";
  const plan = useMemo(() => planFixture("running", AT), []);
  const finished = useMemo(() => planFixture("finished", AT), []);
  useLayoutEffect(() => {
    useShellStore.setState({ rest: { missions: { items: missions() } } as unknown as SnapshotRest, agents: [], connection: "live" });
    document.documentElement.classList.toggle("dark", theme === "dark");
    document.documentElement.classList.toggle("light", theme === "light");
    document.documentElement.style.setProperty("--interface-scale", params.get("scale") ?? "1");
  }, [theme, params]);
  const width = Number(params.get("width") ?? 560);
  const open = () => undefined;
  return (
    <TooltipProvider>
      <div style={{ width }} className="flex min-h-full min-w-0 flex-col gap-md bg-background p-lg text-body text-foreground" data-gallery-scene="shared-parts">
        <Part name="Task row · SessionRow">
          <div className="flex min-w-0 flex-col gap-xs">
            <State name="Working"><ol><SessionRow mark={marks("going")} title="mention 버튼 정리" chips={<PrMark state="pending" number={1903} />} right={time("12m")} line={{ text: "desk/mention-button · 테스트 돌리는 중 · 6/14" }} /></ol></State>
            <State name="Question"><ol><SessionRow mark={marks("turn")} title="갤러리 캡처 비교" right={time("3m")} line={{ text: "질문 · 다크 모드도 찍을까요? 라이트와 같은 크기로 찍어 둘지도 알려 주세요", className: "text-warning", wrap: true }} /></ol></State>
            <State name="Conductor"><ol><SessionRow mark={marks("going")} title="Desk 1차" chips={<MissionChip agent={conductor("running")} onOpen={open} />} right={time("22m")} line={{ text: "herdr-ide · Conductor · 1b 작업 중" }} /></ol></State>
            <State name="Done, pressable"><ol><SessionRow mark={marks("done", "codex")} title="release 노트 초안" titleClassName="text-subtle-foreground" chips={<PrMark state="merged" number={41} />} right={time("1h")} reveal={<span className="text-caption text-subtle-foreground">기록 보기</span>} line={{ text: "herdr-ide · 결과물 1" }} onPress={open} pressLabel="release 노트 초안" /></ol></State>
            <State name="Long title"><ol><SessionRow mark={marks("waiting")} title="세션 기록과 사이드바가 같은 부품을 쓰도록 공용 부품을 꺼내고 갤러리에 올리기" right={time("2h")} line={{ text: "desk/shared-parts · PR CI 기다리는 중" }} /></ol></State>
            <State name="Milestone"><ol><SessionRow mark={<MilestoneMark className="text-pr-merged" />} title="PR 1905 머지됨" right={time("15:02")} /></ol></State>
          </div>
        </Part>
        <Part name="Task row · record size">
          <State name="Live">
            <RowButton className="items-start gap-sm px-0 py-sm">
              <SessionRowBody size="record" mark={<StatusMark mark="going" className="shrink-0" />} head={<span className="block truncate text-caption font-medium">기록 화면 구현</span>}>
                <span className="mt-xs flex min-w-0 flex-wrap items-center gap-x-xs gap-y-xxs text-micro text-muted-foreground"><AgentMark kind="claude" />Claude Code · <span>작업 중</span><span>· PR 2</span></span>
              </SessionRowBody>
            </RowButton>
          </State>
        </Part>
        <Part name="Plan steps · PlanChecklist">
          <State name="Running"><div className="text-caption"><PlanChecklist plan={plan} /></div></State>
          <State name="Finished, folded"><div className="w-(--size-pr-popover) rounded-md border border-border bg-popover px-xs py-sm text-caption"><PlanChecklist plan={finished} popover /></div></State>
        </Part>
        <Part name="Letter · letter-mark">
          <State name="From a session"><span className="flex min-w-0 items-baseline gap-xs text-muted-foreground"><LetterTag who="기록 화면 조사" label="Letter from 기록 화면 조사" /><span className="truncate">조사 결과를 정리해 보냈어요</span></span></State>
          <State name="To a mention"><span className="flex min-w-0 items-baseline gap-xs text-muted-foreground"><LetterTag who={<MentionChip kind="session" name="1a-desk-room" />} label="Letter to 1a-desk-room" /><span className="truncate">리뷰 지적 2개 반영해서 다시 올려줘</span></span></State>
          <State name="Sidebar row"><span className="flex items-center gap-xs text-caption text-muted-foreground"><RowLetterMark />{time("4m")}</span></State>
          <State name="Pane header"><span className="flex items-center gap-xs"><PaneLetterButton paneId="w1:p1" letters={1} onDeliver={open} /><PaneLetterButton paneId="w1:p2" letters={3} onDeliver={open} /></span></State>
        </Part>
        <Part name="PR mark · PrMark">
          <State name="States"><span className="flex flex-wrap items-center gap-sm">{PR_STATES.map((state, index) => <PrMark key={state} state={state} number={1900 + index} />)}</span></State>
          <State name="Compact, stale"><span className="flex flex-wrap items-center gap-sm">{PR_STATES.map((state) => <PrMark key={state} state={state} compact />)}<PrMark state="pending" number={1903} stale /></span></State>
        </Part>
        <Part name="Mission chip · MissionChip">
          <State name="States"><span className="flex flex-wrap items-center gap-sm">{MISSION_STATES.map((state) => <MissionChip key={state} agent={conductor(state)} onOpen={open} />)}</span></State>
        </Part>
        <Part name="@ chip · MentionChip">
          <State name="Session, project"><span className="flex flex-wrap items-baseline gap-xs"><MentionChip kind="session" name="record-now" /><MentionChip kind="project" name="creator" /></span></State>
          <State name="With its marks (D-85)"><span className="flex flex-wrap items-baseline gap-xs"><MentionChip kind="session" name="record-now" lead={<><StatusMark mark="going" /><AgentMark kind="codex" /></>} /><MentionChip kind="project" name="creator" lead={<HouseIcon aria-hidden="true" className="size-(--size-icon-sm)" />} /></span></State>
          <State name="In a sentence"><p className="min-w-0"><MentionChip kind="session" name="record-now" onPress={open} label="record-now 스레드 열기" /> 갤러리 숫자 줄인 거 다크에서도 확인해줘</p></State>
          <State name="Long, narrow"><div className="w-(--size-tab-title-min) min-w-0"><MentionChip kind="project" name="herdr-ide-desk-shared-parts" /></div></State>
        </Part>
      </div>
    </TooltipProvider>
  );
}
