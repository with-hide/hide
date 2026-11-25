import { CircleAlertIcon, EllipsisIcon, FolderIcon, MoonIcon, SquareTerminalIcon, XIcon } from "lucide-react";
import { Children, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import type { Actions } from "../../actions";
import type { DeskJob, DeskOutput } from "../../desk";
import { useInterfaceTranslation } from "../../i18n/client";
import { cn } from "../../lib/utils";
import { PaneHeadChildren, PanePrChip } from "../../PaneHeaderBand";
import { PaneWorkChip } from "../../PaneMissionChip";
import { AncestorPath, usePaneMenu } from "../../PaneRelations";
import { useNoticeFloor } from "../../noticeFloor";
import { wakeFailureReason, wakingLine } from "../../sleep";
import type { AgentRow, AgentSleep, PaneRow } from "../../snapshot";
import { Button } from "../ui/button";
import { Kbd } from "../ui/kbd";
import { Popover, PopoverContent, PopoverTrigger } from "../ui/popover";
import { Hint } from "../ui/tooltip";
import { AskCardView } from "../../mobile/AskCard";
import { JobLead, OutputsChip } from "./desk-job-card";
import { AgentMark } from "../../AgentMark";
import { StatusMark } from "../status-mark";

/**
 * The second line's chips in one line (PRD desk-2-ui B8): as many as fit,
 * in order, and the rest folded into `+n` at the end. Each chip is measured
 * as drawn, so a chip's own width (a long PR number, a mission title) is
 * what counts.
 */
export function FoldingChips({ children }: { children: ReactNode }) {
  const { t } = useInterfaceTranslation();
  const items = Children.toArray(children).filter(Boolean);
  const row = useRef<HTMLDivElement>(null);
  // Where the row folds, as an index into `items`, and how many drawn chips fold there: a chip with nothing to say
  // draws nothing and is neither measured nor counted.
  const [fold, setFold] = useState({ cut: items.length, hidden: 0 });
  useLayoutEffect(() => {
    const node = row.current;
    if (!node) return undefined;
    const measure = () => {
      const room = node.getBoundingClientRect().width;
      const cells = Array.from(node.querySelectorAll<HTMLElement>(":scope > [data-desk-chip-cell]"));
      const drawn = cells.filter((cell) => cell.childElementCount > 0);
      // `+n` takes a chip's room of its own when anything folds.
      const more = node.querySelector<HTMLElement>(":scope > [data-desk-chip-more]")?.getBoundingClientRect().width ?? 0;
      let used = 0;
      let cut = cells.length;
      for (const [index, cell] of drawn.entries()) {
        const width = cell.getBoundingClientRect().width + (index > 0 ? GAP : 0);
        const reserve = index < drawn.length - 1 ? more + GAP : 0;
        if (used + width + reserve > room) {
          cut = cells.indexOf(cell);
          break;
        }
        used += width;
      }
      const hidden = drawn.filter((cell) => cells.indexOf(cell) >= cut).length;
      setFold((now) => (now.cut === cut && now.hidden === hidden ? now : { cut, hidden }));
    };
    measure();
    // The row's width moves the fold, and so does a chip whose content arrives after the row was drawn.
    const resized = new ResizeObserver(measure);
    resized.observe(node);
    for (const cell of node.querySelectorAll(":scope > [data-desk-chip-cell]")) resized.observe(cell);
    const changed = new MutationObserver(measure);
    changed.observe(node, { childList: true, subtree: true });
    return () => {
      resized.disconnect();
      changed.disconnect();
    };
  }, [items.length]);
  const hidden = fold.hidden;
  return (
    <div ref={row} className="flex min-w-0 flex-1 items-center gap-xs overflow-hidden" data-desk-session-chips="true">
      {items.map((item, index) => {
        const out = index >= fold.cut;
        // A folded chip keeps its place to be measured, out of sight and out of reach; an empty one takes no room.
        return <span key={index} className={cn("flex shrink-0 items-center empty:hidden", out && "invisible absolute")} aria-hidden={out || undefined} data-desk-chip-cell="true">{item}</span>;
      })}
      <Popover>
        <PopoverTrigger asChild>
          <Button variant="ghost" size="sm" className={cn("h-(--size-control-compact) shrink-0 px-xxs text-micro font-normal text-muted-foreground", hidden > 0 ? "" : "invisible absolute")} aria-label={t("desk.room.moreChips", { count: hidden })} tabIndex={hidden > 0 ? undefined : -1} data-desk-chip-more={hidden}>+{hidden}</Button>
        </PopoverTrigger>
        <PopoverContent align="end" className="flex w-auto flex-col items-start gap-xs p-xs" data-desk-chip-folded="true">
          {items.slice(fold.cut).map((item, index) => <span key={index} className="flex items-center empty:hidden">{item}</span>)}
        </PopoverContent>
      </Popover>
    </div>
  );
}

/** The gap between two chips, `gap-xs`. */
const GAP = 4;

/**
 * The session panel's head (PRD desk-2-ui D-09, D-10, D-21, B6, B8, B9,
 * B43, B47). The first line is the session as the pane head draws it: its
 * status, its agent, its ancestors (each opening that session beside Desk),
 * its work name as the sidebar row names it, the children chip on a parent;
 * then `파일 ⇧⌘E`, `터미널 ⌘J` (hidden while it sleeps) and `✕`. The second
 * line is its machine and the chips the pane head carries, in one line that
 * folds into `+n`: the work chip (the plan's, or the mission chip on a
 * Conductor), what it opened, and its PR; no model.
 */
export function SessionHead({ paneId, title, job, row, pane, machine, disconnected, sleeping, selectedOutput, explorerOpen, filesChord, terminalChord, deskActions, onOutput, onFiles, onTerminal, onClose }: {
  paneId: string;
  /** The session's work name, kept by the panel when its row and card are gone. */
  title: string;
  /** The core's card of the session, absent before it arrives and once its row is gone (PRD desk-2-ui B6, B43). */
  job: DeskJob | null;
  row: AgentRow | null;
  pane: PaneRow | null;
  machine: string | null;
  disconnected: boolean;
  sleeping: boolean;
  selectedOutput: string | null;
  explorerOpen: boolean;
  filesChord: string | null;
  terminalChord: string | null;
  /** The app's actions with Desk's own opens: a mission, a PR or a session opens beside Desk. */
  deskActions: Actions;
  onOutput: (output: DeskOutput) => void;
  onFiles: () => void;
  onTerminal: () => void;
  onClose: () => void;
}) {
  const { t } = useInterfaceTranslation();
  const lead = job ? <JobLead job={job} /> : row ? <><StatusMark mark={row.mark} /><AgentMark kind={row.agent_kind} /></> : null;
  return (
    <header className="flex shrink-0 flex-col gap-xxs border-b border-border px-md py-xs text-caption" data-desk-session-head={sleeping ? "sleeping" : (job?.state ?? "pending")}>
      <div className="flex h-(--size-control-sm) min-w-0 items-center gap-xs">
        {sleeping ? <MoonIcon aria-label={t("desk.job.sleeping")} className="size-(--size-icon-sm) shrink-0 text-muted-foreground" data-desk-session-asleep="true" /> : lead}
        <nav className="flex min-w-0 flex-1 items-center gap-xxs text-subtle-foreground" aria-label={t("desk.thread.path")}>
          {pane ? <AncestorPath pane={pane} actions={deskActions} names="always" /> : null}
          <Hint label={title} reveals>
            <span className="min-w-0 truncate text-body font-semibold text-foreground" aria-current="page" data-desk-session-title="true">{title}</span>
          </Hint>
        </nav>
        {sleeping ? null : <PaneHeadChildren paneId={paneId} actions={deskActions} />}
        <Hint label={t("desk.room.files")} shortcut={filesChord ?? undefined}>
          <Button variant="ghost" size="sm" aria-pressed={explorerOpen} className={cn("h-(--size-control-compact) gap-xxs px-xs text-caption font-normal text-subtle-foreground", explorerOpen && "bg-secondary text-foreground")} data-desk-session-files="true" onClick={onFiles}>
            <FolderIcon />
            {t("desk.room.files")}
            {filesChord ? <Kbd aria-hidden="true">{filesChord}</Kbd> : null}
          </Button>
        </Hint>
        {sleeping ? null : (
          <Hint label={t("desk.room.terminal")} shortcut={terminalChord ?? undefined}>
            <Button variant="ghost" size="sm" className="h-(--size-control-compact) gap-xxs px-xs text-caption font-normal text-subtle-foreground" data-desk-session-terminal="true" onClick={onTerminal}>
              <SquareTerminalIcon />
              {t("desk.room.terminal")}
              {terminalChord ? <Kbd aria-hidden="true">{terminalChord}</Kbd> : null}
            </Button>
          </Hint>
        )}
        {pane && !sleeping ? <SessionPaneMenu pane={pane} title={title} actions={deskActions} /> : null}
        <Hint label={t("desk.room.closeSession")}>
          <Button variant="ghost" size="icon-sm" className="text-muted-foreground" aria-label={t("desk.room.closeSession")} data-desk-session-close="true" onClick={onClose}><XIcon /></Button>
        </Hint>
      </div>
      <div className="flex min-w-0 items-center gap-xs pl-(--size-control-sm)">
        {machine ? <span className="shrink-0 text-caption text-subtle-foreground" data-desk-session-machine="true">{machine}</span> : null}
        {disconnected ? <span className="shrink-0 text-caption text-warning" data-desk-session-disconnected="true">{t("desk.room.disconnected")}</span> : null}
        <FoldingChips>
          {sleeping ? null : <PaneWorkChip paneId={paneId} actions={deskActions} />}
          {job?.outputs.length ? <OutputsChip outputs={job.outputs} selected={selectedOutput} onOpen={onOutput} /> : null}
          {sleeping ? null : <PanePrChip paneId={paneId} actions={deskActions} />}
        </FoldingChips>
      </div>
    </header>
  );
}

/** What a waiting session asks, as its sidebar row or its card names it (B14, B34). */
export type SessionQuestion = { kind: "question" | "plan_approval" | "decision" | "approval"; text: string };

/** The question a session waits on (B14): the agent's own ask from its sidebar row, else the card's line. */
export function sessionQuestion(job: DeskJob, row: AgentRow | null): SessionQuestion | null {
  if (job.state !== "asking") return null;
  const turn = row?.user_turn;
  if (turn?.content) return { kind: turn.kind, text: turn.content.text };
  const kind = job.ask === "approval" ? "approval" : job.ask === "decision" ? "decision" : job.ask === "plan_review" ? "plan_approval" : "question";
  return job.question ? { kind, text: job.question } : null;
}

const QUESTION_WORD = { question: "desk.ask.question", plan_approval: "desk.ask.plan_review", decision: "desk.ask.decision", approval: "desk.ask.approval" } as const;

/**
 * The question card while what the agent asks on its screen has not been
 * read (PRD desk-2-ui B14): the kind of ask and what it asks, from its
 * sidebar row or its card, and `직접 쓰기`, which gives the keyboard to the
 * input box below.
 */
export function SessionQuestionCard({ question, onWrite }: { question: SessionQuestion; onWrite: () => void }) {
  const { t } = useInterfaceTranslation();
  return (
    <section className="flex min-w-0 flex-col gap-sm rounded-lg border border-warning bg-warning/10 p-md" aria-label={t(QUESTION_WORD[question.kind])} data-desk-question={question.kind}>
      <p className="flex min-w-0 items-baseline gap-xs text-body text-foreground">
        <span className="shrink-0 text-caption text-warning">{t(QUESTION_WORD[question.kind])}</span>
        <span className="min-w-0 whitespace-pre-wrap break-keep font-semibold [overflow-wrap:anywhere]">{question.text}</span>
      </p>
      <div className="flex justify-end">
        <Button variant="text" size="inline" className="text-caption text-subtle-foreground" data-desk-question-write="true" onClick={onWrite}>{t("desk.room.writeOwn")}</Button>
      </div>
    </section>
  );
}

/**
 * The question card at the conversation's end (PRD desk-2-ui D-11, B14,
 * B34): the phone's own ask card (`AskCardView`), its choices answered
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
