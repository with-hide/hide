import { MarkerType, ReactFlow, ReactFlowProvider, useReactFlow, useNodesInitialized, type NodeChange, type Edge, type Node, type NodeTypes, type EdgeTypes } from "@xyflow/react";
import { MaximizeIcon, MinusIcon, PlusIcon, XIcon, LocateFixedIcon, RotateCcwIcon } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { Actions } from "./actions";
import { AgentMark } from "./AgentMark";
import { agentProductName } from "./agentAdapters";
import { markWords, rowSentence } from "./agentRow";
import { Button } from "./components/ui/button";
import { useEscapeLayer, useReturnFocus } from "./components/ui/layer";
import { Hint } from "./components/ui/tooltip";
import { StatusMark } from "./components/status-mark";
import { summariesEnabled } from "./hideAi";
import { useInterfaceTranslation } from "./i18n/client";
import { formatDateTime } from "./i18n/format";
import { requireInterfaceLanguage } from "./i18n/locale";
import { frontDeviceId } from "./devices";
import { Input } from "./components/ui/input";
import { LAYOUT_NODE_LIMIT } from "./dependencyLayout";
import { initialGraphExpansion, graphSessionId, overviewGraphFamily, overviewGraphModel, type Communication, type GraphAgent, type GraphFocus } from "./overviewGraphModel";
import { useGraphReducedMotion, useGraphTheme, useOverviewGraphLayout } from "./overviewGraphLayout";
import { CommunicationGraphEdge, ProjectGraphNode, SessionGraphNode, letterKind, type LetterEdge, type ProjectNode, type SessionNode } from "./overviewGraphNodes";
import type { BoardProject } from "./projectBoard";
import { sessionOfAgentOn, workKey } from "./sessionWork";
import type { WorkProgress, WorkSession } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import "@xyflow/react/dist/base.css";
import "./overviewGraph.css";

type Props = { projects: BoardProject[]; deviceId: string; actions: Actions; onOpenRecord?: (session: WorkSession) => void };
type Selection = { kind: "session" | "letters" | "lineage"; id: string } | null;
const NODE_TYPES: NodeTypes = { project: ProjectGraphNode, session: SessionGraphNode };
const EDGE_TYPES: EdgeTypes = { letters: CommunicationGraphEdge };
const PAGE_SIZE = 50;

/** Production Overview owns the graph; the Factory continues to own its separate worker. */
export function OverviewGraph(props: Props) {
  const projectId = useUiStore((state) => state.overview.projectId);
  const [focus, setFocus] = useState<GraphFocus>("all");
  const [includeResting, setIncludeResting] = useState(false);
  const [scopedSelection, setScopedSelection] = useState<{ deviceId: string; projectId: string; sessionId: string } | null>(null);
  const revealProjectSession = (row: GraphAgent) => {
    setScopedSelection({ deviceId: props.deviceId, projectId: row.project.id, sessionId: row.id });
    useUiStore.getState().setOverview({ projectId: row.project.id });
  };
  const initialSelection = scopedSelection?.deviceId === props.deviceId && scopedSelection.projectId === projectId ? scopedSelection.sessionId : null;
  useEffect(() => { if (initialSelection) setScopedSelection(null); }, [initialSelection]);
  return <ReactFlowProvider key={`${props.deviceId}:${projectId ?? "all"}`}><WorkGraph {...props} projectId={projectId} focus={focus} setFocus={setFocus} includeResting={includeResting} setIncludeResting={setIncludeResting} initialSelection={initialSelection} revealProjectSession={revealProjectSession} /></ReactFlowProvider>;
}

