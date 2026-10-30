import { ChevronDownIcon, GitForkIcon, ListTreeIcon } from "lucide-react";
import { useMemo, type ReactNode } from "react";
import type { TFunction } from "i18next";
import type { Actions } from "./actions";
import { agentProductName } from "./agentAdapters";
import { askVerb, askWhat, VerbText } from "./components/agent-tree";
import { AgentPrMark } from "./components/pr-mark";
import { rowPulls } from "./prMark";
import { AgentTreePopover } from "./components/agent-tree-popover";
import { Elapsed } from "./components/elapsed";
import { Hint } from "./components/ui/tooltip";
import { useInterfaceTranslation } from "./i18n/client";
import { cn } from "./lib/utils";
import { localDeviceId, type PaneBand, type PaneHeader, type PaneRow, type SnapshotRest, type AgentRow, type Device, type RemoteStatus } from "./snapshot";
import { remoteStateText } from "./coreError";
import { machineName } from "./screenMachine";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { relationState } from "./lineage";
import { deviceConnected } from "./devices";
import { remoteTargetOfPane } from "./remote";
import { useRelationProgress } from "./PaneRelations";
import { markTone, markWords, recordWords, waitWord } from "./agentRow";
import { allAgents, agentsByPane } from "./paneAgents";
import { paneChildren, type PaneChildren } from "./paneChildren";
import { sessionOfAgentOn } from "./sessionWork";
import { statusText } from "./agentStatus";
import { StatusMark } from "./components/status-mark";
import { PaneTurnCard, requestWords, type TurnCardTrigger } from "./PaneTurnCard";
import { planOfPane, planWorkLine } from "./plans";
import { missionOfPane, missionWorkLine } from "./missions";
import type { PaneTurnRowMode } from "./generated/hided-ws";

/** The View menu's turn row mode; absent reads as one line. */
export function usePaneTurnRowMode(): PaneTurnRowMode {
  return useShellStore((state) => state.rest?.ui_state?.pane_turn_row_mode) ?? "one_line";
}

/**
 * Marks the pane Fork agent made (PRD fork-operator-root B8, B9): a quiet icon after
 * the title whose name says where it was forked from. While the source still runs
 * that conversation the mark is a button that opens it; once the source closed or
 * moved on it names no pane and opens nothing. Only the core's fork fact draws it.
 */
