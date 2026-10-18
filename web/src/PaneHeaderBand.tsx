import { ChevronDownIcon, GitForkIcon, ListTreeIcon } from "lucide-react";
import { useMemo, type ReactNode } from "react";
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
import { localDeviceId, type PaneBand, type PaneHeader, type PaneRow, type SnapshotRest, type AgentRow } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { relationState } from "./lineage";
import { deviceConnected } from "./devices";
import { remoteTargetOfPane } from "./remote";
import { useRelationProgress } from "./PaneRelations";
import { markWords } from "./agentRow";
import { allAgents, agentsByPane } from "./paneAgents";
import { paneChildren, type PaneChildren } from "./paneChildren";
import { sessionOfAgentOn } from "./sessionWork";

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
 * the Agents explorer): the child-tree entry, then one operator request or
 * the current pane agent's own progress sentence selected by the core.
 * Neither the terminal viewport nor its PTY grid moves for it.
 */
export function PaneHeaderBand({ paneId, header, actions }: { paneId: string; header: PaneHeader | undefined; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const relation = useUiStore((state) => state.relation);
  const outcome = useShellStore((state) => state.rest?.status?.pane_focus_request);
  const agents = usePaneAgents();
  const { children, devices } = usePaneChildren(paneId, agents);
  const childConnection = children?.unavailableDevice ? devices?.status?.remote?.find((row) => row.target_id === children.unavailableDevice)?.message ?? t("devices.rail.notConnected") : null;
  const band = header?.band ?? null;
  const ownProgress = band ? null : header?.progress ?? null;
  // Before any early return: a hook runs on every render.
  const unreachable = useUnreachable(band?.action?.pane_id ?? null);
  const workingLine = header?.working ? <div className="pointer-events-none absolute inset-x-0 top-0 z-10 h-[calc(2*var(--size-hairline))] bg-agent-working" data-pane-working-line={paneId} /> : null;
  if (!band && !children && !ownProgress) return workingLine;
  const tracked = band?.action && relation?.sourcePaneId === paneId && relation.targetPaneId === band.action.pane_id ? relation : null;
  const progress = relationState(tracked, outcome, t);
  const navigation = progress?.phase === "failed" ? progress.message : progress?.phase === "pending" ? t("panes.relation.opening", { name: tracked!.label }) : null;
  return <div className="absolute inset-x-0 top-0 z-10 bg-background">
    <div className={cn("flex h-[var(--size-pane-header)] min-w-0 items-center gap-xs bg-secondary px-sm text-caption", band ? "border-l-2 border-warning text-foreground" : "text-muted-foreground")} data-pane-header-band={band?.kind ?? (ownProgress ? "progress" : "children")}>
      {children ? <PaneTreeButton paneId={paneId} children={children} agents={agents} actions={actions} connection={childConnection} /> : null}
      {band ? <AskBand paneId={paneId} band={band} navigation={navigation} unreachable={unreachable} failed={progress?.phase === "failed"} actions={actions} hasTree={children !== null} /> : <>
        {ownProgress ? <Hint label={ownProgress}><span className="min-w-0 flex-1 truncate" data-pane-own-progress={paneId}>{ownProgress}</span></Hint> : null}
        {childConnection ? <span className={cn("min-w-0 truncate", ownProgress ? "max-w-1/4 shrink" : "flex-1")} title={childConnection} data-pane-child-connection={paneId}>{childConnection}</span> : null}
      </>}
      {band?.action ? <button type="button" data-pane-band-open={paneId} disabled={unreachable !== null || progress?.phase === "pending" || (progress?.phase === "failed" && !progress.retryable)} aria-busy={progress?.phase === "pending"} className="shrink-0 rounded-xs border border-border bg-background px-xs py-xxs text-micro text-foreground outline-none hover:brightness-95 focus-visible:ring-1 focus-visible:ring-ring disabled:opacity-50" onClick={() => actions.followRelation(paneId, band.action!.pane_id, band.action!.label)}>{progress?.phase === "failed" && progress.retryable ? t("common.retry") : t("common.open")}</button> : null}
    </div>
    {workingLine}
  </div>;
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
  const what = raised ? askWhat(t, raised.verb, raised.what) : band.kind === "approval" || band.kind === "answer" ? askWhat(t, band.kind, band.reason) : band.reason;
  const minutes = band.since_unix_ms == null ? null : Math.max(0, Math.floor((Date.now() - band.since_unix_ms) / 60_000));
  // The path from the root already ends with the agent named; the wait is its verb's (B5).
  const who = raised ? [
    agentProductName(raised.agent_kind),
    raised.path.join(" › "),
    [raised.checkout, minutes === null ? null : t("agentSessions.waited", { minutes, verb: t(`agentSessions.verb.${raised.verb}`) })].filter(Boolean).join(" · "),
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
      {raised ? <Hint label={who!}><span className="flex min-w-0 max-w-1/3 shrink items-center gap-xxs font-semibold" data-pane-band-who={raised.pane_id}><span className="min-w-0 truncate">{raised.title}</span><span className="shrink-0 text-muted-foreground">({t("panes.children.descendant")})</span></span></Hint> : null}
      <Hint label={navigation ?? what ?? ""}><span className={cn("min-w-0 flex-1 truncate", failed && "text-destructive")} role={failed ? "alert" : navigation ? "status" : undefined} data-pane-band-navigation={navigation ? failed ? "failed" : "pending" : undefined}>{navigation ?? what}</span></Hint>
    </span>
    {unreachable && !navigation ? <span className="min-w-0 max-w-1/4 shrink truncate text-muted-foreground" title={unreachable} data-pane-band-unreachable="true">{unreachable}</span> : null}
    {raised?.unreceived_by && minutes !== null ? <span className="min-w-0 max-w-1/4 flex-1 truncate text-muted-foreground" data-pane-band-unreceived="true">{t("agentSessions.unreceived", { name: raised.unreceived_by, minutes })}</span> : null}
    {raised?.unreceived_by ? null : <Elapsed since={band.since_unix_ms} className={cn("shrink-0 font-mono text-micro text-muted-foreground", read && "opacity-(--opacity-read-status)")} />}
    {more}
  </>;
}

/** Why a pane on a device that is not connected cannot be opened now, or null when it can (B8). */
function useUnreachable(paneId: string | null): string | null {
  const { t } = useInterfaceTranslation();
  return useShellStore((state) => {
    if (paneId === null) return null;
    const device = remoteTargetOfPane(state.rest, paneId);
    if (device === null || deviceConnected(state.rest, device)) return null;
    return state.rest?.status?.remote?.find((row) => row.target_id === device)?.message ?? t("devices.rail.notConnected");
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
  // The mark's words ride the button's hint and name, which the mark sits inside (status-marks B36).
  const words = agent ? markWords(t, agent) : null;
  return (
    // The button names the agent as the plain head does, then what a press
    // does; the hint reveals a title the head cuts short.
    <Hint label={[title, product, words, t("record.recordHint")].filter(Boolean).join("\n")} reveals>
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
    </Hint>
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
  return <AgentPrMark agent={agent} family={family} staleness={staleness} onOpen={(pull) => actions.openSessionPullRequest({ workspace_id: null, url: pull.url, number: pull.number })} onExternal={(pull) => actions.openLink(pull.url, true)} />;
}

/**
 * The children button at the row's left edge. Its hint and name spell out
 * every state count while the face holds only tree, direct-child count and chevron.
 */
function PaneTreeButton({ paneId, children, agents, actions, connection }: { paneId: string; children: PaneChildren; agents: AgentRow[]; actions: Actions; connection: string | null }) {
  const { t } = useInterfaceTranslation();
  const count = children.children.length;
  const label = [t("agentSessions.tree.children", { count }), ...(connection ? [connection] : children.parts.map((part) => t(part.label, { count: part.count })))].join(" · ");
  // A device that is away keeps its raised count off the button, while the
  // tree still opens on the raised branches that remain reachable.
  const raised = connection ? 0 : children.raised;
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
      <span aria-hidden="true" className="font-mono">{count}</span>
      <ChevronDownIcon aria-hidden="true" className="size-(--size-icon-sm) text-muted-foreground" />
    </button>}
  />;
}