function WorkGraph({ projects, deviceId, actions, onOpenRecord, projectId, focus, setFocus, includeResting, setIncludeResting, initialSelection, revealProjectSession }: Props & { projectId: string | null; focus: GraphFocus; setFocus: (focus: GraphFocus) => void; includeResting: boolean; setIncludeResting: (value: boolean) => void; initialSelection: string | null; revealProjectSession: (row: GraphAgent) => void }) {
  const { t } = useInterfaceTranslation();
  const rest = useShellStore((state) => state.rest);
  const summaries = useShellStore((state) => state.linkSummaries);
  const [expanded, setExpanded] = useState(() => initialGraphExpansion(projects, projectId, (rest?.ui_state?.recent_checkouts ?? []).filter((row) => row.device_id === deviceId).map((row) => row.checkout_id)));
  const initialized = useRef(projects.length > 0);
  useEffect(() => {
    if (initialized.current || projects.length === 0) return;
    initialized.current = true;
    setExpanded(initialGraphExpansion(projects, projectId, (rest?.ui_state?.recent_checkouts ?? []).filter((row) => row.device_id === deviceId).map((row) => row.checkout_id)));
  }, [projects, projectId, rest?.ui_state?.recent_checkouts, deviceId]);
  // Freeze the core-confirmed entry identity, including native session and graph digest.
  // A later pane reuse cannot become the entry selection.
  const [entry] = useState(() => {
    const store = useShellStore.getState();
    const pane = store.focusedPaneId;
    if (!pane || !store.rest || frontDeviceId(store.rest) !== deviceId) return null;
    const row = projects.flatMap((project) => project.agents).find((row) => row.pane_id === pane && (row.device_id ?? deviceId) === deviceId);
    const proof = store.rest.overview_graph?.sessions.find((row) => row.pane_id === pane && row.device_id === deviceId);
    return row?.session_id && proof ? { id: graphSessionId(deviceId, pane, proof.session_digest), native: row.session_id } : null;
  });
  const [highlightedSession, setHighlightedSession] = useState<string | null>(initialSelection ?? entry?.id ?? null);
  const [projectQuery, setProjectQuery] = useState("");
  const [offsets, setOffsets] = useState<Map<string, { x: number; y: number }>>(() => new Map());
  const [motion, setMotion] = useState(true);
  const [playback, setPlayback] = useState(0);
  const [playing, setPlaying] = useState(false);
  const [mode, setMode] = useState<"graph" | "list">("graph");
  const [selection, setSelection] = useState<Selection>(initialSelection ? { kind: "session", id: initialSelection } : null);
  const [hovered, setHovered] = useState<string | null>(null);
  const [focused, setFocused] = useState<string | null>(null);
  const [mail, setMail] = useState(true);
  const [direct, setDirect] = useState(true);
  const [folded, setFolded] = useState<Set<string>>(() => new Set());
  const [edgeFocus, setEdgeFocus] = useState<string | null>(null);
  const [edgeHover, setEdgeHover] = useState<string | null>(null);
  const [retry, setRetry] = useState(0);
  const host = useRef<HTMLDivElement>(null);
  const opener = useRef<HTMLElement | null>(null);
  const reduced = useGraphReducedMotion();
  const theme = useGraphTheme();
  const model = useMemo(() => overviewGraphModel(projects, rest?.overview_graph, expanded, projectId, { focus, includeResting, folded, pinned: entry ? [entry.id] : [], selected: highlightedSession }), [projects, rest?.overview_graph, expanded, projectId, focus, includeResting, folded, entry, highlightedSession]);
  useEffect(() => { if ([...folded].some((id) => !model.folded.has(id))) setFolded(new Set(model.folded)); }, [folded, model.folded]);
  const layout = useOverviewGraphLayout(model, host, retry);
  const flow = useReactFlow();
  const { fitView, zoomIn, zoomOut } = flow;
  const nodesInitialized = useNodesInitialized();
  const cameraInitialized = useRef(false);
  const current = model.allAgents.find((row) => row.id === entry?.id && row.agent.session_id === entry.native);
  const centerProject = useCallback((id: string) => {
    const box = layout.placement?.groups.get(id);
    if (!box) return;
    const offset = offsets.get(id);
    void flow.setCenter(box.x + box.width / 2 + (offset?.x ?? 0), box.y + box.height / 2 + (offset?.y ?? 0), { zoom: 0.8, duration: 0 });
  }, [flow, layout.placement, offsets]);
  const restoreCamera = useCallback(() => {
    const group = current ? layout.placement?.groups.get(current.projectId) : undefined;
    const point = current ? layout.placement?.nodes.get(current.id) : undefined;
    if (current && group && point && layout.size) {
      const canvas = host.current?.querySelector(".og-canvas")?.getBoundingClientRect();
      // Endpoint-safe relationship labels can extend above the member footprint.
      // Include their full kind stack when framing the readable current cluster.
      const labelMargin = Math.max(layout.size.spacing.betweenNodes, layout.size.compactHeight / 3) * 2 + layout.size.rootHeight / 5;
      const padding = Math.max(layout.size.spacing.padding, labelMargin);
      const zoom = canvas ? Math.min(0.95, canvas.width / (group.width + padding * 2), canvas.height / (group.height + padding * 2)) : 0.8;
      if (zoom >= 0.6) {
        void flow.setCenter(group.x + group.width / 2 + (offsets.get(current.projectId)?.x ?? 0), group.y + group.height / 2 + (offsets.get(current.projectId)?.y ?? 0), { zoom, duration: 0 });
        return;
      }
      void flow.setCenter(group.x + point.x + (offsets.get(current.projectId)?.x ?? 0) + (offsets.get(current.id)?.x ?? 0) + layout.size.rootWidth / 2,
        group.y + point.y + (offsets.get(current.projectId)?.y ?? 0) + (offsets.get(current.id)?.y ?? 0) + layout.size.rootHeight / 2, { zoom: 0.8, duration: 0 });
    } else if (model.projects[0]) centerProject(model.projects[0].id);
  }, [current, model.projects, centerProject, layout.placement, layout.size, offsets, flow]);
  useEffect(() => {
    if (cameraInitialized.current || !nodesInitialized || layout.phase !== "ready") return;
    cameraInitialized.current = true;
    restoreCamera();
  }, [nodesInitialized, layout.phase, restoreCamera]);
  useEffect(() => {
    const admitted = new Set([...model.projects.map((group) => group.id), ...model.allAgents.map((row) => row.id)]);
    setOffsets((previous) => [...previous.keys()].some((id) => !admitted.has(id)) ? new Map([...previous].filter(([id]) => admitted.has(id))) : previous);
  }, [model.structure, model.projects, model.allAgents]);
  useEffect(() => {
    if (!playback || !motion || reduced) { setPlaying(false); return; }
    const style = host.current ? getComputedStyle(host.current) : null;
    const duration = Number(style?.getPropertyValue("--graph-flow-step-ms")) * Number(style?.getPropertyValue("--graph-flow-steps"));
    if (!Number.isFinite(duration) || duration <= 0) throw new Error("missing graph playback duration");
    setPlaying(true);
    const timer = setTimeout(() => setPlaying(false), duration);
    return () => clearTimeout(timer);
  }, [playback, motion, reduced]);
  const progress = useMemo(() => new Map((summariesEnabled(rest?.status?.background_ai) ? summaries?.progress ?? [] : []).map((row) => [workKey(row.session), row])), [summaries, rest?.status?.background_ai]);
  const toggle = useCallback((id: string) => setExpanded((previous) => { const next = new Set(previous); if (next.has(id)) next.delete(id); else next.add(id); return next; }), []);
  const rememberOpener = useCallback((target?: HTMLElement) => {
    const active = target ?? document.activeElement;
    if (active instanceof HTMLElement && active !== document.body && !active.closest(".og-details")) opener.current = active;
  }, []);
  const highlight = useCallback((id: string | null, keyboard: boolean) => { if (keyboard) setFocused(id); else setHovered(id); }, []);
  const fold = useCallback((id: string) => setFolded((previous) => { const next = new Set(previous); if (next.has(id)) next.delete(id); else next.add(id); return next; }), []);
  const chooseSession = useCallback((id: string, target?: HTMLElement) => {
    const row = model.agents.find((row) => row.id === id);
    // An omitted project's uncapped list row has a deterministic route back to a graph.
    // Scope navigation is explicit user selection and does not move the existing map.
    if (row && !model.projects.some((group) => group.id === row.projectId)) { revealProjectSession(row); return; }
    rememberOpener(target); setHighlightedSession(id); setSelection({ kind: "session", id }); setPlayback((value) => value + 1);
  }, [rememberOpener, model.agents, model.projects, revealProjectSession]);
  const highlightLetter = useCallback((id: string | null, keyboard: boolean) => { if (keyboard) setEdgeFocus(id); else setEdgeHover(id); }, []);
  const chooseLetters = useCallback((id: string, target?: HTMLElement) => { rememberOpener(target); setSelection({ kind: id.startsWith("lineage:") ? "lineage" : "letters", id }); setPlayback((value) => value + 1); }, [rememberOpener]);
  const lineageBundle = selection?.kind === "lineage" ? model.lineage.find((edge) => `lineage:${edge.from}:${edge.to}` === selection.id) : undefined;
  const returnFocus = useReturnFocus(selection !== null, { target: () => opener.current, within: () => host.current?.querySelector<HTMLElement>(".og-details") ?? null });
  const close = useCallback(() => {
    returnFocus(new Event("closeAutoFocus", { cancelable: true }));
    setSelection(null);

  }, [returnFocus]);
  useEscapeLayer(selection !== null, close);
  const agent = [...model.agents, ...model.visible].find((row) => row.id === (selection?.kind === "session" ? selection.id : null));
  const bundle = selection?.kind === "letters" ? model.communication.find((row) => row.id === selection.id) : undefined;
  useEffect(() => { if (selection && (selection.kind === "session" ? !agent : selection.kind === "lineage" ? !lineageBundle : !bundle || !(bundle.kind === "herdr_direct" ? direct : mail))) close(); }, [selection, agent, bundle, lineageBundle, mail, direct, close]);
  useEffect(() => {
    if (highlightedSession && (!model.agents.some((row) => row.id === highlightedSession) || highlightedSession === entry?.id && !current)) setHighlightedSession(null);
  }, [highlightedSession, model.agents, entry?.id, current]);
  const activeSession = selection?.kind === "session" ? selection.id : selection ? null : highlightedSession;
  const emphasized = (id: string) => id === hovered || id === focused || id === activeSession;
  const related = new Set([hovered, focused, activeSession]);
  if (bundle) { related.add(bundle.source); related.add(bundle.target); }
  if (lineageBundle) { related.add(lineageBundle.from); related.add(lineageBundle.to); }
  for (const edge of [...model.lineage.map((edge) => ({ from: edge.from, to: edge.to })), ...model.communication.map((edge) => ({ from: edge.source, to: edge.target }))]) if (emphasized(edge.from) || emphasized(edge.to)) { related.add(edge.from); related.add(edge.to); }
  const openRecord = onOpenRecord ?? actions.openSessionRecord;
  const progressFor = (row: GraphAgent) => { const session = sessionOfAgentOn(row.agent, row.deviceId); return session ? progress.get(workKey(session)) ?? null : null; };
  // React Flow requires parents before their relative-positioned children.
  const nodes: Node[] = [];
  const placed = layout.placement;
  const size = layout.size;
  if (placed && size) {
    for (const group of model.projects) {
      const box = placed.groups.get(group.id);
      if (!box) continue;
      const project: ProjectNode = { id: group.id, type: "project", position: { x: box.x + (offsets.get(group.id)?.x ?? 0), y: box.y + (offsets.get(group.id)?.y ?? 0) }, width: box.width, height: box.height, measured: { width: box.width, height: box.height }, style: { width: box.width, height: box.height }, data: { group, toggle, current: current?.projectId === group.id, highlighted: related.has(group.id), highlight }, dragHandle: ".og-project-drag", draggable: true, connectable: false, focusable: false, selectable: false, zIndex: 1 };
      nodes.push(project);
    }
    for (const row of model.visible) {
      const revealSlot = model.projects.find((group) => group.id === row.projectId)?.revealSlot;
      const point = placed.nodes.get(row.id) ?? (revealSlot ? placed.nodes.get(revealSlot) : undefined);
      if (!point) continue;
      const root = model.roots.has(row.id);
      const compact = !model.projects.find((group) => group.id === row.projectId)?.expanded;
      // The layout owns these fixed dimensions. Retaining them also preserves React Flow's
      // measured handle bounds when hover/focus replaces a controlled node object.
      const measured = { width: compact ? size.compactWidth : root ? size.rootWidth : size.childWidth, height: compact ? size.compactHeight : root ? size.rootHeight : size.childHeight };
      const node: SessionNode = { id: row.id, parentId: row.projectId, type: "session", position: { x: point.x + (offsets.get(row.id)?.x ?? 0), y: point.y + (offsets.get(row.id)?.y ?? 0) }, ...measured, measured, selected: activeSession === row.id, data: { row, root, compact, progress: progressFor(row), context: model.context.has(row.id), highlighted: related.has(row.id), current: current?.id === row.id, quietCount: model.quietBranches.get(row.id) ?? 0, folded: model.folded.has(row.id), fold, highlight, choose: chooseSession }, dragHandle: ".og-session-drag", draggable: true, connectable: false, focusable: false, selectable: false, zIndex: 2 };
      nodes.push(node);
    }
  }
  const names = new Map([...model.projects.map((group): [string, string] => [group.id, group.project.workspace.label]), ...model.allAgents.map((row): [string, string] => [row.id, row.agent.identity_label])]);
  const nodesById = new Map(nodes.map((node) => [node.id, node]));
  const positionOf = (id: string) => {
    const node = nodesById.get(id);
    if (!node) return { x: 0, y: 0 };
    const parent = node.parentId ? nodesById.get(node.parentId) : null;
    return { x: node.position.x + (parent?.position.x ?? 0) + (node.width ?? 0) / 2, y: node.position.y + (parent?.position.y ?? 0) };
  };
  const activeEdge = selection?.kind !== "session" ? selection?.id : null;
  const hasEmphasis = Boolean(activeSession || hovered || focused || activeEdge || edgeHover || edgeFocus);
  const edgeClass = (active: boolean) => `${active ? " og-emphasized" : hasEmphasis ? " og-muted" : ""}${playing && (!hasEmphasis || active) ? " og-playing" : ""}`;
  const arrow = { type: MarkerType.ArrowClosed, color: "var(--agent-working)", width: 24, height: 24, orient: "auto-start-reverse" };
  const labelLaneStep = size ? Math.max(size.spacing.betweenNodes, size.compactHeight / 3) : 0;
  const handles = (source: string, target: string) => {
    const leftward = positionOf(source).x > positionOf(target).x;
    return { sourceHandle: leftward ? "source-left" : "source-right", targetHandle: leftward ? "target-right" : "target-left" };
  };
  const edges: Edge[] = model.lineage.map((edge): LetterEdge => {
    const id = `lineage:${edge.from}:${edge.to}`;
    const active = emphasized(edge.from) || emphasized(edge.to) || edgeHover === id || edgeFocus === id || activeEdge === id;
    const label = `${t("overviewGraph.lineage")}: ${names.get(edge.from)} → ${names.get(edge.to)}`;
    return { id, source: edge.from, target: edge.to, ...handles(edge.from, edge.to), type: "letters", className: `og-lineage${edgeClass(active)}`, ariaLabel: label,
      data: { lineage: edge, label, playback, emphasized: active, paired: false, laneOffset: 0, labelLaneStep, highlight: highlightLetter, choose: chooseLetters },
      markerEnd: { ...arrow, color: "var(--primary)" }, selected: activeEdge === id, focusable: false, selectable: false, zIndex: 0 };
  });
  const pairKey = (edge: Communication) => JSON.stringify([edge.source, edge.target].sort());
  const directPairs = new Set(model.communication.filter((edge) => edge.kind === "herdr_direct").map(pairKey));
  const letterPairs = new Set(model.communication.filter((edge) => edge.kind === "letters").map(pairKey));
  edges.push(...model.communication.filter((edge) => edge.kind === "herdr_direct" ? direct : mail).map((edge): LetterEdge => {
    const active = emphasized(edge.source) || emphasized(edge.target) || edgeHover === edge.id || edgeFocus === edge.id || selection?.kind === "letters" && selection.id === edge.id;
    const label = `${names.get(edge.source)} ${edge.directions.length > 1 ? "↔" : "→"} ${names.get(edge.target)}: ${t(edge.kind === "herdr_direct" ? "overviewGraph.directInputs" : "overviewGraph.letters", { count: edge.count })}`;
    const paired = directPairs.has(pairKey(edge)) && letterPairs.has(pairKey(edge));
    const laneOffset = labelLaneStep * (edge.kind === "herdr_direct" ? -1 : 1);
    return { id: edge.id, source: edge.source, target: edge.target, ...handles(edge.source, edge.target), type: "letters", className: `og-communication${edge.kind === "herdr_direct" ? " og-direct-input" : ""}${edgeClass(active)}`, ariaLabel: label, data: { bundle: edge, label, playback, emphasized: active, paired, laneOffset, labelLaneStep, highlight: highlightLetter, choose: chooseLetters }, markerEnd: edge.kind === "herdr_direct" ? { ...arrow, color: "var(--primary)" } : arrow, markerStart: edge.directions.length > 1 ? (edge.kind === "herdr_direct" ? { ...arrow, color: "var(--primary)" } : arrow) : undefined, selected: selection?.kind === "letters" && selection.id === edge.id, focusable: false, selectable: false, zIndex: 0 };
  }));
  const moveNodes = (changes: NodeChange[]) => {
    const moves = changes.filter((change) => change.type === "position" && change.position);
    if (!moves.length) return;
    setOffsets((previous) => {
      const next = new Map(previous);
      for (const change of moves) {
        if (change.type !== "position" || !change.position) continue;
        const revealSlot = model.projects.find((group) => group.revealed?.id === change.id)?.revealSlot;
        const base = placed?.groups.get(change.id) ?? placed?.nodes.get(change.id) ?? (revealSlot ? placed?.nodes.get(revealSlot) : undefined);
        if (base) next.set(change.id, { x: change.position.x - base.x, y: change.position.y - base.y });
      }
      return next;
    });
  };
  const known = new Set(nodes.map((node) => node.id));
  const visibleEdges = edges.filter((edge) => known.has(edge.source) && known.has(edge.target));
  const showList = mode === "list" || layout.phase === "failed" || !placed;
  return <div className="overview-graph" ref={host} data-overview-graph="true" data-graph-phase={layout.phase} data-playing={playing} data-motion={motion && !reduced}>
    <div className="og-toolbar">
      <div><h2>{t("overviewGraph.title")}</h2></div>
      <div className="og-toolbar-actions">{projectId ? <Button variant="ghost" size="sm" onClick={() => useUiStore.getState().setOverview({ projectId: null })}>{t("overviewGraph.allProjects")}</Button> : null}<Button variant="ghost" size="sm" onClick={() => setExpanded(new Set())}>{t("overviewGraph.collapseAll")}</Button><div role="group" aria-label={t("overviewGraph.title")}><Button variant={mode === "graph" ? "secondary" : "ghost"} size="sm" aria-pressed={mode === "graph"} onClick={() => setMode("graph")}>{t("overviewGraph.graph")}</Button><Button variant={mode === "list" ? "secondary" : "ghost"} size="sm" aria-pressed={mode === "list"} onClick={() => setMode("list")}>{t("overviewGraph.list")}</Button></div></div>
    </div>
    <div className="og-filters">
      <div role="group" aria-label={t("overviewGraph.filter")}>{(["all", "turn", "working", "results"] as const).map((value) => <Button key={value} variant={focus === value ? "secondary" : "ghost"} size="sm" aria-pressed={focus === value} onClick={() => setFocus(value)}>{t(`overviewGraph.focus.${value}`)} <span>{model.counts[value]}</span></Button>)}</div>
      <label><input type="checkbox" checked={includeResting} onChange={(event) => setIncludeResting(event.target.checked)} />{t("overviewGraph.includeResting")}</label>
      <label><input type="checkbox" checked={mail} onChange={(event) => setMail(event.target.checked)} />{t("overviewGraph.showLetters")}</label>
      <label><input type="checkbox" checked={direct} onChange={(event) => setDirect(event.target.checked)} />{t("overviewGraph.showDirectInputs")}</label>
      <label><input type="checkbox" checked={motion && !reduced} disabled={reduced} onChange={(event) => setMotion(event.target.checked)} />{t("overviewGraph.motion")}</label>
      <Button variant="ghost" size="sm" disabled={!motion || reduced} onClick={() => setPlayback((value) => value + 1)}>{t("overviewGraph.replay")}</Button>
    </div>
    {model.omitted.length > 0 ? <details className="og-omitted"><summary>{t("overviewGraph.omitted", { count: model.omitted.length })}</summary><ul>{model.omitted.map((group) => <li key={group.id}><span><strong>{group.project.workspace.label}</strong> · {t(`overviewGraph.omitted.${group.reason}`, { count: group.count })}</span><Button variant="ghost" size="sm" onClick={() => { setFocus("all"); setIncludeResting(true); useUiStore.getState().setOverview({ projectId: group.project.workspace.id }); }}>{t("overviewGraph.showProject")}</Button></li>)}</ul></details> : null}
    {model.limited ? <p className="og-notice" role="status">{t("overviewGraph.limit", { count: LAYOUT_NODE_LIMIT })}</p> : null}
    <div className="og-legend"><span>{t("overviewGraph.legend")}</span><span>{rest?.overview_graph?.truncated ? t("overviewGraph.letterLimit", { count: rest.overview_graph.limit }) : t("overviewGraph.letterRange")}</span>{rest?.overview_graph?.available !== true ? <span>{t("overviewGraph.lettersUnavailable")}</span> : null}</div>
    {model.agents.length > 0 && layout.phase === "loading" && mode === "graph" ? <p className="og-notice" role="status">{t("overviewGraph.loading")}</p> : null}
    {model.agents.length > 0 && layout.phase === "failed" && mode === "graph" ? <div className="og-notice" role="status"><span>{t("overviewGraph.failed")}</span><Button variant="outline" size="sm" onClick={() => setRetry((value) => value + 1)}>{t("overviewGraph.retry")}</Button></div> : null}
    {model.agents.length === 0 ? <div className="og-empty"><p>{t(model.allAgents.length > 0 ? "overviewGraph.filteredEmpty" : "overviewGraph.empty")}</p>{model.allAgents.length > 0 ? <Button variant="outline" size="sm" onClick={() => { setFocus("all"); setIncludeResting(true); }}>{t("overviewGraph.resetFilters")}</Button> : null}<Button variant="outline" size="sm" onClick={() => actions.openStartPanel(projectId ?? undefined)}>{t("overviewGraph.newAgent")}</Button></div> : null}
    {model.agents.length > 0 ? <div className="og-stage">
      <div className="og-canvas">{!showList ? <ReactFlow zIndexMode="manual" colorMode={theme} nodes={nodes} edges={visibleEdges} nodeTypes={NODE_TYPES} edgeTypes={EDGE_TYPES} nodesDraggable onNodesChange={moveNodes} nodesConnectable={false} nodesFocusable={false} edgesFocusable={false} elementsSelectable={false} deleteKeyCode={null} selectionKeyCode={null} multiSelectionKeyCode={null} panOnDrag panOnScroll zoomOnScroll defaultViewport={{ x: 24, y: 24, zoom: 1 }} minZoom={0.12} maxZoom={2} onPaneClick={() => { close(); setHighlightedSession(null); }} onEdgeMouseEnter={(_, edge) => setEdgeHover(edge.id)} onEdgeMouseLeave={() => setEdgeHover(null)} onEdgeClick={(_, edge) => { chooseLetters(edge.id); }} ariaLabelConfig={{ "node.a11yDescription.default": t("overviewGraph.nodeHelp"), "node.a11yDescription.keyboardDisabled": t("overviewGraph.nodeHelp"), "edge.a11yDescription.default": t("overviewGraph.edgeHelp") }}>
        <div className="og-zoom nopan nodrag"><Hint label={t("overviewGraph.zoomOut")}><Button variant="ghost" size="icon-sm" onClick={() => void zoomOut({ duration: 0 })}><MinusIcon /></Button></Hint><Hint label={t("overviewGraph.zoomIn")}><Button variant="ghost" size="icon-sm" onClick={() => void zoomIn({ duration: 0 })}><PlusIcon /></Button></Hint><Hint label={t("overviewGraph.fit")}><Button variant="ghost" size="icon-sm" onClick={() => void fitView({ padding: 0.08, duration: 0, maxZoom: 1 })}><MaximizeIcon /></Button></Hint><Hint label={t("overviewGraph.currentPane")}><Button variant="ghost" size="icon-sm" disabled={!current} onClick={() => { if (current) { setHighlightedSession(current.id); const node = nodes.find((node) => node.id === current.id); if (node) { const point = positionOf(node.id); void flow.setCenter(point.x, point.y + (node.height ?? 0) / 2, { zoom: 1, duration: 0 }); } else centerProject(current.projectId); } }}><LocateFixedIcon /></Button></Hint><Hint label={t("overviewGraph.resetLayout")}><Button variant="ghost" size="icon-sm" onClick={() => { setOffsets(new Map()); cameraInitialized.current = false; }}><RotateCcwIcon /></Button></Hint></div>
      </ReactFlow> : <SessionList key={`${focus}:${includeResting}`} rows={model.agents} selected={activeSession} choose={chooseSession} />}</div>
      <aside className="og-navigator" aria-label={t("overviewGraph.projects")}>
        <Input aria-label={t("overviewGraph.findProject")} placeholder={t("overviewGraph.findProject")} value={projectQuery} onChange={(event) => setProjectQuery(event.target.value)} />
        <div className="og-project-list">{model.projects.filter((group) => group.project.workspace.label.toLocaleLowerCase().includes(projectQuery.trim().toLocaleLowerCase())).map((group) => <div key={group.id}>
          <button type="button" aria-label={t("overviewGraph.locateProject", { project: group.project.workspace.label })} onClick={() => centerProject(group.id)}><strong>{group.project.workspace.label}</strong><span>{group.matches}</span></button>
          <Button variant="ghost" size="sm" onClick={() => useUiStore.getState().setOverview({ projectId: group.project.workspace.id })}>{t("overviewGraph.showProject")}</Button>
        </div>)}</div>
      </aside>
      {agent ? <SessionPreview row={agent} family={overviewGraphFamily(agent, model.allAgents)} progress={progressFor(agent)} actions={actions} openRecord={openRecord} close={close} /> : bundle ? <Correspondence bundle={bundle} names={names} close={close} /> : lineageBundle ? <aside className="og-details" aria-label={t("overviewGraph.lineage")}><div className="og-details-head"><strong>{t("overviewGraph.lineage")}</strong><Button variant="ghost" size="icon-sm" aria-label={t("overviewGraph.close")} onClick={close}><XIcon /></Button></div><p className="og-direction">{names.get(lineageBundle.from)} → {names.get(lineageBundle.to)}</p><p>{t("overviewGraph.delegations", { count: lineageBundle.count })}</p></aside> : null}
    </div> : null}
  </div>;
}

