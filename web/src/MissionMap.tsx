import { GitBranchIcon, HistoryIcon, XIcon } from "lucide-react";
import { useMemo, useRef, useState, type ReactNode } from "react";
import type { Actions } from "./actions";
import { AgentMark } from "./AgentMark";
import { DependencyGraphView, type EdgeLook } from "./DependencyGraph";
import type { GraphEdge } from "./dependencyLayout";
import { formatElapsed, useElapsed } from "./components/elapsed";
import { PrMark } from "./components/pr-mark";
import { StatusMark } from "./components/status-mark";
import { Button } from "./components/ui/button";
import { Popover, PopoverAnchor, PopoverContent } from "./components/ui/popover";
import { Hint } from "./components/ui/tooltip";
import { useInterfaceTranslation } from "./i18n/client";
import { requireInterfaceLanguage } from "./i18n/locale";
import { cn } from "./lib/utils";
import { launchModelOf, missionCounts, missionOf, missionOpen, nodeLine, nodeMark, shownNodes, stackWaiting, stepsOf, workerWords, kindWord } from "./missions";
import { allAgents, agentsByPane } from "./paneAgents";
import { PR_LOOK, staleLabel } from "./prMark";
import { layerDependencies } from "./projectBoard";
import { machineName } from "./screenMachine";
import { sessionOfAgent } from "./sessionWork";
import type { Mission, MissionNode, PrState } from "./snapshot";
import { useShellStore } from "./store";

// A mission's map in the lane's 보기 slot (PRD software-factory-conductor
// D-06, D-14, D-42, B14-B21, B43): its nodes top down by what they wait on,
// each card with its worker, its PR mark and its four steps, and one line on
// top only when the operator has something to decide or no Conductor holds
// it. Every value is the core's (`MissionView`); the map words and draws it.

/** The card that stands for the report every node ends in; a node key is letters, digits, `-`, `_` and `.`, so none is this. */
const REPORT = "#report";

/** The left band's colour on a card with a pull request (D-42 A): the PR mark's own state colour. */
const BAND: Record<PrState, string> = {
  failed: "border-l-pr-failed",
  pending: "border-l-pr-pending",
  mergeable: "border-l-pr-mergeable",
  draft: "border-l-pr-draft",
  merged: "border-l-pr-merged",
  closed: "border-l-pr-closed",
};

