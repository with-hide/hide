import { ArrowUpRightIcon, CircleAlertIcon, EllipsisIcon, FolderIcon, MoonIcon, SquareTerminalIcon, WaypointsIcon } from "lucide-react";
import { useMemo, useRef, type ReactNode } from "react";
import type { Actions } from "../../actions";
import type { DeskJob, DeskPullRequest } from "../../desk";
import { useInterfaceTranslation } from "../../i18n/client";
import { cn } from "../../lib/utils";
import { missionNumber, missionWorkedIn } from "../../missions";
import { PaneHeadChildren } from "../../PaneHeaderBand";
import { PaneMissionCard, usePaneMission } from "../../PaneMissionChip";
import { PanePlanChip } from "../../PanePlanChip";
import { planOfPane } from "../../plans";
import { useShellStore } from "../../store";
import { DeviceChip } from "../device-chip";
import { Elapsed } from "../elapsed";
import { AncestorPath, usePaneMenu } from "../../PaneRelations";
import { useNoticeFloor } from "../../noticeFloor";
import { wakeFailureReason, wakingLine } from "../../sleep";
import type { AgentRow, AgentSleep, PaneRow } from "../../snapshot";
import { Badge } from "../ui/badge";
import { Button } from "../ui/button";
import { PrMark } from "../pr-mark";
import { Hint } from "../ui/tooltip";
import { AskCardView } from "../../mobile/AskCard";
import { JobLead } from "./desk-job-card";
import { AgentMark } from "../../AgentMark";
import { StatusMark } from "../status-mark";

/**
 * The session panel's head (PRD desk-2-ui D-09, D-10, D-21, B6, B9, B43,
 * B47; Doop H 다 and H2 1, operator decision 2026-11-25). One line is the
 * session as the pane head draws it: its status, its agent, its ancestors
 * (each opening that session beside Desk), its work name as the sidebar row
 * names it, the children chip on a parent, then ↗ to its own tab, `파일` and `터미널` as icons
 * (the terminal hidden while it sleeps) and the pane's menu; `‹ 한눈에` above
 * it closes it (`GlanceCrumb`), so it has no `✕` of its own. A quiet
 * second line stands only when there is something to know: the machine when
 * it is another device (this machine is never named), a device that is not
 * connected, the mission it conducts as the app's mission chip and its name,
 * or the one it works a node of by its number and name, and its plan as the
 * pane's plan chip. What it made and its PRs are on the shelf below.
 *
 * A session whose pane is gone, opened from its record (`ended`, feedback 2,
 * frame K), reads `끝남` after its title and keeps nothing more: it has no pane,
 * terminal, files or menu; its second line names its machine and its pull
 * requests, which no shelf lists for it.
 */