export function PaneForkMark({ pane, actions }: { pane: PaneRow; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const source = pane.fork?.forked_from;
  if (!source) return null;
  if (source.pane_id && source.label) return <ForkSourceButton paneId={pane.id} source={{ paneId: source.pane_id, name: source.label }} actions={actions} />;
  const label = t("panes.fork.forked");
  return (
    <Hint label={label}>
      <span className="flex shrink-0 items-center text-subtle-foreground" role="img" aria-label={label} data-pane-fork="">
        <GitForkIcon className="size-(--size-icon-sm)" aria-hidden="true" />
      </span>
    </Hint>
  );
}

/** The mark of a fork whose source still runs its conversation: it opens the source. */
function ForkSourceButton({ paneId, source, actions }: { paneId: string; source: { paneId: string; name: string }; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const progress = useRelationProgress(paneId, source.paneId);
  const label = t("panes.fork.forkedFrom", { name: source.name });
  const pending = progress?.phase === "pending";
  const failed = progress?.phase === "failed";
  return (
    <Hint label={failed ? progress.message : label}>
      <button
        type="button"
        className={cn("flex shrink-0 items-center rounded-xs text-subtle-foreground outline-none hover:text-foreground focus-visible:ring-1 focus-visible:ring-ring disabled:opacity-50", failed && "text-destructive")}
        aria-label={label}
        aria-busy={pending}
        data-pane-fork={source.paneId}
        disabled={pending || (failed && !progress.retryable)}
        onClick={() => actions.followRelation(paneId, source.paneId, source.name)}
      >
        <GitForkIcon className="size-(--size-icon-sm)" aria-hidden="true" />
      </button>
    </Hint>
  );
}

/**
 * What a pane's turn shows now, read once for both places that draw it: the
 * head, after the title, in one line (`PaneHeadTurn`), and the second row
 * over the terminal in two lines (`PaneHeaderBand`). The answer is an ask,
 * the pane agent's own progress, a finished turn's result or a wait on
 * working children; off draws none of them, the pane's own ask riding the
 * head as `PaneAskMark`.
 */
function usePaneTurn(paneId: string, header: PaneHeader | undefined) {
  const { t } = useInterfaceTranslation();
  const relation = useUiStore((state) => state.relation);
  const outcome = useShellStore((state) => state.rest?.status?.pane_focus_request);
  const agents = usePaneAgents();
  const { children, devices } = usePaneChildren(paneId, agents);
  const childConnection = children?.unavailableDevice ? notConnectedText(devices?.status?.remote, devices?.navigator?.devices, children.unavailableDevice, t) : null;
  const mode = usePaneTurnRowMode();
  const off = mode === "off";
  // A Conductor's pane says its waiting decision, else the nodes a worker is
  // at, and nothing when no node runs: the mission line owns the place, so
  // the Conductor's own plan or progress does not stand in (PRD
  // mission-plan-visibility B9, D-21).
  const mission = useShellStore((state) => missionOfPane(state.rest, allAgents(state.agents, state.rest?.status?.remote), paneId));
  const missionLine = mission ? missionWorkLine(mission, t) : null;
  // Off draws no turn; a raised descendant's ask stays on the children
  // button's warning border. A waiting decision is the operator's to answer
  // and the Conductor's own work left is not, so the decision takes its place.
  const ownBand = off ? null : header?.band ?? null;
  const band = ownBand?.kind === "stopped" && !ownBand.raised && missionLine?.decision ? null : ownBand;
  // A running plan's current step and its line take the progress place (PRD hide-plan B7).
  const planLine = useShellStore((state) => {
    const plan = planOfPane(state.rest, paneId);
    return plan ? planWorkLine(plan) : null;
  });
  const ownProgress = band || off ? null : mission ? missionLine?.text ?? null : planLine ?? header?.progress ?? null;
  const request = off ? null : header?.request ?? null;
  const finished = !band && !off && (header?.finished ?? false);
  // A Conductor's mission line already names the work its children do.
  const waitingOnChildren = band || off || missionLine ? null : header?.waiting_on_children ?? null;
  const unreachable = useUnreachable(band?.action?.pane_id ?? null);
  const tracked = band?.action && relation?.sourcePaneId === paneId && relation.targetPaneId === band.action.pane_id ? relation : null;
  const progress = relationState(tracked, outcome, t);
  const navigation = progress?.phase === "failed" ? progress.message : progress?.phase === "pending" ? t("panes.relation.opening", { name: tracked!.label }) : null;
  // The one thing the turn says now, the request aside.
  const kind = band?.kind ?? (finished ? "finished" : ownProgress ? "progress" : waitingOnChildren ? "waiting" : null);
  return { mode, agents, children, childConnection, missionLine, band, ownProgress, request, finished, waitingOnChildren, unreachable, progress, navigation, kind };
}

type PaneTurn = ReturnType<typeof usePaneTurn>;

/**
 * The pane's turn on its head in one line, the default (docs/UI_BEHAVIOR.md,
 * Pane turn row mode): after the title and its marks, `·` and the one thing
 * the turn says now, an ask in the warning colour, a finished turn's result,
 * a wait on children or the agent's progress, and never the request, which
 * the turn card this text opens keeps. It fills the head's unused space, so
 * its blank still moves the pane; progress gives way before the title and an
 * ask shares the room with it, so the ask's verb stays in sight. On a pane too
 * narrow for words the status mark alone says it. The children button stands
 * before the head's tools in one line and off, so neither adds a row over the
 * terminal.
 */
export function PaneHeadTurn({ paneId, header, actions }: { paneId: string; header: PaneHeader | undefined; actions: Actions }) {
  const turn = usePaneTurn(paneId, header);
  const { mode, band, kind, childConnection } = turn;
  const tree = turn.children && mode !== "two_lines" ? <PaneTreeButton paneId={paneId} children={turn.children} agents={turn.agents} actions={actions} connection={childConnection} /> : null;
  const shown = mode === "one_line" && (kind !== null || childConnection !== null);
  const ask = band !== null;
  return <>
    {/* Progress takes only the room the title leaves; an ask shares the
        shrinking with the title, so its verb stays in sight. */}
    <span data-pane-move-blank="true" className={cn("flex h-full min-w-0 items-center gap-xs overflow-hidden", ask ? "flex-auto" : "flex-1")}>
      {shown ? (
        <span className="hidden min-w-0 items-center gap-xs pane-turn-room:flex" data-pane-head-turn={kind ?? "children"} data-pane-turn-row-mode={mode}>
          <span aria-hidden="true" className="shrink-0 text-subtle-foreground">·</span>
          {kind ? <PaneTurnCard paneId={paneId} actions={actions} trigger={(state) => <HeadTurnButton paneId={paneId} turn={turn} state={state} />} /> : null}
          {band?.action ? <BandOpen paneId={paneId} band={band} turn={turn} actions={actions} /> : null}
          {band && !tree ? <MoreAsks paneId={paneId} band={band} agents={turn.agents} actions={actions} /> : null}
          {childConnection ? <span className={cn("min-w-0 truncate text-muted-foreground", kind ? "max-w-1/4 shrink" : "flex-1")} title={childConnection} data-pane-child-connection={paneId}>{childConnection}</span> : null}
        </span>
      ) : null}
    </span>
    {tree}
  </>;
}

/**
 * The head's turn words as one button: a press or focus opens the turn card
 * under it, the pointer passing over does not, and it never takes the
 * keyboard from the terminal. Its name carries the request the head leaves out.
 */
function HeadTurnButton({ paneId, turn, state }: { paneId: string; turn: PaneTurn; state: TurnCardTrigger }) {
  const { t } = useInterfaceTranslation();
  const { band, request } = turn;
  const ask = useAskWords(band);
  const read = band !== null && !band.raised && band.read;
  const words = band ? [ask.verb, ask.originTitle, turn.navigation ?? ask.what].filter(Boolean).join(" ") : turn.finished ? [t("paneTurn.finished"), turn.ownProgress].filter(Boolean).join(" ") : turn.ownProgress ?? waitingWords(t, turn.waitingOnChildren);
  const label = [words, request ? `${t("paneTurn.request")}: ${requestWords(t, request)}` : null].filter(Boolean).join(", ");
  return (
    <button
      type="button"
      aria-label={label}
      aria-expanded={state.open}
      data-pane-turn-trigger={paneId}
      data-state={state.open ? "open" : "closed"}
      className={cn("flex min-w-0 items-center gap-xs rounded-xs px-xxs -mx-xxs text-muted-foreground underline decoration-border decoration-dotted underline-offset-4 outline-none hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring data-[state=open]:bg-secondary", read && "opacity-(--opacity-read-status)")}
      data-pane-band-read={read ? "true" : undefined}
      onMouseDown={(event) => event.preventDefault()}
      onClick={state.onToggle}
      onFocus={state.onFocus}
      onBlur={state.onBlur}
      onKeyDown={state.onKeyDown}
    >
      {band ? <>
        <AskVerb band={band} />
        {ask.raised && ask.originTitle ? <span className="flex max-w-(--size-pane-child-chip-max) shrink-0 items-center gap-xxs overflow-hidden font-semibold text-foreground" data-pane-band-who={ask.raised.raised_pane_id}><span className="min-w-0 truncate">{ask.originTitle}</span><span className="shrink-0 text-muted-foreground">({t(ask.originDepth > 2 ? "panes.children.descendant" : "panes.children.direct")})</span></span> : null}
        {turn.navigation ?? ask.what ? <span className={cn("min-w-0 truncate", turn.progress?.phase === "failed" ? "text-destructive" : unfinished(band) ? "text-muted-foreground" : "text-warning")} role={turn.progress?.phase === "failed" ? "alert" : turn.navigation ? "status" : undefined} data-pane-band-navigation={turn.navigation ? turn.progress?.phase === "failed" ? "failed" : "pending" : undefined}>{turn.navigation ?? ask.what}</span> : null}
      </> : <>
        {turn.finished ? <span className="shrink-0 font-medium text-success" data-pane-finished={paneId}>{t("paneTurn.finished")}</span> : null}
        {turn.waitingOnChildren ? <span className="min-w-0 truncate" data-pane-children-wait={paneId}>{waitingWords(t, turn.waitingOnChildren)}</span> : null}
        {turn.ownProgress ? <span className={cn("min-w-0 truncate", turn.missionLine?.decision && "text-mission-hold")} data-pane-own-progress={paneId} data-pane-mission-line={turn.missionLine ? (turn.missionLine.decision ? "decision" : "running") : undefined}>{turn.ownProgress}</span> : null}
      </>}
    </button>
  );
}

/** The words a wait on working children says, the ones its sidebar row says. */
function waitingWords(t: TFunction<"translation">, count: number | null): string {
  return count ? [waitWord(t, "children"), t("agents.line.children", { count })].join(" · ") : "";
}

/**
 * The pane's second row over its terminal in two lines (docs/UI_BEHAVIOR.md,
 * Agent panes and the Agents explorer): the child-tree entry and the request
 * on one line, the arrow and the answer on the line under it, each cut. A
 * working turn with nothing beyond its request shows the request alone: the
 * status mark and the working line already say it works. One line and off
 * draw the turn on the head instead, so this row is only the working line
 * then. Neither the terminal viewport nor its PTY grid moves for it.
 */
export function PaneHeaderBand({ paneId, header, actions }: { paneId: string; header: PaneHeader | undefined; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const turn = usePaneTurn(paneId, header);
  const { mode, band, children, childConnection, ownProgress, request, finished, waitingOnChildren, unreachable, progress, navigation } = turn;
  const workingLine = header?.working ? <div className="pointer-events-none absolute inset-x-0 top-0 z-10 h-[calc(2*var(--size-hairline))] bg-agent-working" data-pane-working-line={paneId} /> : null;
  if (mode !== "two_lines" || (!band && !children && !ownProgress && !request && !finished && !waitingOnChildren)) return workingLine;
  const tree = children ? <PaneTreeButton paneId={paneId} children={children} agents={turn.agents} actions={actions} connection={childConnection} /> : null;
  const answer = <>
    {band ? <AskBand paneId={paneId} band={band} navigation={navigation} unreachable={unreachable} failed={progress?.phase === "failed"} actions={actions} hasTree={children !== null} /> : <>
      {finished ? <span className="shrink-0 font-medium text-success" data-pane-finished={paneId}>{t("paneTurn.finished")}</span> : null}
      {waitingOnChildren ? <span className="min-w-0 flex-1 truncate" data-pane-children-wait={paneId}>{waitingWords(t, waitingOnChildren)}</span> : null}
      {ownProgress ? <Hint label={ownProgress}><span className={cn("min-w-0 flex-1 truncate", turn.missionLine?.decision && "text-mission-hold")} data-pane-own-progress={paneId} data-pane-mission-line={turn.missionLine ? (turn.missionLine.decision ? "decision" : "running") : undefined}>{ownProgress}</span></Hint> : null}
      {childConnection ? <span className={cn("min-w-0 truncate", ownProgress ? "max-w-1/4 shrink" : "flex-1")} title={childConnection} data-pane-child-connection={paneId}>{childConnection}</span> : null}
    </>}
    {band?.action ? <BandOpen paneId={paneId} band={band} turn={turn} actions={actions} /> : null}
  </>;
  const kind = turn.kind ?? (request ? "request" : "children");
  // A working turn with no word beyond its request, or a parent with no turn
  // to show, draws no answer at all, so two lines never add an empty one.
  const answered = (kind !== "request" && kind !== "children") || childConnection !== null;
  // An ask is the one row the operator acts on, so it keeps the raised
  // surface; work left and a quiet turn sit in the head's surface.
  const tone = band && !unfinished(band) ? "bg-secondary border-l-2 border-warning text-foreground" : "bg-card border-t border-border text-muted-foreground";
  return <div className="absolute inset-x-0 top-0 z-10 bg-background">
    {/* One block whether or not it leads with a request, so the tree's
        popover stays open when the request comes or goes. */}
    <div className={cn("flex min-w-0 flex-col items-stretch gap-x-xs px-sm text-caption", tone)} data-pane-header-band={kind} data-pane-turn={request ? paneId : undefined} data-pane-turn-row-mode={mode}>
      {tree || request ? (
        <div key="request" className="flex h-(--size-pane-header) min-w-0 max-w-full shrink items-center gap-xs" data-pane-turn-part="request">
          {tree}
          {request ? <PaneTurnCard paneId={paneId} actions={actions} trigger={(state) => <TurnRequestButton paneId={paneId} request={request} state={state} />} /> : null}
        </div>
      ) : null}
      {answered ? (
        <div key="answer" className="flex h-(--size-pane-header) min-w-0 max-w-full items-center gap-xs" data-pane-turn-part="answer">
          {request ? <span aria-hidden="true" className="shrink-0 text-subtle-foreground">→</span> : null}
          {answer}
        </div>
      ) : null}
    </div>
    {workingLine}
  </div>;
}

/** Open on a raised descendant's ask: it goes to that agent, disabled while its device is away or a move is pending. */
function BandOpen({ paneId, band, turn, actions }: { paneId: string; band: PaneBand; turn: PaneTurn; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const { progress, unreachable } = turn;
  const action = band.action!;
  // Off its row the reason has no words of its own beside Open, so Open's name and hint carry it.
  const reason = turn.mode === "one_line" && unreachable && !turn.navigation ? unreachable : null;
  const button = <button type="button" data-pane-band-open={paneId} aria-label={reason ? `${t("common.open")} · ${reason}` : undefined} disabled={unreachable !== null || progress?.phase === "pending" || (progress?.phase === "failed" && !progress.retryable)} aria-busy={progress?.phase === "pending"} className="shrink-0 rounded-xs border border-border bg-background px-xs py-xxs text-micro text-foreground outline-none hover:brightness-95 focus-visible:ring-1 focus-visible:ring-ring disabled:opacity-50" onClick={() => actions.followRelation(paneId, action.pane_id, action.label)}>{progress?.phase === "failed" && progress.retryable ? t("common.retry") : t("common.open")}</button>;
  return reason ? <Hint label={reason}><span className="flex shrink-0" data-pane-band-unreachable="true">{button}</span></Hint> : button;
}

/**
 * The request half of the turn: the request's one line in quiet text, with
 * no word before it on screen (its name still says `요청`), a button the
 * keyboard reaches. A press or focus on it opens the turn card, the
 * pointer passing over does not; it never takes the keyboard from the terminal.
 */
function TurnRequestButton({ paneId, request, state }: { paneId: string; request: NonNullable<PaneHeader["request"]>; state: TurnCardTrigger }) {
  const { t } = useInterfaceTranslation();
  const words = requestWords(t, request);
  return (
    <button
      type="button"
      aria-label={`${t("paneTurn.request")}: ${words}`}
      aria-expanded={state.open}
      data-pane-turn-request={paneId}
      data-pinned={state.pinned || undefined}
      data-state={state.open ? "open" : "closed"}
      className="flex min-w-0 items-center gap-xxs rounded-xs px-xxs -mx-xxs text-subtle-foreground outline-none hover:bg-accent hover:text-foreground focus-visible:ring-1 focus-visible:ring-ring data-[state=open]:text-foreground"
      onMouseDown={(event) => event.preventDefault()}
      onClick={state.onToggle}
      onFocus={state.onFocus}
      onBlur={state.onBlur}
      onKeyDown={state.onKeyDown}
    >
      <span className="min-w-0 truncate" data-pane-turn-request-text="true">{words}</span>
    </button>
  );
}

/**
 * The pane's own ask on its head while the View menu turns the turn row off:
 * its verb and wait, a trigger of the turn card as the request is, so a
 * question is never hidden by the mode. A raised descendant's ask stays on the
 * children button's warning border, whose tree opens on it.
 */
export function PaneAskMark({ paneId, header, actions }: { paneId: string; header: PaneHeader | undefined; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const mode = usePaneTurnRowMode();
  const band = header?.band ?? null;
  if (mode !== "off" || !band || band.raised || band.kind === "raised") return null;
  const what = band.kind === "approval" || band.kind === "answer" ? askWhat(t, band.kind, band.reason, band.subject) : band.reason;
  const verb = bandVerbWords(t, band.kind);
  return <PaneTurnCard paneId={paneId} actions={actions} trigger={(state) => (
    <button
      type="button"
      aria-label={what ? `${verb}: ${what}` : verb}
      aria-expanded={state.open}
      data-pane-ask-mark={paneId}
      data-state={state.open ? "open" : "closed"}
      className={cn("flex h-(--size-sidebar-line-detail) shrink-0 items-center gap-xxs rounded-xs border px-xs text-micro outline-none hover:bg-popover focus-visible:ring-1 focus-visible:ring-ring data-[state=open]:bg-secondary", unfinished(band) ? "border-border" : "border-warning/60", band.read && "opacity-(--opacity-read-status)")}
      onMouseDown={(event) => event.preventDefault()}
      onClick={state.onToggle}
      onFocus={state.onFocus}
      onBlur={state.onBlur}
      onKeyDown={state.onKeyDown}
    >
      <span className={cn("font-semibold", unfinished(band) ? "text-foreground" : "text-warning")} data-verb={band.kind}>{verb}</span>
      <Elapsed since={band.since_unix_ms} className="font-mono text-muted-foreground" />
    </button>
  )} />;
}

/** The pane's own ask's verb, as the row's ask starts with it; a stop is work left, `일 남음`, as the row's line says. */
function bandVerbWords(t: TFunction<"translation">, kind: PaneBand["kind"]): string {
  if (kind === "approval" || kind === "answer") return t(`agentSessions.verb.${kind}`);
  return kind === "blocked" ? t("agentSessions.tag.blocked") : statusText(t, "unfinished");
}

/** The pane's own work left: not the operator's to answer, so it is drawn without the warning colour. */
function unfinished(band: PaneBand): boolean {
  return band.kind === "stopped" && !band.raised;
}

/**
 * Whose ask it is and what it asks, as the row and the head both word it:
 * the pane's own approval, answer, block or stop by its verb, a raised
 * descendant by its verb, then that agent's name and relationship, so it
 * never reads as its parent's own ask.
 */
function useAskWords(band: PaneBand | null) {
  const { t } = useInterfaceTranslation();
  const agents = usePaneAgents();
  const raised = band?.raised ?? null;
  const origin = raised?.verb === "draft" ? agentsByPane(agents).get(raised.raised_pane_id) : null;
  const originTitle = raised?.origin_title ?? (raised?.verb === "draft" ? origin?.identity_label : raised?.title) ?? null;
  const originDepth = raised ? raised.path.length + (raised.verb === "draft" ? 1 : 0) : 0;
  const what = !band ? null : raised ? askWhat(t, raised.verb, raised.what, raised.subject) : band.kind === "approval" || band.kind === "answer" ? askWhat(t, band.kind, band.reason, band.subject) : band.reason;
  const verb = !band ? null : raised ? askVerb(t, raised.verb) : bandVerbWords(t, band.kind);
  const minutes = band?.since_unix_ms == null ? null : Math.max(0, Math.floor((Date.now() - band.since_unix_ms) / 60_000));
  // The path from the root already ends with the agent named; the wait is its verb's (B5).
  const who = raised ? [
    raised.verb === "draft" ? origin && agentProductName(origin.agent_kind) : agentProductName(raised.agent_kind),
    (raised.verb === "draft" && originTitle ? [...raised.path, originTitle] : raised.path).join(" › "),
    [raised.verb === "draft" ? origin?.checkout_label : raised.checkout, minutes === null ? null : t("agentSessions.waited", { minutes, verb: t(`agentSessions.verb.${raised.verb}`) })].filter(Boolean).join(" · "),
  ].filter(Boolean).join("\n") : null;
  return { raised, originTitle, originDepth, what, verb, minutes, who };
}

/** An ask's leading word: a raised descendant's verb or the pane's own approval and question in the warning colour, work left in the foreground's. */
function AskVerb({ band }: { band: PaneBand }) {
  const { t } = useInterfaceTranslation();
  if (band.raised) return <VerbText verb={band.raised.verb} />;
  if (band.kind === "approval" || band.kind === "answer") return <VerbText verb={band.kind} />;
  return <span className={cn("shrink-0 font-semibold", unfinished(band) ? "text-foreground" : "text-warning")} data-verb={band.kind}>{bandVerbWords(t, band.kind)}</span>;
}

/** The older more-asks trigger, kept only where no children button is the tree's way in. */
function MoreAsks({ paneId, band, agents, actions }: { paneId: string; band: PaneBand; agents: AgentRow[]; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const self = agentsByPane(agents).get(paneId);
  if (band.more <= 0 || !self) return null;
  return <AgentTreePopover
    parent={self}
    agents={agents}
    raised
    onOpenChild={(id, label) => actions.followRelation(paneId, id, label)}
    onTask={() => actions.revealTask(paneId)}
    returnFocus={() => document.querySelector<HTMLButtonElement>(`[data-pane-band-more="${paneId}"]`)?.focus()}
    triggerLabel={t("agentSessions.moreAsks", { count: band.more })}
    trigger={<button type="button" data-pane-band-more={paneId} className="shrink-0 rounded-xs px-xs py-xxs text-micro text-muted-foreground underline-offset-2 outline-none hover:text-foreground hover:underline focus-visible:ring-1 focus-visible:ring-ring">{t("agentSessions.moreAsks", { count: band.more })}</button>}
  />;
}

/**
 * An ask on the second row, led by whose it is (`useAskWords`). The pane's
 * own ask dims once the operator has looked and stays until it is answered.
 */
function AskBand({ paneId, band, navigation, unreachable, failed, actions, hasTree }: { paneId: string; band: PaneBand; navigation: string | null; unreachable: string | null; failed: boolean; actions: Actions; hasTree: boolean }) {
  const { t } = useInterfaceTranslation();
  const agents = usePaneAgents();
  const { raised, originTitle, originDepth, what, minutes, who } = useAskWords(band);
  const read = !raised && band.read;
  return <>
    <span className={cn("flex min-w-0 flex-[3] items-center gap-xs", read && "opacity-(--opacity-read-status)")} data-pane-band-read={read ? "true" : undefined}>
      <AskVerb band={band} />
      {raised && originTitle ? <Hint label={who!}><span className="flex min-w-0 max-w-1/3 shrink items-center gap-xxs font-semibold" data-pane-band-who={raised.raised_pane_id}><span className="min-w-0 truncate">{originTitle}</span><span className="shrink-0 text-muted-foreground">({t(originDepth > 2 ? "panes.children.descendant" : "panes.children.direct")})</span></span></Hint> : null}
      <Hint label={navigation ?? what ?? ""}><span className={cn("min-w-0 flex-1 truncate", failed && "text-destructive")} role={failed ? "alert" : navigation ? "status" : undefined} data-pane-band-navigation={navigation ? failed ? "failed" : "pending" : undefined}>{navigation ?? what}</span></Hint>
    </span>
    {unreachable && !navigation ? <span className="min-w-0 max-w-1/4 shrink truncate text-muted-foreground" title={unreachable} data-pane-band-unreachable="true">{unreachable}</span> : null}
    {raised?.unreceived_by && minutes !== null ? <span className="min-w-0 max-w-1/4 flex-1 truncate text-muted-foreground" data-pane-band-unreceived="true">{t("agentSessions.unreceived", { name: raised.unreceived_by, minutes })}</span> : null}
    {raised?.unreceived_by ? null : <Elapsed since={band.since_unix_ms} className={cn("shrink-0 font-mono text-micro text-muted-foreground", read && "opacity-(--opacity-read-status)")} />}
    {hasTree ? null : <MoreAsks paneId={paneId} band={band} agents={agents} actions={actions} />}
  </>;
}

/** Why `device` is not connected, worded from its state; a device still connecting or holding its last report is plainly not connected. */
function notConnectedText(remote: readonly RemoteStatus[] | undefined, devices: readonly Device[] | undefined, device: string, t: TFunction<"translation">): string {
  const state = remote?.find((row) => row.target_id === device)?.state;
  return !state || state === "not_connected" || state === "stale" ? t("devices.rail.notConnected") : remoteStateText(state, machineName(devices, device, t), t);
}

/** Why a pane on a device that is not connected cannot be opened now, or null when it can (B8). */
function useUnreachable(paneId: string | null): string | null {
  const { t } = useInterfaceTranslation();
  return useShellStore((state) => {
    if (paneId === null) return null;
    const device = remoteTargetOfPane(state.rest, paneId);
    if (device === null || deviceConnected(state.rest, device)) return null;
    return notConnectedText(state.rest?.status?.remote, state.rest?.navigator?.devices, device, t);
  });
}

/** Every agent row this page knows, including a device's retained topology; it changes only when a row does. */
export function usePaneAgents(): AgentRow[] {
  return useShellStore((state) => allAgents(state.agents, state.rest?.status?.remote));
}

/**
 * This pane's child rows. Only a pane that is a parent reads the device
 * connection rows its totals depend on; every other pane stays out of the rest
 * of the snapshot, so it neither re-renders nor recomputes when it moves.
 */
function usePaneChildren(paneId: string, agents: AgentRow[]): { children: PaneChildren | null; devices: SnapshotRest | null } {
  const parent = (agentsByPane(agents).get(paneId)?.lineage_child_pane_ids?.length ?? 0) > 0;
  const deviceList = useShellStore((state) => (parent ? state.rest?.navigator?.devices : null));
  const remote = useShellStore((state) => (parent ? state.rest?.status?.remote : null));
  const devices = useMemo<SnapshotRest | null>(() => (parent ? { navigator: { devices: deviceList ?? undefined }, status: { remote: remote ?? undefined } } : null), [parent, deviceList, remote]);
  const children = useMemo(() => paneChildren(paneId, agents, devices), [paneId, agents, devices]);
  return { children, devices };
}

/**
 * The status mark, logo and title of an agent pane's head as one button once
 * the pane's session is known (PRD overview-device-tasks B22-B24): it shows
 * that session's record in the lane, and a press on the record already shown
 * goes back to the list. A shell, or an agent whose session is not known yet,
 * keeps the plain head. The press never takes the keyboard from the terminal.
 */
export function PaneRecordButton({ paneId, title, product, actions, children, plain }: { paneId: string; title: string; product: string; actions: Actions; children: ReactNode; plain: ReactNode }) {
  const { t } = useInterfaceTranslation();
  const agent = agentsByPane(usePaneAgents()).get(paneId);
  // Only this machine's id, so the head redraws on its own pane's changes rather than on every snapshot.
  const localId = useShellStore((s) => localDeviceId(s.rest));
  const session = agent ? sessionOfAgentOn(agent, localId) : null;
  if (!session) return <>{plain}</>;
  // The mark's words ride the button's name, which the mark sits inside (status-marks B36).
  const words = agent ? [markWords(t, agent), recordWords(t, agent), agent.record_summary].filter(Boolean).join(" · ") : null;
  return (
    // The button names the agent as the plain head does, then what a press
    // does. The pointer opens nothing here: the turn card opens from the
    // turn's own trigger only, and the record shows the whole title.
    <button
      type="button"
      // The turn on the head gives way before the identity's first words do.
      className="flex min-w-0 items-center gap-sm rounded-sm px-xxs -mx-xxs outline-none pane-turn-room:min-w-(--size-pane-title-min) hover:bg-accent hover:text-foreground focus-visible:ring-1 focus-visible:ring-ring"
      aria-label={[title, product, words, t("record.recordHint")].filter(Boolean).join(", ")}
      onMouseDown={(event) => event.preventDefault()}
      onClick={() => actions.togglePaneRecord(session)}
      data-pane-record={session.session_id}
    >
      {children}
    </button>
  );
}

/**
 * The pane's PR mark after its title, as the 태스크 row of the same agent
 * draws it (B21, B22): the core's list of its session and every descendant,
 * live or ended, so the operator sees a child's pull request from the pane
 * that delegated it, after the child is gone too.
 */
export function PanePrChip({ paneId, actions }: { paneId: string; actions: Actions }) {
  const agents = usePaneAgents();
  const agent = agentsByPane(agents).get(paneId);
  const pulls = agent ? rowPulls(agent) : [];
  if (!agent || pulls.length === 0) return null;
  // The pane names no project; the PR's URL finds the one that lists it.
  return <AgentPrMark agent={agent} pulls={pulls} side="bottom" onOpen={(pull) => actions.openSessionPullRequest({ workspace_id: null, url: pull.url, number: pull.number })} onExternal={(pull) => actions.openLink(pull.url, true)} />;
}

/** How many direct children the children button draws a mark for before `+N`. */
const CHILD_MARKS = 3;

/**
 * The children button at the row's left edge. Its face holds the tree, the
 * first direct children's status marks in tree order with `+N` for the rest,
 * and the chevron; its hint and name spell out every state count. A child of
 * unknown state draws no mark, so it is counted in `+N`, and with a device
 * away the face keeps the last total, since no mark can claim current work.
 */
function PaneTreeButton({ paneId, children, agents, actions, connection }: { paneId: string; children: PaneChildren; agents: AgentRow[]; actions: Actions; connection: string | null }) {
  const { t } = useInterfaceTranslation();
  const count = children.children.length;
  const label = [t("agentSessions.tree.children", { count }), ...(connection ? [connection] : children.parts.map((part) => t(part.label, { count: part.count })))].join(" · ");
  // A device that is away keeps its raised count off the button, while the
  // tree still opens on the raised branches that remain reachable.
  const raised = connection ? 0 : children.raised;
  const marked = connection ? [] : children.children.filter((child) => child.mark !== "unknown").slice(0, CHILD_MARKS);
  const rest = count - marked.length;
  return <AgentTreePopover
    parent={children.parent}
    agents={agents}
    raised={children.raised > 0}
    onOpenChild={(id, name) => actions.followRelation(paneId, id, name)}
    onTask={() => actions.revealTask(paneId)}
    returnFocus={() => document.querySelector<HTMLButtonElement>(`[data-pane-tree="${paneId}"]`)?.focus()}
    triggerLabel={label}
    trigger={<button type="button" aria-label={label} data-pane-tree={paneId} className={cn("flex h-(--size-sidebar-line-detail) shrink-0 items-center gap-xxs rounded-xs border bg-background px-xs text-micro text-subtle-foreground outline-none hover:bg-popover focus-visible:ring-1 focus-visible:ring-ring data-[state=open]:bg-secondary", raised > 0 ? "border-warning/60" : "border-border")}>
      <ListTreeIcon aria-hidden="true" className="size-(--size-icon-sm) text-muted-foreground" />
      {marked.map((child) => <StatusMark key={child.pane_id} mark={child.mark} className={markTone(child)} data-pane-tree-mark={child.pane_id} />)}
      {rest > 0 ? <span aria-hidden="true" className="font-mono">{marked.length > 0 ? `+${rest}` : rest}</span> : null}
      <ChevronDownIcon aria-hidden="true" className="size-(--size-icon-sm) text-muted-foreground" />
    </button>}
  />;
}