function SessionList({ rows, selected, choose }: { selected: string | null; rows: readonly GraphAgent[]; choose: (id: string, target: HTMLElement) => void }) {
  const { t } = useInterfaceTranslation();
  const [page, setPage] = useState(0);
  const [query, setQuery] = useState("");
  const filtered = rows.filter((row) => `${row.agent.identity_label} ${row.project.label} ${row.checkout.label}`.toLocaleLowerCase().includes(query.trim().toLocaleLowerCase()));
  const pages = Math.max(1, Math.ceil(filtered.length / PAGE_SIZE));
  const current = Math.min(page, pages - 1);
  return <div className="og-list" data-graph-session-list="true">
    <Input aria-label={t("overviewGraph.findSession")} placeholder={t("overviewGraph.findSession")} value={query} onChange={(event) => { setQuery(event.target.value); setPage(0); }} />
    {filtered.length === 0 ? <p>{t("overviewGraph.filteredEmpty")}</p> : null}
    {filtered.slice(current * PAGE_SIZE, (current + 1) * PAGE_SIZE).map((row) => <button key={row.id} type="button" aria-pressed={selected === row.id} onClick={(event) => choose(row.id, event.currentTarget)}><AgentMark kind={row.agent.agent_kind} /><span className="og-list-name"><strong>{row.agent.identity_label}</strong><small>{row.project.label} · {row.checkout.label}</small></span><StatusMark mark={row.agent.mark} /><span>{markWords(t, row.agent)}</span></button>)}
    {pages > 1 ? <div className="og-pagination"><Button variant="ghost" size="sm" disabled={current === 0} onClick={() => setPage(current - 1)}>{t("overviewGraph.previous")}</Button><span>{t("overviewGraph.page", { page: current + 1, pages })}</span><Button variant="ghost" size="sm" disabled={current + 1 === pages} onClick={() => setPage(current + 1)}>{t("overviewGraph.next")}</Button></div> : null}
  </div>;
}

