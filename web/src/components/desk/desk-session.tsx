import { FolderIcon, MoonIcon, SquareTerminalIcon, XIcon } from "lucide-react";
import { Children, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import type { Actions } from "../../actions";
import type { DeskJob, DeskOutput } from "../../desk";
import { useInterfaceTranslation } from "../../i18n/client";
import { cn } from "../../lib/utils";
import { PaneHeadChildren, PanePrChip } from "../../PaneHeaderBand";
import { PaneWorkChip } from "../../PaneMissionChip";
import { AncestorPath } from "../../PaneRelations";
import type { AgentRow, PaneRow, SleepingSession } from "../../snapshot";
import { Button } from "../ui/button";
import { Kbd } from "../ui/kbd";
import { Hint } from "../ui/tooltip";
import { AskCardView } from "../../mobile/AskCard";
import { JobLead, OutputsChip } from "./desk-job-card";

/**
 * The second line's chips in one line (PRD desk-2-ui B8): as many as fit,
 * in order, and the rest folded into `+n` at the end. Each chip is measured
 * as drawn, so a chip's own width (a long PR number, a mission title) is
 * what counts.
 */
export function FoldingChips({ children }: { children: ReactNode }) {
  const items = Children.toArray(children).filter(Boolean);
  const row = useRef<HTMLDivElement>(null);
  const [shown, setShown] = useState(items.length);
  useLayoutEffect(() => {
    const node = row.current;
    if (!node) return;
    const measure = () => {
      const room = node.getBoundingClientRect().width;
      const cells = Array.from(node.querySelectorAll<HTMLElement>(":scope > [data-desk-chip-cell]"));
      // `+n` takes a chip's room of its own when anything folds.
      const more = node.querySelector<HTMLElement>(":scope > [data-desk-chip-more]")?.getBoundingClientRect().width ?? 0;
      let used = 0;
      let fit = 0;
      for (const [index, cell] of cells.entries()) {
        const width = cell.getBoundingClientRect().width + (index > 0 ? GAP : 0);
        const reserve = index < cells.length - 1 ? more + GAP : 0;
        if (used + width + reserve > room) break;
        used += width;
        fit += 1;
      }
      setShown((now) => (now === fit ? now : fit));
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(node);
    return () => observer.disconnect();
  }, [items.length]);
  const hidden = items.length - shown;
  return (
    <div ref={row} className="flex min-w-0 flex-1 items-center gap-xs overflow-hidden" data-desk-session-chips={items.length}>
      {items.map((item, index) => (
        // A folded chip keeps its place to be measured, out of sight and out of reach.
        <span key={index} className={cn("flex shrink-0 items-center", index >= shown && "invisible absolute")} aria-hidden={index >= shown || undefined} data-desk-chip-cell="true">{item}</span>
      ))}
      <span className={cn("shrink-0 text-micro text-muted-foreground", hidden > 0 ? "" : "invisible absolute")} data-desk-chip-more={hidden}>+{hidden}</span>
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
export function SessionHead({ paneId, job, row, pane, machine, disconnected, sleeping, selectedOutput, explorerOpen, filesChord, terminalChord, deskActions, onOutput, onFiles, onTerminal, onClose }: {
  paneId: string;
  job: DeskJob;
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
  const title = row?.identity_label ?? job.title;
  return (
    <header className="flex shrink-0 flex-col gap-xxs border-b border-border px-md py-xs text-caption" data-desk-session-head={job.state}>
      <div className="flex h-(--size-control-sm) min-w-0 items-center gap-xs">
        {sleeping ? <MoonIcon aria-label={t("desk.job.sleeping")} className="size-(--size-icon-sm) shrink-0 text-muted-foreground" data-desk-session-asleep="true" /> : <JobLead job={job} />}
        <nav className="flex min-w-0 flex-1 items-center gap-xxs text-subtle-foreground" aria-label={t("desk.thread.path")}>
          {pane ? <AncestorPath pane={pane} actions={deskActions} /> : null}
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
        <Hint label={t("desk.room.closeSession")}>
          <Button variant="ghost" size="icon-sm" className="text-muted-foreground" aria-label={t("desk.room.closeSession")} data-desk-session-close="true" onClick={onClose}><XIcon /></Button>
        </Hint>
      </div>
      <div className="flex min-w-0 items-center gap-xs pl-(--size-control-sm)">
        {machine ? <span className="shrink-0 text-caption text-subtle-foreground" data-desk-session-machine="true">{machine}</span> : null}
        {disconnected ? <span className="shrink-0 text-caption text-warning" data-desk-session-disconnected="true">{t("desk.room.disconnected")}</span> : null}
        <FoldingChips>
          {sleeping ? null : <PaneWorkChip paneId={paneId} actions={deskActions} />}
          {job.outputs.length ? <OutputsChip outputs={job.outputs} selected={selectedOutput} onOpen={onOutput} /> : null}
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
 * the sidebar's sleeping row offers it.
 */
export function SessionSleepingFoot({ title, session, onWake }: { title: string; session: SleepingSession | null; onWake: (sleepId: string) => void }) {
  const { t } = useInterfaceTranslation();
  return (
    <div className="flex min-w-0 shrink-0 items-center gap-xs border-t border-border px-md py-sm text-body" data-desk-session-sleeping={session?.phase ?? "unknown"}>
      <MoonIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-muted-foreground" />
      <span className="min-w-0 flex-1 truncate text-subtle-foreground">{title}</span>
      {session?.wake_available ? <Button variant="ghost" size="sm" data-desk-session-wake="true" onClick={() => onWake(session.sleep_id)}>{t("panes.sleep.wake")}</Button> : null}
    </div>
  );
}
