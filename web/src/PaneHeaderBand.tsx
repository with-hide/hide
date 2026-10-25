import { ChevronDownIcon, GitForkIcon, ListTreeIcon } from "lucide-react";
import { useMemo, type ReactNode } from "react";
import type { TFunction } from "i18next";
import type { Actions } from "./actions";
import { agentProductName } from "./agentAdapters";
import { askWhat, VerbText } from "./components/agent-tree";
import { AgentPrMark, useAgentStaleness } from "./components/pr-mark";
import { familyPulls, type SessionPull } from "./prMark";
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
import { markTone, markWords, recordWords } from "./agentRow";
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
 * The pane's second row over its terminal (docs/UI_BEHAVIOR.md, Agent panes and
 * the Agents explorer): the child-tree entry, then the turn the core selects,
 * `<request> → <answer>`, where the answer is an ask, the pane agent's own
 * progress or a finished turn's result. A working turn with nothing beyond
 * its request shows the request alone: the status mark and the working line
 * already say it works. The View menu's mode shapes it: one line cuts the
 * request and the answer, an ask included, two lines give the request its own
 * line, and off leaves only the children button, the pane's own ask riding the
 * head as `PaneAskMark`. Neither the terminal viewport nor its PTY grid moves
 * for it.
 */
export function PaneHeaderBand({ paneId, header, actions }: { paneId: string; header: PaneHeader | undefined; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const relation = useUiStore((state) => state.relation);
  const outcome = useShellStore((state) => state.rest?.status?.pane_focus_request);
  const agents = usePaneAgents();
  const { children, devices } = usePaneChildren(paneId, agents);
  const childConnection = children?.unavailableDevice ? notConnectedText(devices?.status?.remote, devices?.navigator?.devices, children.unavailableDevice, t) : null;
  const mode = usePaneTurnRowMode();
  const off = mode === "off";
  // Off keeps only the children button; a raised descendant's ask stays on its warning border.
  const band = off ? null : header?.band ?? null;
  // A running plan's current step and its line take the progress place (PRD hide-plan B7).
  const planLine = useShellStore((state) => {
    const plan = planOfPane(state.rest, paneId);
    return plan ? planWorkLine(plan) : null;
  });
  // A Conductor's pane says its waiting decision, else the nodes a worker is
  // at, and nothing when no node runs: the mission line owns the place, so
  // the Conductor's own plan or progress does not stand in (PRD
  // mission-plan-visibility B9, D-21).
  const mission = useShellStore((state) => missionOfPane(state.rest, allAgents(state.agents, state.rest?.status?.remote), paneId));
  const missionLine = mission ? missionWorkLine(mission, t) : null;
  const ownProgress = band || off ? null : mission ? missionLine?.text ?? null : planLine ?? header?.progress ?? null;
  const request = off ? null : header?.request ?? null;
  const finished = !band && !off && (header?.finished ?? false);
  const waitingOnChildren = band || off ? null : header?.waiting_on_children ?? null;
  // Before any early return: a hook runs on every render.
  const unreachable = useUnreachable(band?.action?.pane_id ?? null);
  const workingLine = header?.working ? <div className="pointer-events-none absolute inset-x-0 top-0 z-10 h-[calc(2*var(--size-hairline))] bg-agent-working" data-pane-working-line={paneId} /> : null;
  if (!band && !children && !ownProgress && !request && !finished && !waitingOnChildren) return workingLine;
  const tracked = band?.action && relation?.sourcePaneId === paneId && relation.targetPaneId === band.action.pane_id ? relation : null;
  const progress = relationState(tracked, outcome, t);
  const navigation = progress?.phase === "failed" ? progress.message : progress?.phase === "pending" ? t("panes.relation.opening", { name: tracked!.label }) : null;
  const tree = children ? <PaneTreeButton paneId={paneId} children={children} agents={agents} actions={actions} connection={childConnection} /> : null;
  const answer = <>
    {band ? <AskBand paneId={paneId} band={band} navigation={navigation} unreachable={unreachable} failed={progress?.phase === "failed"} actions={actions} hasTree={children !== null} /> : <>
      {finished ? <span className="shrink-0 font-medium text-success" data-pane-finished={paneId}>{t("paneTurn.finished")}</span> : null}
      {waitingOnChildren ? <span className="min-w-0 flex-1 truncate" data-pane-children-wait={paneId}>{[statusText(t, "waiting"), t("agents.line.children", { count: waitingOnChildren })].join(" · ")}</span> : null}
      {ownProgress ? <Hint label={ownProgress}><span className={cn("min-w-0 flex-1 truncate", missionLine?.decision && "text-mission-hold")} data-pane-own-progress={paneId} data-pane-mission-line={missionLine ? (missionLine.decision ? "decision" : "running") : undefined}>{ownProgress}</span></Hint> : null}
      {childConnection ? <span className={cn("min-w-0 truncate", ownProgress ? "max-w-1/4 shrink" : "flex-1")} title={childConnection} data-pane-child-connection={paneId}>{childConnection}</span> : null}
    </>}
    {band?.action ? <button type="button" data-pane-band-open={paneId} disabled={unreachable !== null || progress?.phase === "pending" || (progress?.phase === "failed" && !progress.retryable)} aria-busy={progress?.phase === "pending"} className="shrink-0 rounded-xs border border-border bg-background px-xs py-xxs text-micro text-foreground outline-none hover:brightness-95 focus-visible:ring-1 focus-visible:ring-ring disabled:opacity-50" onClick={() => actions.followRelation(paneId, band.action!.pane_id, band.action!.label)}>{progress?.phase === "failed" && progress.retryable ? t("common.retry") : t("common.open")}</button> : null}
  </>;
  const kind = band?.kind ?? (finished ? "finished" : ownProgress ? "progress" : waitingOnChildren ? "waiting" : request ? "request" : "children");
  // A working turn with no word beyond its request, or a parent with no turn
  // to show, draws no answer at all, so two lines never add an empty one.
  const answered = (kind !== "request" && kind !== "children") || childConnection !== null;
  // An ask is the one row the operator acts on, so it keeps the raised
  // surface; a quiet turn sits in the head's surface.
  const tone = band ? "bg-secondary border-l-2 border-warning text-foreground" : "bg-card border-t border-border text-muted-foreground";
  const twoLines = mode === "two_lines";
  return <div className="absolute inset-x-0 top-0 z-10 bg-background">
    {/* One row whether or not it leads with a request, so the tree's
        popover stays open when the request comes or goes. One line caps the
        request at two fifths when an answer follows and cuts each part, so
        no turn grows over the terminal; two lines stack the parts, each cut. */}
    <div className={cn("flex min-w-0 gap-x-xs px-sm text-caption", twoLines ? "flex-col items-stretch" : "flex-nowrap items-center", tone)} data-pane-header-band={kind} data-pane-turn={request ? paneId : undefined} data-pane-turn-row-mode={mode}>
      {tree || request ? (
        <div key="request" className={cn("flex h-(--size-pane-header) min-w-0 shrink items-center gap-xs", !twoLines && answered ? "max-w-2/5" : "max-w-full")} data-pane-turn-part="request">
          {tree}
          {request ? <PaneTurnCard paneId={paneId} actions={actions} trigger={(state) => <TurnRequestButton paneId={paneId} request={request} state={state} />} /> : null}
        </div>
      ) : null}
      {answered ? (
        <div key="answer" className={cn("flex h-(--size-pane-header) min-w-0 max-w-full items-center gap-xs", !twoLines && "grow", !twoLines && !request && "basis-0")} data-pane-turn-part="answer">
          {request ? <span aria-hidden="true" className="shrink-0 text-subtle-foreground">→</span> : null}
          {answer}
        </div>
      ) : null}
    </div>
    {workingLine}
  </div>;
}

/**
 * The request half of the turn: the request's one line in quiet text, with
 * no word before it on screen (its name still says `요청`), a button the
 * keyboard reaches. The pointer or focus on it opens the turn
 * card and a press pins it; it never takes the keyboard from the terminal.
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
      onPointerEnter={state.onPointerEnter}
      onPointerLeave={state.onPointerLeave}
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
      className={cn("flex h-(--size-sidebar-line-detail) shrink-0 items-center gap-xxs rounded-xs border border-warning/60 px-xs text-micro outline-none hover:bg-popover focus-visible:ring-1 focus-visible:ring-ring data-[state=open]:bg-secondary", band.read && "opacity-(--opacity-read-status)")}
      onMouseDown={(event) => event.preventDefault()}
      onClick={state.onToggle}
      onPointerEnter={state.onPointerEnter}
      onPointerLeave={state.onPointerLeave}
      onFocus={state.onFocus}
      onBlur={state.onBlur}
      onKeyDown={state.onKeyDown}
    >
      <span className="font-semibold text-warning" data-verb={band.kind}>{verb}</span>
      <Elapsed since={band.since_unix_ms} className="font-mono text-muted-foreground" />
    </button>
  )} />;
}

/** The pane's own ask's verb, as the row's ask starts with it. */
function bandVerbWords(t: TFunction<"translation">, kind: PaneBand["kind"]): string {
  if (kind === "approval" || kind === "answer") return t(`agentSessions.verb.${kind}`);
  return t(kind === "blocked" ? "agentSessions.tag.blocked" : "agentSessions.tag.stopped");
}

/**
 * An ask, led by whose it is: the pane's own approval, answer, block or stop
 * starts with its verb, and a raised descendant's starts with that agent's
 * name and relationship, so the row never reads as its parent's own ask.
 * The pane's own ask dims once the operator has looked and stays until it is
 * answered. The older more-asks trigger remains only when it is the
 * available tree access.
 */
function AskBand({ paneId, band, navigation, unreachable, failed, actions, hasTree }: { paneId: string; band: PaneBand; navigation: string | null; unreachable: string | null; failed: boolean; actions: Actions; hasTree: boolean }) {
  const { t } = useInterfaceTranslation();
  const agents = usePaneAgents();
  const self = agentsByPane(agents).get(paneId);
  const raised = band.raised ?? null;
  const origin = raised?.verb === "draft" ? agentsByPane(agents).get(raised.raised_pane_id) : null;
  const originTitle = raised?.origin_title ?? (raised?.verb === "draft" ? origin?.identity_label : raised?.title);
  const originDepth = raised ? raised.path.length + (raised.verb === "draft" ? 1 : 0) : 0;
  const what = raised ? askWhat(t, raised.verb, raised.what, raised.subject) : band.kind === "approval" || band.kind === "answer" ? askWhat(t, band.kind, band.reason, band.subject) : band.reason;
  const minutes = band.since_unix_ms == null ? null : Math.max(0, Math.floor((Date.now() - band.since_unix_ms) / 60_000));
  // The path from the root already ends with the agent named; the wait is its verb's (B5).
  const who = raised ? [
    raised.verb === "draft" ? origin && agentProductName(origin.agent_kind) : agentProductName(raised.agent_kind),
    (raised.verb === "draft" && originTitle ? [...raised.path, originTitle] : raised.path).join(" › "),
    [raised.verb === "draft" ? origin?.checkout_label : raised.checkout, minutes === null ? null : t("agentSessions.waited", { minutes, verb: t(`agentSessions.verb.${raised.verb}`) })].filter(Boolean).join(" · "),
  ].filter(Boolean).join("\n") : null;
  const more = !hasTree && band.more > 0 && self ? <AgentTreePopover
    parent={self}
    agents={agents}
    raised
    onOpenChild={(id, label) => actions.followRelation(paneId, id, label)}
    onTask={() => actions.revealTask(paneId)}
    returnFocus={() => document.querySelector<HTMLButtonElement>(`[data-pane-band-more="${paneId}"]`)?.focus()}
    triggerLabel={t("agentSessions.moreAsks", { count: band.more })}
    trigger={<button type="button" data-pane-band-more={paneId} className="shrink-0 rounded-xs px-xs py-xxs text-micro text-muted-foreground underline-offset-2 outline-none hover:text-foreground hover:underline focus-visible:ring-1 focus-visible:ring-ring">{t("agentSessions.moreAsks", { count: band.more })}</button>}
  /> : null;
  const read = !raised && band.read;
  return <>
    <span className={cn("flex min-w-0 flex-[3] items-center gap-xs", read && "opacity-(--opacity-read-status)")} data-pane-band-read={read ? "true" : undefined}>
      {raised ? <VerbText verb={raised.verb} /> : band.kind === "approval" || band.kind === "answer" ? <VerbText verb={band.kind} /> : <span className="shrink-0 font-semibold text-warning" data-verb={band.kind}>{t(band.kind === "blocked" ? "agentSessions.tag.blocked" : "agentSessions.tag.stopped")}</span>}
      {raised && originTitle ? <Hint label={who!}><span className="flex min-w-0 max-w-1/3 shrink items-center gap-xxs font-semibold" data-pane-band-who={raised.raised_pane_id}><span className="min-w-0 truncate">{originTitle}</span><span className="shrink-0 text-muted-foreground">({t(originDepth > 2 ? "panes.children.descendant" : "panes.children.direct")})</span></span></Hint> : null}
      <Hint label={navigation ?? what ?? ""}><span className={cn("min-w-0 flex-1 truncate", failed && "text-destructive")} role={failed ? "alert" : navigation ? "status" : undefined} data-pane-band-navigation={navigation ? failed ? "failed" : "pending" : undefined}>{navigation ?? what}</span></Hint>
    </span>
    {unreachable && !navigation ? <span className="min-w-0 max-w-1/4 shrink truncate text-muted-foreground" title={unreachable} data-pane-band-unreachable="true">{unreachable}</span> : null}
    {raised?.unreceived_by && minutes !== null ? <span className="min-w-0 max-w-1/4 flex-1 truncate text-muted-foreground" data-pane-band-unreceived="true">{t("agentSessions.unreceived", { name: raised.unreceived_by, minutes })}</span> : null}
    {raised?.unreceived_by ? null : <Elapsed since={band.since_unix_ms} className={cn("shrink-0 font-mono text-micro text-muted-foreground", read && "opacity-(--opacity-read-status)")} />}
    {more}
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
    // request on the second row only, and the record shows the whole title.
    <button
      type="button"
      className="flex min-w-0 items-center gap-sm rounded-sm px-xxs -mx-xxs outline-none hover:bg-accent hover:text-foreground focus-visible:ring-1 focus-visible:ring-ring"
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
 * The pane's PR mark after its title, as the Sessions row of the same agent
 * draws it: a pane without children shows its own PRs (B21, B22), and a
 * parent shows `PR N` over itself and every descendant, so the operator sees a
 * child's pull request from the pane that delegated it.
 */
export function PanePrChip({ paneId, actions }: { paneId: string; actions: Actions }) {
  const agents = usePaneAgents();
  const byPane = agentsByPane(agents);
  const agent = byPane.get(paneId);
  const family = useMemo(() => (agent ? familyPulls(agent, byPane) : undefined), [agent, byPane]);
  if (!agent || (family ? family.length === 0 : !agent.state.pr)) return null;
  return <PaneAgentPrMark agent={agent} family={family} actions={actions} />;
}

function PaneAgentPrMark({ agent, family, actions }: { agent: AgentRow; family?: SessionPull[]; actions: Actions }) {
  const staleness = useAgentStaleness(agent, family);
  // The pane names no project; the PR's URL finds the one that lists it.
  return <AgentPrMark agent={agent} family={family} staleness={staleness} side="bottom" onOpen={(pull) => actions.openSessionPullRequest({ workspace_id: null, url: pull.url, number: pull.number })} onExternal={(pull) => actions.openLink(pull.url, true)} />;
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