export function MissionMap({ missionId, actions }: { missionId: string; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const mission = useShellStore((s) => missionOf(s.rest, missionId));
  const failed = useShellStore((s) => s.rest?.missions?.failed ?? null);
  if (failed) return <MapState text={t("mission.unreadable")} tone="text-destructive" state="unreadable" />;
  if (!mission) return <MapState text={t("mission.gone")} state="gone" />;
  return <MissionBody mission={mission} actions={actions} />;
}

function MapState({ text, tone, state }: { text: string; tone?: string; state: string }) {
  return <p className={cn("px-md py-md text-body text-muted-foreground", tone)} data-mission-state={state}>{text}</p>;
}

function MissionBody({ mission, actions }: { mission: Mission; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const [selected, setSelected] = useState<string | null>(null);
  const [unfolded, setUnfolded] = useState(false);
  const device = useShellStore((s) => (mission.stale ? machineName(s.rest?.navigator?.devices, mission.device_id, t) : null));
  const present = mission.conductor.present;
  const takeover = !present && missionOpen(mission);
  const { nodes, folded } = shownNodes(mission, unfolded);
  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col" data-mission={mission.id} data-mission-status={mission.status} data-mission-stale={mission.stale || undefined}>
      {takeover ? (
        <Banner
          mark={<span aria-hidden="true" className="size-(--size-agent-mark) shrink-0 rounded-full border border-dashed border-muted-foreground" />}
          text={[t("mission.noConductor"), mission.decision].filter(Boolean).join(" · ")}
          action={<Button size="sm" onClick={() => actions.takeOverMission(mission.id)} data-mission-takeover="true">{t("mission.takeOver")}</Button>}
          data-mission-banner="no-conductor"
        />
      ) : mission.decision && missionOpen(mission) ? (
        <Banner
          mark={<StatusMark mark="turn" />}
          text={t("mission.decide", { line: mission.decision })}
          tone="text-warning"
          action={<Button size="sm" variant="secondary" onClick={() => actions.openAgent(mission.conductor.pane_id)} data-mission-conversation="true">{t("mission.toConversation")}</Button>}
          data-mission-banner="decision"
        />
      ) : null}
      {/* Before the first plan the map is one line (B19). */}
      {mission.nodes.length > 0 ? <Summary mission={mission} /> : null}
      {mission.delay && missionOpen(mission) ? <p className="px-md pb-xs text-caption text-muted-foreground" data-mission-delay="true">{t("mission.delay", { line: mission.delay })}</p> : null}
      {device ? <p className="px-md pb-xs text-caption text-muted-foreground" data-mission-stale-note="true">{t("mission.stale", { device })}</p> : null}
      <div className={cn("min-h-0 flex-1 overflow-auto px-md pb-md pt-xs", mission.stale && "opacity-(--opacity-dimmed)")} data-mission-scroll="true">
        {nodes.length === 0 ? (
          <p className="text-body text-muted-foreground" data-mission-planning="true">{t("mission.planning")}</p>
        ) : (
          <Graph mission={mission} nodes={nodes} selected={selected} onSelect={setSelected} actions={actions} />
        )}
        {folded > 0 ? (
          <Button size="sm" variant="ghost" className="mt-sm" onClick={() => setUnfolded(true)} data-mission-unfold={folded}>
            {t("mission.folded", { count: folded })} · {t("mission.unfold")}
          </Button>
        ) : null}
      </div>
    </div>
  );
}

function Banner({ mark, text, tone, action, ...data }: { mark: ReactNode; text: string; tone?: string; action: ReactNode } & Record<`data-${string}`, string>) {
  return (
    <div className="mx-md mt-sm flex min-w-0 items-center gap-sm rounded-md bg-muted px-sm py-xs" {...data}>
      {mark}
      <span className={cn("min-w-0 flex-1 truncate text-body", tone)} title={text}>{text}</span>
      {action}
    </div>
  );
}

/** The numbers above the map (B19, B21): what the record holds, in the words of the mission's state. */
function Summary({ mission }: { mission: Mission }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const running = useElapsed(mission.status === "running" ? mission.started_at_unix_ms : null);
  const { done, total, dropped } = missionCounts(mission);
  const stack = stackWaiting(mission);
  const items: { label: string; value: string; key: string }[] = [];
  if (mission.status === "planning") {
    items.push({ key: "planning", label: t("mission.planning"), value: "" });
    items.push({ key: "nodes", label: t("mission.nodes"), value: String(mission.nodes.length) });
    items.push({ key: "scope", label: t("mission.scope"), value: t(`mission.scope.${mission.scope}`) });
  } else {
    if (mission.status === "running" && running) items.push({ key: "elapsed", label: t("mission.elapsed"), value: running });
    else if (mission.started_at_unix_ms != null) items.push({ key: "total", label: t("mission.total"), value: formatElapsed(language, (mission.closed_at_unix_ms ?? mission.started_at_unix_ms) - mission.started_at_unix_ms) });
    if (mission.status === "running") items.push({ key: "agents", label: t("mission.agents"), value: `${mission.running}/${mission.cap}` });
    items.push({ key: "done", label: t("mission.done"), value: `${done}/${total}` });
    if (dropped > 0) items.push({ key: "dropped", label: t("mission.dropped"), value: String(dropped) });
    if (mission.decision && missionOpen(mission)) items.push({ key: "decisions", label: t("mission.decisions"), value: "1" });
    if (mission.status === "merge_wait") items.push({ key: "merge-wait", label: t("mission.mergeWait"), value: t("mission.stack", { count: stack }) });
    if (mission.status === "done") items.push({ key: "finished", label: t("mission.finished"), value: "" });
    if (mission.status === "cancelled") items.push({ key: "cancelled", label: t("mission.cancelled"), value: "" });
    if (!mission.conductor.present && missionOpen(mission)) items.push({ key: "conductor", label: t("mission.conductor"), value: t("mission.none") });
  }
  return (
    <div className="flex min-w-0 flex-wrap items-baseline gap-x-lg gap-y-xxs px-md pb-xs pt-sm text-body" data-mission-summary="true">
      {items.map((item) => (
        <span key={item.key} className="flex items-baseline gap-xs" data-mission-stat={item.key}>
          <span className="text-muted-foreground">{item.label}</span>
          {item.value ? <span className="font-semibold tabular-nums text-foreground">{item.value}</span> : null}
        </span>
      ))}
    </div>
  );
}

type GraphCard = { key: string; node: MissionNode | null };

function Graph({ mission, nodes, selected, onSelect, actions }: { mission: Mission; nodes: MissionNode[]; selected: string | null; onSelect: (key: string | null) => void; actions: Actions }) {
  const graph = useMemo(() => {
    const shown = new Set(nodes.map((node) => node.key));
    const cards: GraphCard[] = [...nodes.map((node) => ({ key: node.key, node })), { key: REPORT, node: null }];
    const byKey = new Map(cards.map((card) => [card.key, card]));
    const depended = new Set(nodes.flatMap((node) => node.depends_on));
    const blockers = (card: GraphCard): GraphCard[] => {
      // Every node no other node waits on ends in the report.
      if (!card.node) return nodes.filter((node) => !depended.has(node.key)).map((node) => byKey.get(node.key)!);
      return card.node.depends_on.filter((key) => shown.has(key)).map((key) => byKey.get(key)!);
    };
    return layerDependencies(cards, (card) => card.key, blockers);
  }, [nodes]);
  const byKey = useMemo(() => new Map(nodes.map((node) => [node.key, node])), [nodes]);
  const edgeLook = (edge: GraphEdge): EdgeLook => {
    const from = byKey.get(edge.from);
    const to = byKey.get(edge.to);
    // A stack's edge: the pull request below is the one above's base (B15).
    if (from && to && from.pr != null && to.base_pr === from.pr) return { tone: "text-pr-merged", dashed: true };
    if (from && (from.status === "working" || from.status === "review")) return { tone: "text-agent-working" };
    return { tone: "text-border" };
  };
  return (
    <DependencyGraphView
      graph={graph}
      idOf={(card) => card.key}
      flow="down"
      gaps={{ layers: "--size-mission-layer-gap", nodes: "--size-mission-node-gap" }}
      edgeLook={edgeLook}
      draw={(card) =>
        card.node ? (
          <NodeCard key={card.key} mission={mission} node={card.node} open={selected === card.key} onOpen={(open) => onSelect(open ? card.key : null)} actions={actions} />
        ) : (
          <ReportCard key={card.key} mission={mission} />
        )
      }
    />
  );
}

/** A node's duration: to its finish, or counting while it runs. */
function Duration({ node }: { node: MissionNode }) {
  const { i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const running = useElapsed(node.finished_at_unix_ms == null ? node.started_at_unix_ms : null);
  const text = node.started_at_unix_ms == null ? null : node.finished_at_unix_ms != null ? formatElapsed(language, node.finished_at_unix_ms - node.started_at_unix_ms) : running;
  return text ? <span className="shrink-0 tabular-nums text-caption text-muted-foreground" data-mission-duration="true">{text}</span> : null;
}

/** The worker's live row, whose status mark the card wears (B15). */
function useWorkerRow(node: MissionNode) {
  return useShellStore((s) => {
    if (!node.pane_id) return null;
    const rows = agentsByPane(allAgents(s.agents, s.rest?.status?.remote));
    return rows.get(node.pane_id) ?? null;
  });
}

function NodeCard({ mission, node, open, onOpen, actions }: { mission: Mission; node: MissionNode; open: boolean; onOpen: (open: boolean) => void; actions: Actions }) {
  const { t, i18n } = useInterfaceTranslation();
  const row = useWorkerRow(node);
  const launchModel = useShellStore((s) => launchModelOf(s.rest, mission, node));
  const card = useRef<HTMLButtonElement>(null);
  const mark = nodeMark(node, row?.mark ?? null);
  const worker = workerWords(node, launchModel, t("mission.auto"));
  const lineText = nodeLine(node, t);
  const staleTip = staleLabel({ stale: mission.github_stale, lastRead: mission.github_read_at_unix_ms }, t, requireInterfaceLanguage(i18n.language));
  const pending = node.status === "planned" || node.blocked !== null;
  const attention = node.status === "failed" || node.status === "waiting";
  return (
    <Popover open={open} onOpenChange={onOpen}>
      <PopoverAnchor asChild>
        <button
          ref={card}
          type="button"
          onClick={() => onOpen(!open)}
          aria-expanded={open}
          aria-label={`${node.title} · ${t(`mission.status.${node.status}`)}`}
          data-dependency-node={node.key}
          data-mission-node={node.key}
          data-node-status={node.status}
          data-node-blocked={node.blocked ?? undefined}
          className={cn(
            "flex w-(--size-mission-node) min-w-0 flex-col gap-xxs rounded-md border border-border bg-card px-sm py-xs text-left outline-none hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring",
            pending && "border-dashed bg-transparent",
            node.status === "failed" && "border-destructive",
            // The band after the failed outline, so the outline leaves the band its PR colour.
            node.pr_state && "border-l-(length:--size-mission-band)",
            node.pr_state && BAND[node.pr_state],
            node.status === "dropped" && "opacity-(--opacity-dimmed)",
            open && "ring-1 ring-primary",
          )}
        >
          <span className="flex min-w-0 items-center gap-xs">
            <StatusMark mark={mark} />
            <span className={cn("min-w-0 flex-1 truncate text-body font-semibold", node.status === "dropped" && "line-through")} title={node.title}>{node.title}</span>
            {node.change ? (
              <Hint label={t("mission.changed")}>
                <HistoryIcon aria-label={t("mission.changed")} className="size-(--size-icon-sm) shrink-0 text-muted-foreground" data-node-changed="true" />
              </Hint>
            ) : null}
          </span>
          {worker ? (
            <span className="flex min-w-0 items-center gap-xs text-caption text-subtle-foreground">
              <AgentMark kind={node.worker?.agent} />
              <span className="min-w-0 flex-1 truncate" data-node-worker={worker}>{worker}</span>
              <Duration node={node} />
            </span>
          ) : null}
          <span className="flex min-w-0 items-center gap-xs">
            <Delivery node={node} staleTip={staleTip} stale={mission.github_stale} fallback={lineText} attention={attention} />
            <Steps node={node} />
          </span>
          {(node.pr_state || node.branch) && node.line ? <span className={cn("min-w-0 truncate text-caption", attention ? "text-destructive" : "text-muted-foreground")} title={node.line}>{node.line}</span> : null}
        </button>
      </PopoverAnchor>
      <PopoverContent
        align="start"
        side="bottom"
        className="w-(--size-mission-popover)"
        data-mission-detail={node.key}
        // The keyboard stays where it was, as everything the 보기 slot opens leaves it.
        onOpenAutoFocus={(event) => event.preventDefault()}
        onCloseAutoFocus={(event) => event.preventDefault()}
        // Focus going back to the terminal is not a dismissal, and the card's own press toggles it.
        onFocusOutside={(event) => event.preventDefault()}
        onInteractOutside={(event) => {
          if (event.target instanceof Node && card.current?.contains(event.target)) event.preventDefault();
        }}
      >
        <NodeDetail mission={mission} node={node} onClose={() => onOpen(false)} actions={actions} />
      </PopoverContent>
    </Popover>
  );
}

/** The card's third line: the PR mark and the base of a stack, or the branch of a direct change, or the node's own line (D-48, D-53). */
function Delivery({ node, stale, staleTip, fallback, attention }: { node: MissionNode; stale: boolean; staleTip: string | null; fallback: string; attention: boolean }) {
  const { t } = useInterfaceTranslation();
  if (node.pr != null && node.pr_state) {
    const mark = (
      <span className="flex min-w-0 flex-1 items-center gap-xs" data-node-pr={node.pr}>
        <PrMark state={node.pr_state} number={node.pr} stale={stale} />
        {node.base_pr != null ? <span className="truncate font-mono text-caption text-pr-merged" data-node-base={node.base_pr}>↰ {t("mission.base", { number: String(node.base_pr) })}</span> : null}
      </span>
    );
    return staleTip ? <Hint label={staleTip}>{mark}</Hint> : mark;
  }
  if (node.kind === "change" && node.branch) {
    return (
      <span className="flex min-w-0 flex-1 items-center gap-xxs text-caption text-subtle-foreground" data-node-branch={node.branch}>
        <GitBranchIcon aria-hidden="true" className="size-(--size-pr-icon) shrink-0" />
        <span className="truncate font-mono">{node.branch}</span>
      </span>
    );
  }
  return <span className={cn("min-w-0 flex-1 truncate text-caption", attention ? "text-destructive" : "text-muted-foreground")} title={fallback} data-node-line="true">{fallback}</span>;
}

/** The four steps of a change node (D-42 B): work, PR or branch, review, merge or main; a research or verify node draws none, its status mark says where it is. */
function Steps({ node }: { node: MissionNode }) {
  const { t } = useInterfaceTranslation();
  const steps = stepsOf(node);
  if (steps === 0) return null;
  const reached = node.stage ?? 0;
  const failing = node.status === "failed" || node.pr_state === "failed";
  const moving = node.status === "working" || node.status === "review";
  const label = t("mission.step", { step: reached, steps: t(node.delivery === "direct" ? "mission.steps.direct" : "mission.steps.pr") });
  return (
    <span role="img" aria-label={label} className="flex shrink-0 items-center gap-xxs" data-node-steps={reached}>
      {Array.from({ length: steps }, (_, index) => {
        const filled = index < reached;
        const last = index === reached - 1;
        const tone = !filled ? "bg-muted" : last && failing ? "bg-pr-failed" : last && moving && node.status !== "done" ? "bg-agent-working" : "bg-success";
        return <span key={index} aria-hidden="true" className={cn("h-(--size-mission-step-h) w-(--size-mission-step) rounded-full", tone)} />;
      })}
    </span>
  );
}

function ReportCard({ mission }: { mission: Mission }) {
  const { t } = useInterfaceTranslation();
  const stack = stackWaiting(mission);
  const note = mission.status === "done" ? t("mission.report.done") : mission.status === "merge_wait" && stack > 0 ? t("mission.report.stack") : mission.status === "cancelled" ? t("mission.cancelled") : t("mission.report.waiting");
  const done = mission.status === "done" || mission.status === "merge_wait";
  return (
    <div data-dependency-node={REPORT} data-mission-report={mission.status} className={cn("flex w-(--size-mission-node) items-center justify-center gap-xs rounded-xl border border-border px-md py-sm", done ? "bg-card" : "border-dashed")}>
      <StatusMark mark={mission.status === "done" ? "done" : "quiet"} />
      <span className="text-body font-semibold">{t("mission.report")}</span>
      <span className="text-caption text-muted-foreground">{note}</span>
    </div>
  );
}

/** A node's detail (B15, B18): its rows, and the way to its terminal, its record and its PR card. */
function NodeDetail({ mission, node, onClose, actions }: { mission: Mission; node: MissionNode; onClose: () => void; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const row = useWorkerRow(node);
  // Derived outside the selector: it builds a new session each call, which a selector must not return.
  const rest = useShellStore((s) => s.rest);
  const session = useMemo(() => (row ? sessionOfAgent(row, rest) : null), [row, rest]);
  const depends = node.depends_on.map((key) => mission.nodes.find((other) => other.key === key)?.title ?? key);
  const worker = node.worker;
  const rows: [string, ReactNode][] = [
    [t("mission.field.kind"), t(kindWord(node.kind))],
    ...(worker ? [[t("mission.field.agent"), (
      <span className="flex min-w-0 items-center gap-xs"><AgentMark kind={worker.agent} /><span className="truncate">{[worker.agent, worker.model].filter(Boolean).join(" · ")}</span></span>
    )] as [string, ReactNode]] : []),
    ...(worker?.effort ? [[t("mission.field.effort"), worker.effort_auto ? t("mission.effortAuto", { effort: worker.effort }) : worker.effort] as [string, ReactNode]] : []),
    ...(node.started_at_unix_ms != null ? [[t("mission.field.elapsed"), <Duration key="d" node={node} />] as [string, ReactNode]] : []),
    ...(node.attempts > 0 ? [[t("mission.field.attempts"), String(node.attempts)] as [string, ReactNode]] : []),
    ...(depends.length > 0 ? [[t("mission.field.depends"), depends.join(", ")] as [string, ReactNode]] : []),
    ...(node.pr != null ? [[t("mission.field.pr"), node.pr_state ? (
      <span className="flex items-center gap-xs"><PrMark state={node.pr_state} number={node.pr} stale={mission.github_stale} /><span className={PR_LOOK[node.pr_state].tone}>{t(PR_LOOK[node.pr_state].word)}</span></span>
    ) : `#${node.pr}`] as [string, ReactNode]] : []),
    ...(node.branch ? [[t("mission.field.branch"), <span key="b" className="font-mono">{node.branch}</span>] as [string, ReactNode]] : []),
    ...(node.review ? [[t("mission.field.review"), node.review] as [string, ReactNode]] : []),
    ...(node.line ? [[t("mission.field.now"), node.line] as [string, ReactNode]] : []),
    ...(node.change ? [[t("mission.field.change"), node.change] as [string, ReactNode]] : []),
    ...(node.criteria ? [[t("mission.field.criteria"), node.criteria] as [string, ReactNode]] : []),
  ];
  return (
    <div className="flex flex-col gap-sm">
      <div className="flex min-w-0 items-center gap-xs">
        <StatusMark mark={nodeMark(node, row?.mark ?? null)} />
        <span className="min-w-0 flex-1 truncate text-body font-semibold">{node.title}</span>
        <Button variant="ghost" size="icon-sm" aria-label={t("mission.closeNode")} onClick={onClose}><XIcon /></Button>
      </div>
      <dl className="grid grid-cols-[auto_1fr] gap-x-md gap-y-xs text-body">
        {rows.map(([label, value]) => (
          <div key={label} className="contents">
            <dt className="text-muted-foreground">{label}</dt>
            <dd className="min-w-0 break-words">{value}</dd>
          </div>
        ))}
      </dl>
      <div className="flex flex-wrap gap-xs">
        <Button size="sm" disabled={!node.pane_id || !row} onClick={() => node.pane_id && actions.openAgent(node.pane_id)} data-node-terminal="true">{t("mission.goTerminal")}</Button>
        <Button size="sm" variant="secondary" disabled={!session} onClick={() => session && actions.openChildRecord(session)} data-node-record="true">{t("mission.record")}</Button>
        <Button size="sm" variant="secondary" disabled={node.pr == null || !node.pr_url} onClick={() => node.pr != null && node.pr_url && actions.openSessionPullRequest({ workspace_id: mission.workspace_id, url: node.pr_url, number: node.pr })} data-node-pr-card="true">{t("mission.prCard")}</Button>
      </div>
    </div>
  );
}