export function SessionHead({ paneId, title, job, row, pane, machine, disconnected, sleeping, ended = null, explorerOpen, filesChord, terminalChord, deskActions, onFiles, onTerminal, onOwnTab }: {
  paneId: string;
  /** The session's work name, kept by the panel when its row and card are gone. */
  title: string;
  /** The core's card of the session, absent before it arrives and once its row is gone (PRD desk-2-ui B6, B43). */
  job: DeskJob | null;
  row: AgentRow | null;
  pane: PaneRow | null;
  /** The machine's name when the session runs on another device; null on this machine. */
  machine: string | null;
  disconnected: boolean;
  sleeping: boolean;
  /** The pull requests of a session opened from its record, its pane gone; null for a session with a pane. */
  ended?: { pulls: readonly DeskPullRequest[] } | null;
  explorerOpen: boolean;
  filesChord: string | null;
  terminalChord: string | null;
  /** The app's actions with Desk's own opens: a mission, a PR or a session opens beside Desk. */
  deskActions: Actions;
  onFiles: () => void;
  onTerminal: () => void;
  /** ↗, `원래 화면에서 보기` in its hint (feedback 1 D2): the session's own tab, its terminal with the sidebar and files, as a sidebar row's ⌘-click (Ctrl on Windows and Linux) opens it; Desk leaves the front. */
  onOwnTab: () => void;
}) {
  const { t } = useInterfaceTranslation();
  const conducts = usePaneMission(paneId);
  // The record is picked from the store and the pair made from it here: a selector making a new pair on each read
  // would never settle.
  const rest = useShellStore((s) => s.rest);
  const agentId = job?.agent_id ?? null;
  const works = useMemo(() => (row && !conducts ? missionWorkedIn(rest, row, agentId) : null), [rest, row, conducts, agentId]);
  const plan = useShellStore((s) => (sleeping ? null : planOfPane(s.rest, paneId)));
  const lead = job ? <JobLead job={job} /> : row ? <><StatusMark mark={row.mark} /><AgentMark kind={row.agent_kind} /></> : null;
  const pulls = ended?.pulls ?? [];
  const known = machine !== null || disconnected || conducts !== null || works !== null || plan !== null || pulls.length > 0;
  const tool = "text-muted-foreground";
  // A session with no pane has nothing but its record to show.
  const live = ended === null;
  return (
    <header className="flex shrink-0 flex-col gap-xxs border-b border-border px-md py-xs text-caption" data-desk-session-head={ended ? "ended" : sleeping ? "sleeping" : (job?.state ?? "pending")}>
      <div className="flex h-(--size-control-sm) min-w-0 items-center gap-xs">
        {sleeping ? <MoonIcon aria-label={t("desk.job.sleeping")} className="size-(--size-icon-sm) shrink-0 text-muted-foreground" data-desk-session-asleep="true" /> : lead}
        <nav className="flex min-w-0 flex-1 items-center gap-xxs text-subtle-foreground" aria-label={t("desk.thread.path")}>
          {pane ? <AncestorPath pane={pane} actions={deskActions} names="always" /> : null}
          <Hint label={title} reveals>
            <span className="min-w-0 truncate text-body font-semibold text-foreground" aria-current="page" data-desk-session-title="true">{title}</span>
          </Hint>
          {ended ? <span className="ml-xs shrink-0 text-muted-foreground" data-desk-session-ended="true">{t("desk.job.done")}</span> : null}
        </nav>
        {sleeping || !live ? null : <PaneHeadChildren paneId={paneId} actions={deskActions} />}
        {sleeping || !live ? null : (
          // Its words live in its hint: at the panel's first width the head keeps its room for the session's path and name.
          <Hint label={`${t("desk.room.openOwn")} · ${t("desk.room.openOwnHint")}`}>
            <Button variant="ghost" size="icon-sm" className="text-muted-foreground" aria-label={t("desk.room.openOwn")} data-desk-session-own-tab="true" onClick={onOwnTab}><ArrowUpRightIcon /></Button>
          </Hint>
        )}
        {live ? (
          <Hint label={t("desk.room.files")} shortcut={filesChord ?? undefined}>
            <Button variant="ghost" size="icon-sm" aria-label={t("desk.room.files")} aria-pressed={explorerOpen} className={cn(tool, explorerOpen && "bg-secondary text-foreground")} data-desk-session-files="true" onClick={onFiles}><FolderIcon /></Button>
          </Hint>
        ) : null}
        {sleeping || !live ? null : (
          <Hint label={t("desk.room.terminal")} shortcut={terminalChord ?? undefined}>
            <Button variant="ghost" size="icon-sm" aria-label={t("desk.room.terminal")} className={tool} data-desk-session-terminal="true" onClick={onTerminal}><SquareTerminalIcon /></Button>
          </Hint>
        )}
        {pane && !sleeping ? <SessionPaneMenu pane={pane} title={title} actions={deskActions} /> : null}
      </div>
      {known ? (
        <div className="flex min-w-0 items-center gap-xs pl-(--size-control-sm) text-caption text-muted-foreground" data-desk-session-meta="true">
          {machine ? <DeviceChip label={machine} variant="inline" className="shrink-0" /> : null}
          {disconnected ? <span className="shrink-0 text-warning" data-desk-session-disconnected="true">{t("desk.room.disconnected")}</span> : null}
          {conducts ? (
            <span className="flex min-w-0 items-center" data-desk-session-mission={conducts.id}>
              <PaneMissionCard paneId={paneId} mission={conducts} actions={deskActions} />
            </span>
          ) : null}
          {works ? (
            <Button variant="text" size="inline" className="min-w-0 gap-xxs text-caption text-subtle-foreground" aria-label={t("desk.room.workedMission", { title: works.mission.title })} data-desk-session-mission={works.mission.id} onClick={() => deskActions.openMission(works.mission.id)}>
              <WaypointsIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-muted-foreground" />
              {missionNumber(works.mission.id) ? <span className="shrink-0 font-mono text-micro text-muted-foreground">{missionNumber(works.mission.id)}</span> : null}
              <span className="min-w-0 truncate">{works.mission.title}</span>
            </Button>
          ) : null}
          {plan && !conducts ? <PanePlanChip paneId={paneId} actions={deskActions} /> : null}
          {pulls.map((pull) => (
            <Badge key={pull.number} variant="secondary" className="gap-xxs" data-desk-session-pr={pull.number}>
              <PrMark state={pull.state} compact />#{pull.number}
            </Badge>
          ))}
        </div>
      ) : null}
    </header>
  );
}

/**
 * What the agent's own screen asks, at the conversation's end, answered in
 * place (PRD desk-2-ui D-11, B14, B34; Doop B4): the phone's own ask card
 * (`AskCardView`), its choices answered
 * through the phone's path, which types the keys the agent's screen takes;
 * one the computer cannot answer goes to the terminal. `직접 쓰기` under it
 * gives the keyboard to the input box below, for an answer of the
 * operator's own words.
 */