function SessionPreview({ row, family, progress, actions, openRecord, close }: { row: GraphAgent; family: { direct: number; descendants: number }; progress: WorkProgress | null; actions: Actions; openRecord: (session: WorkSession) => void; close: () => void }) {
  const { t } = useInterfaceTranslation();
  const session = sessionOfAgentOn(row.agent, row.deviceId);
  const children = row.agent.direct_child_counts;
  const sentence = rowSentence(t, row.agent);
  return <aside className="og-details overview-narrow:absolute overview-narrow:inset-y-0 overview-narrow:right-0 overview-narrow:shadow-lg" aria-label={t("overviewGraph.preview")} data-graph-preview="true">
    <div className="og-details-head"><strong>{row.agent.identity_label}</strong><Hint label={t("overviewGraph.close")}><Button variant="ghost" size="icon-sm" onClick={close}><XIcon /></Button></Hint></div>
    <p className="og-detail-status"><StatusMark mark={row.agent.mark} />{markWords(t, row.agent)}</p>
    {sentence ? <p className="og-work">{sentence}</p> : null}
    <p className="og-context">{row.project.label} · {row.checkout.label}{row.device ? ` · ${row.device}` : ""}</p>
    <p className="og-context">{agentProductName(row.agent.agent_kind)}{row.launchModel ? ` · ${t("overviewGraph.launchModel", { model: row.launchModel })}` : ""}</p>
    {progress && progress.total > 0 ? <p>{t("overviewGraph.progress", { done: progress.done, total: progress.total })}</p> : null}
    <div className="og-children"><strong>{t("overviewGraph.children")}</strong><p>{t("overviewGraph.childSummary", family)}</p>{children && Object.values(children).some((value) => value > 0) ? <p>{t("overviewGraph.childStates", { working: children.working, turn: children.error + children.approval + children.question, done: children.done })}</p> : null}</div>
    <div className="og-details-actions"><Button variant="outline" size="sm" onClick={() => actions.openAgent(row.agent.pane_id)}>{t("overviewGraph.openSession")}</Button>{session ? <Button variant="secondary" size="sm" onClick={() => openRecord(session)}>{t("overviewGraph.openRecord")}</Button> : null}</div>
  </aside>;
}

