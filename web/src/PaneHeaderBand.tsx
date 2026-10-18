import { ChevronDownIcon, CircleAlertIcon, GitForkIcon, GitPullRequestIcon, ListTreeIcon, MoonIcon } from "lucide-react";
import type { TFunction } from "i18next";
import { useMemo, type ReactNode } from "react";
import type { Actions } from "./actions";
import { agentProductName } from "./agentAdapters";
import { askWhat, VerbText } from "./components/agent-tree";
import { AgentPrMark, useAgentStaleness } from "./components/pr-mark";
import { reviewWord, sessionPulls, type SessionPull } from "./prMark";
import { AgentTreePopover, childrenOf } from "./components/agent-tree-popover";
import { StatusMark } from "./components/status-mark";
import { Elapsed } from "./components/elapsed";
import { Hint } from "./components/ui/tooltip";
import { useInterfaceTranslation } from "./i18n/client";
import type { MessageKey } from "./i18n/catalogs";
import { cn } from "./lib/utils";
import { localDeviceId, type PaneHeader, type PaneRow, type SnapshotRest, type AgentRow } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { relationState } from "./lineage";
import { deviceConnected } from "./devices";
import { remoteTargetOfPane } from "./remote";
import { useRelationProgress } from "./PaneRelations";
import { lineText, markWords, stateLine } from "./agentRow";
import { allAgents, agentsByPane } from "./paneAgents";
import { paneChildren, type PaneChildren } from "./paneChildren";
import { sessionOfAgentOn } from "./sessionWork";

const LABELS: Record<string, MessageKey> = {
  sleeping: "panes.sleep.captionSleeping", waking: "panes.sleep.captionWaking", failed: "panes.sleep.captionFailed", restore_failed: "panes.restore.failed",
  disconnected: "panes.transport.disconnected", closing: "panes.transport.closing", starting: "panes.transport.starting", reconnecting: "panes.transport.reconnecting", helper_not_allowed: "panes.transport.helperNotAllowed",
  unavailable: "panes.transport.remoteUnavailable", terminated: "panes.transport.remoteEnded", exit: "agentSessions.exitCode",
  controlled_elsewhere: "panes.transport.scrollElsewhere", device_offline: "panes.transport.disconnected",
  blocked: "agentSessions.tag.blocked", stopped: "agentSessions.tag.stopped",
  result: "agentSessions.tag.result", fix: "agentSessions.tag.fix", review: "agentSessions.tag.review", merge: "agentSessions.tag.merge",
};
const TONES = { muted: "bg-secondary text-muted-foreground", warning: "bg-warning/10 text-warning", error: "bg-destructive/10 text-destructive", success: "bg-success/10 text-success", pr: "bg-pr-mergeable/10 text-pr-mergeable" };
const ACTION_TONES = { muted: "bg-secondary text-secondary-foreground", warning: "bg-warning text-status-foreground", error: "bg-destructive text-destructive-foreground", success: "bg-success text-status-foreground", pr: "bg-pr-mergeable text-status-foreground" };

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