export function SessionAskCard({ onWrite, ...card }: Parameters<typeof AskCardView>[0] & { onWrite: () => void }) {
  const { t } = useInterfaceTranslation();
  return (
    <div className="flex min-w-0 flex-col gap-xs" data-desk-ask={card.ask.kind}>
      <AskCardView {...card} />
      {card.ask.answerable && card.answer !== "sent" ? (
        <div className="flex justify-end">
          <Button variant="text" size="inline" className="text-caption text-subtle-foreground" data-desk-question-write="true" onClick={onWrite}>{t("desk.room.writeOwn")}</Button>
        </div>
      ) : null}
    </div>
  );
}

/**
 * Where the input box stood while the session sleeps (PRD desk-2-ui D-28,
 * B43): the sleeping session row's moon, its name and `에이전트 깨우기`, as
 * the sidebar's sleeping row and the sleeping pane offer it. An agent asleep
 * in its pane (`sleep`) says what its pane says (`SleepBody`, `sleep.ts`):
 * a wake under way and how old the conversation is, or a wake that failed,
 * its reason, `다시 시도` and `새 세션 시작`. A session whose sleep closed its
 * pane has no `sleep`; `state` is its phase. `onWake` is null while nothing
 * can wake it.
 */
export function SessionSleepingFoot({ title, sleep, state, onWake, onFresh }: { title: string; sleep: AgentSleep | null; state: string; onWake: (() => void) | null; onFresh: (() => void) | null }) {
  const { t } = useInterfaceTranslation();
  // A notice in the window's corner stands above the foot, as above the input box it replaces.
  const foot = useRef<HTMLDivElement>(null);
  useNoticeFloor(foot);
  const failed = sleep?.state === "failed";
  const reason = failed ? wakeFailureReason(sleep, t) : null;
  return (
    <div ref={foot} className="flex min-w-0 shrink-0 flex-col gap-xxs border-t border-border px-md py-sm text-body" data-desk-session-sleeping={sleep?.state ?? state}>
      <div className="flex min-w-0 items-center gap-xs">
        {failed ? <CircleAlertIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-destructive" /> : <MoonIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-muted-foreground" />}
        <span className="min-w-0 flex-1 truncate text-subtle-foreground">{failed ? t("panes.sleep.resumeFailed") : title}</span>
        {sleep?.state === "waking" ? <span role="status" className="shrink-0 text-caption text-muted-foreground">{wakingLine(sleep, Date.now(), t)}</span> : null}
        {onWake ? <Button variant="ghost" size="sm" data-desk-session-wake="true" onClick={onWake}>{failed ? t("common.retry") : t("panes.sleep.wake")}</Button> : null}
        {failed && onFresh ? <Button variant="ghost" size="sm" data-desk-session-wake-fresh="true" onClick={onFresh}>{t("panes.sleep.newSession")}</Button> : null}
      </div>
      {reason ? <p role="alert" className="pl-(--size-control-sm) text-caption text-muted-foreground" data-desk-session-wake-reason="true">{reason}</p> : null}
    </div>
  );
}

/**
 * Where the input box stood for a session opened from its record, its pane
 * gone (feedback 2, frame K, 다): how long ago it ended and the app's
 * `이어서 하기` (`resume`, the session record row's button), or, when it
 * cannot resume, the one line the app gives why in its place.
 */
export function SessionRecordFoot({ endedAt, resume, blocked }: { endedAt: number | null; resume: ReactNode; blocked: string | null }) {
  const { t } = useInterfaceTranslation();
  const foot = useRef<HTMLDivElement>(null);
  useNoticeFloor(foot);
  return (
    <div ref={foot} className="flex min-w-0 shrink-0 items-center gap-xs border-t border-border px-md py-sm text-caption" data-desk-session-record-foot={blocked ? "blocked" : "resume"}>
      {/* `끝남 · <time>`, as frame K 다 reads it; the time is the app's elapsed time. */}
      <span className="flex shrink-0 items-center gap-xs text-muted-foreground" data-desk-session-record-ended="true">
        {t("desk.job.done")}
        {endedAt !== null ? <><span aria-hidden="true">·</span><Elapsed since={endedAt} className="font-mono" /></> : null}
      </span>
      {blocked ? <span className="min-w-0 flex-1 text-right text-muted-foreground" data-desk-session-resume-blocked="true">{blocked}</span> : <span className="ml-auto flex shrink-0 items-center">{resume}</span>}
    </div>
  );
}

/**
 * The head's `⋯` (PRD desk-2-ui B6, Doop ②): the pane head's own menu for
 * the session's pane, with Desk's opens, so a relative opens beside Desk.
 */
function SessionPaneMenu({ pane, title, actions }: { pane: PaneRow; title: string; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const menu = usePaneMenu(pane, title, actions);
  return (
    <>
      <Hint label={t("panes.actions", { name: title })}>
        <Button
          variant="ghost"
          size="icon-sm"
          className="shrink-0 text-muted-foreground"
          aria-haspopup="menu"
          data-desk-session-menu={pane.id}
          onClick={(event) => {
            const button = event.currentTarget.getBoundingClientRect();
            menu.openAt(button.left, button.bottom);
          }}
        >
          <EllipsisIcon />
        </Button>
      </Hint>
      {menu.menu}
    </>
  );
}