function Correspondence({ bundle, names, close }: { bundle: Communication; names: ReadonlyMap<string, string>; close: () => void }) {
  const { t, i18n } = useInterfaceTranslation();
  return <aside className="og-details overview-narrow:absolute overview-narrow:inset-y-0 overview-narrow:right-0 overview-narrow:shadow-lg" aria-label={t(bundle.kind === "herdr_direct" ? "overviewGraph.directKind" : "overviewGraph.communication")} data-graph-correspondence="true">
    <div className="og-details-head"><strong>{t(bundle.kind === "herdr_direct" ? "overviewGraph.directKind" : "overviewGraph.communication")}</strong><Hint label={t("overviewGraph.close")}><Button variant="ghost" size="icon-sm" onClick={close}><XIcon /></Button></Hint></div>
    {bundle.directions.map((direction) => <p key={`${direction.from}:${direction.to}`} className="og-direction"><span>{names.get(direction.from)} → {names.get(direction.to)}</span><b>{t(bundle.kind === "herdr_direct" ? "overviewGraph.directInputs" : "overviewGraph.letters", { count: direction.count })}</b></p>)}
    {bundle.kind === "herdr_direct" ? <p className="og-context">{t("overviewGraph.directMeaning")}</p> : null}
    <dl>{Object.entries(bundle.kinds).map(([kind, count]) => <div key={kind}><dt>{letterKind(t, kind)}</dt><dd>{count}</dd></div>)}<div><dt>{t("overviewGraph.latest")}</dt><dd>{formatDateTime(requireInterfaceLanguage(i18n.language), bundle.latest, { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" })}</dd></div></dl>
  </aside>;
}