export function PaneHeaderBand({ paneId, header, actions }: { paneId: string; header: PaneHeader | undefined; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const relation = useUiStore((state) => state.relation);
  const outcome = useShellStore((state) => state.rest?.status?.pane_focus_request);
  const agents = usePaneAgents();
  const { children, devices } = usePaneChildren(paneId, agents);
  const childConnection = children?.unavailableDevice ? devices?.status?.remote?.find((row) => row.target_id === children.unavailableDevice)?.message ?? t("devices.rail.notConnected") : null;
  const band = header?.band;
  // Before any early return: a hook runs on every render.
  const unreachable = useUnreachable(band?.action?.kind === "child" ? band.action.pane_id : null);
  const workingLine = header?.working ? <div className="pointer-events-none absolute inset-x-0 top-0 z-10 h-[calc(2*var(--size-hairline))] bg-agent-working" data-pane-working-line={paneId} /> : null;
  if (!band) return children ? <div className="absolute inset-x-0 top-0 z-10 bg-background">
    <div className="flex h-[var(--size-pane-header)] min-w-0 items-center gap-xs bg-secondary px-sm text-caption text-muted-foreground" data-pane-header-band="children">
      <PaneTreeButton paneId={paneId} children={children} agents={agents} actions={actions} connection={childConnection} />
      <ChildSummary child={children.lead} connection={childConnection} />
    </div>
    {workingLine}
  </div> : workingLine;
  const action = band.action;
  const tracked = action?.kind === "child" && relation?.sourcePaneId === paneId && relation.targetPaneId === action.pane_id ? relation : null;
  const progress = relationState(tracked, outcome, t);
  const navigation = progress?.phase === "failed" ? progress.message : progress?.phase === "pending" ? t("panes.relation.opening", { name: tracked!.label }) : null;
  const ask = band.kind === "raised" || band.kind === "approval" || band.kind === "answer";
  const labelKey = LABELS[band.kind];
  if (!ask && !labelKey) throw new Error(`Unknown core pane band: ${band.kind}`);
  const label = ask ? null : band.kind === "exit" ? t("agentSessions.exitCode", { exitCode: band.exit_code! }) : t(labelKey as Exclude<MessageKey, "agentSessions.exitCode">);
  const Icon = ["sleeping", "waking"].includes(band.kind) ? MoonIcon : ["fix", "review", "merge"].includes(band.kind) ? GitPullRequestIcon : CircleAlertIcon;
  const reason = navigation ?? (ask ? null : bandReason(band, t));
  // An ask reads as text on a quiet fill with the warning rail (D-43); the verb carries the colour.
  return <div className="absolute inset-x-0 top-0 z-10 bg-background"><div className={cn("flex h-[var(--size-pane-header)] min-w-0 items-center gap-xs px-sm text-caption", ask ? "border-l-2 border-warning bg-secondary text-foreground" : TONES[band.tone])} data-pane-header-band={band.kind}>
    {children ? <PaneTreeButton paneId={paneId} children={children} agents={agents} actions={actions} compact={ask} connection={childConnection} /> : null}
    {ask ? <AskBand paneId={paneId} band={band} navigation={navigation} unreachable={unreachable} failed={progress?.phase === "failed"} actions={actions} hasTree={children !== null} /> : <>
      <Icon className="size-(--size-icon-sm) shrink-0" aria-hidden="true" /><span className={band.kind === "restore_failed" ? "min-w-0 truncate" : "shrink-0"}>{label}</span>
      <Hint label={reason ?? label!}><span className={cn("min-w-0 flex-1 truncate", progress?.phase === "failed" && "text-destructive")} role={progress?.phase === "failed" ? "alert" : progress?.phase === "pending" ? "status" : undefined} data-pane-band-navigation={progress?.phase}>{reason}</span></Hint>
      {band.more > 0 ? <span className="shrink-0 text-micro">+{band.more}</span> : null}
      <Elapsed since={band.since_unix_ms} className="shrink-0 font-mono text-micro" />
    </>}
    {action ? <button type="button" data-pane-band-open={paneId} disabled={unreachable !== null || progress?.phase === "pending" || (progress?.phase === "failed" && !progress.retryable)} aria-busy={progress?.phase === "pending"} className={cn("shrink-0 rounded-xs px-xs py-xxs text-micro outline-none hover:brightness-95 focus-visible:ring-1 focus-visible:ring-ring disabled:opacity-50", ask ? "border border-border bg-background text-foreground" : ACTION_TONES[band.tone])} onClick={() => action.kind === "child" ? actions.followRelation(paneId, action.pane_id, action.label) : actions.openSessionPullRequest(action)}>{progress?.phase === "failed" && progress.retryable ? t("common.retry") : action.kind === "child" ? t("common.open") : t("agentSessions.openPr")}</button> : null}
  </div></div>;
}

/**
 * An ask band: raised requests · verb · what · who · waited · Open.
 * The pane's own approval or question names no one; a raised descendant's
 * names its provider and title, whose hover gives the path from the root,
 * the checkout and the wait. The child trigger opens the raised branches;
 * the older more-asks trigger remains only when it is the available tree access.
 */
function AskBand({ paneId, band, navigation, unreachable, failed, actions, hasTree }: { paneId: string; band: NonNullable<PaneHeader["band"]>; navigation: string | null; unreachable: string | null; failed: boolean; actions: Actions; hasTree: boolean }) {
  const { t } = useInterfaceTranslation();
  const agents = usePaneAgents();
  const self = agentsByPane(agents).get(paneId);
  const raised = band.raised ?? null;
  const verb = raised?.verb ?? (band.kind === "answer" ? "answer" : "approval");
  const what = askWhat(t, verb, raised ? raised.what : band.reason);
  const minutes = band.since_unix_ms == null ? null : Math.max(0, Math.floor((Date.now() - band.since_unix_ms) / 60_000));
  // The path from the root already ends with the agent named; the wait is its verb's (B5).
  const who = raised ? [
    agentProductName(raised.agent_kind),
    raised.path.join(" › "),
    [raised.checkout, minutes === null ? null : t("agentSessions.waited", { minutes, verb: t(`agentSessions.verb.${verb}`) })].filter(Boolean).join(" · "),
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
  return <>
    <VerbText verb={verb} />
    <Hint label={navigation ?? what}><span className={cn("min-w-0 flex-[3] truncate", failed && "text-destructive")} role={failed ? "alert" : navigation ? "status" : undefined} data-pane-band-navigation={navigation ? failed ? "failed" : "pending" : undefined}>{navigation ?? what}</span></Hint>
    {unreachable && !navigation ? <span className="min-w-0 max-w-1/4 shrink truncate text-muted-foreground" title={unreachable} data-pane-band-unreachable="true">{unreachable}</span> : null}
    {raised ? <Hint label={who!}><span className="flex min-w-0 max-w-1/3 flex-1 items-center gap-xxs text-muted-foreground" data-pane-band-who={raised.pane_id}><span className="min-w-0 truncate">{raised.title}</span></span></Hint> : null}
    {raised?.unreceived_by && minutes !== null ? <span className="min-w-0 max-w-1/4 flex-1 truncate text-muted-foreground" data-pane-band-unreceived="true">{t("agentSessions.unreceived", { name: raised.unreceived_by, minutes })}</span> : null}
    {raised?.unreceived_by ? null : <Elapsed since={band.since_unix_ms} className="shrink-0 font-mono text-micro text-muted-foreground" />}
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
  const family = useMemo(() => agent && childrenOf(agent, byPane).length > 0 ? sessionPulls(agent, byPane) : undefined, [agent, byPane]);
  if (!agent || (family ? family.length === 0 : !agent.state.pr)) return null;
  return <PaneAgentPrMark agent={agent} family={family} actions={actions} />;
}

function PaneAgentPrMark({ agent, family, actions }: { agent: AgentRow; family?: SessionPull[]; actions: Actions }) {
  const staleness = useAgentStaleness(agent, family);
  // The pane names no project; the PR's URL finds the one that lists it.
  return <AgentPrMark agent={agent} family={family} staleness={staleness} onOpen={(pull) => actions.openSessionPullRequest({ workspace_id: null, url: pull.url, number: pull.number })} onExternal={(pull) => actions.openLink(pull.url, true)} />;
}

/** The second row's child trigger: core state counts, with the total in its hint. */
function PaneTreeButton({ paneId, children, agents, actions, compact = false, connection }: { paneId: string; children: PaneChildren; agents: AgentRow[]; actions: Actions; compact?: boolean; connection: string | null }) {
  const { t } = useInterfaceTranslation();
  const count = children.children.length;
  const label = [t("agentSessions.tree.children", { count }), ...(connection ? [connection] : children.parts.map((part) => t(part.label, { count: part.count })))].join(" · ");
  const parts = children.parts.filter((part) => part.mark !== null && (!compact || part.mark === "turn"));
  return <AgentTreePopover
    parent={children.parent}
    agents={agents}
    raised={children.raised > 0}
    onOpenChild={(id, name) => actions.followRelation(paneId, id, name)}
    onTask={() => actions.revealTask(paneId)}
    returnFocus={() => document.querySelector<HTMLButtonElement>(`[data-pane-tree="${paneId}"]`)?.focus()}
    triggerLabel={label}
    trigger={<button type="button" aria-label={label} data-pane-tree={paneId} className="flex h-(--size-sidebar-line-detail) shrink-0 items-center gap-xxs rounded-xs border border-border bg-background px-xs outline-none hover:bg-popover focus-visible:ring-1 focus-visible:ring-ring data-[state=open]:bg-secondary">
      {parts.length === 0 ? <ListTreeIcon aria-hidden="true" className="size-(--size-icon-sm) text-muted-foreground" /> : null}
      {parts.length ? <span className="flex items-center gap-xxs" aria-hidden="true">{parts.map((part) => <span key={part.kind} className="flex items-center font-mono text-micro text-muted-foreground" data-pane-child-count={part.kind}><StatusMark mark={part.mark} /><span>{part.count}</span></span>)}</span> : null}
      <ChevronDownIcon aria-hidden="true" className="size-(--size-icon-sm) text-muted-foreground" />
    </button>}
  />;
}

/** The existing core-ranked child line fills the otherwise unused band slot. */
function ChildSummary({ child, connection }: { child: AgentRow; connection: string | null }) {
  const { t } = useInterfaceTranslation();
  // A sleeping child says nothing more; a waking, failed or unknown one says its state (status-marks B31).
  const own = stateLine(child);
  const line = own === undefined ? child.state.line : own;
  const summary = connection ?? (line ? lineText(t, child, line) : child.identity_label);
  // With no line, a sleeping child's hover still says what its moon means.
  const state = line ? summary : child.mark === "asleep" ? markWords(t, child) : summary;
  return <Hint label={connection ?? [child.identity_label, agentProductName(child.agent_kind), state].filter((text, index, all) => all.indexOf(text) === index).join(" · ")}>
    <span className="flex min-w-0 flex-1 items-center gap-xxs" data-pane-child-summary={child.pane_id}><span className="min-w-0 truncate">{summary}</span></span>
  </Hint>;
}

/**
 * Localize the core's facts, saying only what the verb does not: a failed
 * check and asked-for changes. Review and Merge already say the rest, and a
 * fact Hide could not read stays off the band. No verb or attention decision
 * lives here.
 */
function bandReason(band: NonNullable<PaneHeader["band"]>, t: TFunction<"translation">): string | null {
  const facts = band.facts;
  if (!facts) return band.reason;
  const words = [facts.checks === "failed" ? t("agentSessions.pr.failed") : null, facts.review === "changes_requested" ? t(reviewWord(facts.review)!.key) : null];
  return words.filter(Boolean).join(" · ") || null;
}
