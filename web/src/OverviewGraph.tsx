import { MarkerType, ReactFlow, ReactFlowProvider, useReactFlow, useNodesInitialized, type NodeChange, type Edge, type Node, type NodeTypes, type EdgeTypes } from "@xyflow/react";
import { MaximizeIcon, MinusIcon, PlusIcon, XIcon, LocateFixedIcon, RotateCcwIcon } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { Actions } from "./actions";
import { AgentMark } from "./AgentMark";
import { agentProductName } from "./agentAdapters";
import { markWords, rowAccessibleName } from "./agentRow";
import { AgentRecordTitle } from "./components/agent-record-title";
import { askWhat } from "./components/agent-tree";
import { RevisitStar, starred } from "./components/revisit-mark";
import { Button } from "./components/ui/button";
import { useEscapeLayer, useReturnFocus } from "./components/ui/layer";
import { Hint } from "./components/ui/tooltip";
import { StatusMark } from "./components/status-mark";
import { useInterfaceTranslation } from "./i18n/client";
import { frontDeviceId } from "./devices";
import { Input } from "./components/ui/input";
import { LAYOUT_NODE_LIMIT } from "./dependencyLayout";
import { graphActivity, graphSessionId, overviewGraphModel, overviewGraphTeam, type Communication, type GraphAgent, type GraphFocus, type OverviewGraphModel, type ProjectSummary } from "./overviewGraphModel";
import { useGraphTheme, useOverviewGraphLayout } from "./overviewGraphLayout";
import { GraphLine, GraphStateCounts, hasGraphLine, LineGraphEdge, OperatorGraphNode, ProjectGraphNode, SessionGraphNode, TeamGraphNode, type LineEdge, type OperatorNode, type ProjectNode, type SessionNode, type TeamNode } from "./overviewGraphNodes";
import type { BoardProject } from "./projectBoard";
import { sessionOfAgentOn } from "./sessionWork";
import type { WorkSession } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import "@xyflow/react/dist/base.css";
import "./overviewGraph.css";

type Props = { projects: BoardProject[]; deviceId: string; actions: Actions; onOpenRecord?: (session: WorkSession) => void };
const NODE_TYPES: NodeTypes = { operator: OperatorGraphNode, project: ProjectGraphNode, team: TeamGraphNode, session: SessionGraphNode };
const EDGE_TYPES: EdgeTypes = { line: LineGraphEdge };
const PAGE_SIZE = 50;
const OPERATOR = "operator";
/** Below this zoom the whole map is unreadable, so the first camera frames the current session instead. */
const READABLE_ZOOM = 0.5;

/** Production Overview owns the graph; the Factory continues to own its separate worker. */
export function OverviewGraph(props: Props) {
  const projectId = useUiStore((state) => state.overview.projectId);
  const [focus, setFocus] = useState<GraphFocus>("all");
  const [includeResting, setIncludeResting] = useState(false);
  const [hiddenProjects, setHiddenProjects] = useState<ReadonlySet<string>>(() => new Set());
  const [scopedSelection, setScopedSelection] = useState<{ deviceId: string; projectId: string; sessionId: string } | null>(null);
  const revealProjectSession = (row: GraphAgent) => {
    setScopedSelection({ deviceId: props.deviceId, projectId: row.project.id, sessionId: row.id });
    useUiStore.getState().setOverview({ projectId: row.project.id });
  };
  const initialSelection = scopedSelection?.deviceId === props.deviceId && scopedSelection.projectId === projectId ? scopedSelection.sessionId : null;
  useEffect(() => { if (initialSelection) setScopedSelection(null); }, [initialSelection]);
  return <ReactFlowProvider key={`${props.deviceId}:${projectId ?? "all"}`}><WorkGraph {...props} projectId={projectId} focus={focus} setFocus={setFocus} includeResting={includeResting} setIncludeResting={setIncludeResting} hiddenProjects={hiddenProjects} setHiddenProjects={setHiddenProjects} initialSelection={initialSelection} revealProjectSession={revealProjectSession} /></ReactFlowProvider>;
}

type GraphProps = Props & { projectId: string | null; focus: GraphFocus; setFocus: (focus: GraphFocus) => void; includeResting: boolean; setIncludeResting: (value: boolean) => void; hiddenProjects: ReadonlySet<string>; setHiddenProjects: (update: (previous: ReadonlySet<string>) => ReadonlySet<string>) => void; initialSelection: string | null; revealProjectSession: (row: GraphAgent) => void };

/** An orb center and radius in flow coordinates, where every line starts and ends. */
type Orb = { x: number; y: number; r: number };

/** The segment between two orbs, cut back so it starts and ends at their edges (and leaves its arrow room). */
function between(from: Orb, to: Orb, clearance: number): LineEdge["data"] | null {
  const length = Math.hypot(to.x - from.x, to.y - from.y);
  if (length <= from.r + to.r + clearance * 2) return null;
  const ux = (to.x - from.x) / length;
  const uy = (to.y - from.y) / length;
  return { x1: from.x + ux * (from.r + clearance), y1: from.y + uy * (from.r + clearance), x2: to.x - ux * (to.r + clearance), y2: to.y - uy * (to.r + clearance) };
}

/** The convex hull of the points, counterclockwise in the drawing's own axes (monotone chain). */
function hull(points: readonly { x: number; y: number }[]): { x: number; y: number }[] {
  const sorted = [...points].sort((a, b) => a.x - b.x || a.y - b.y);
  if (sorted.length < 3) return sorted;
  const cross = (o: { x: number; y: number }, a: { x: number; y: number }, b: { x: number; y: number }) => (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x);
  const lower: { x: number; y: number }[] = [];
  for (const point of sorted) { while (lower.length >= 2 && cross(lower.at(-2)!, lower.at(-1)!, point) <= 0) lower.pop(); lower.push(point); }
  const upper: { x: number; y: number }[] = [];
  for (const point of [...sorted].reverse()) { while (upper.length >= 2 && cross(upper.at(-2)!, upper.at(-1)!, point) <= 0) upper.pop(); upper.push(point); }
  return [...lower.slice(0, -1), ...upper.slice(0, -1)];
}

/** A rounded outline `pad` outside every point, as a path relative to (left, top). */
function outline(points: readonly { x: number; y: number }[], pad: number, left: number, top: number): string {
  const shape = hull(points).map((point) => ({ x: point.x - left, y: point.y - top }));
  if (shape.length === 1) { const [p] = shape; return `M ${p!.x - pad},${p!.y} a ${pad},${pad} 0 1 0 ${pad * 2},0 a ${pad},${pad} 0 1 0 ${-pad * 2},0 Z`; }
  const normals = shape.map((point, index) => {
    const next = shape[(index + 1) % shape.length]!;
    const length = Math.hypot(next.x - point.x, next.y - point.y) || 1;
    return { x: (next.y - point.y) / length, y: -(next.x - point.x) / length };
  });
  const at = (index: number, normal: { x: number; y: number }) => `${shape[index]!.x + normal.x * pad},${shape[index]!.y + normal.y * pad}`;
  let path = `M ${at(0, normals.at(-1)!)}`;
  shape.forEach((_, index) => {
    path += ` A ${pad},${pad} 0 0 1 ${at(index, normals[index]!)}`;
    path += ` L ${at((index + 1) % shape.length, normals[index]!)}`;
  });
  return `${path} Z`;
}

function WorkGraph({ projects, deviceId, actions, onOpenRecord, projectId, focus, setFocus, includeResting, setIncludeResting, hiddenProjects, setHiddenProjects, initialSelection, revealProjectSession }: GraphProps) {
  const { t } = useInterfaceTranslation();
  const rest = useShellStore((state) => state.rest);
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
  const [mode, setMode] = useState<"graph" | "list">("graph");
  const [selection, setSelection] = useState<string | null>(initialSelection);
  const [hovered, setHovered] = useState<string | null>(null);
  const [focused, setFocused] = useState<string | null>(null);
  const [folded, setFolded] = useState<Set<string>>(() => new Set());
  const [retry, setRetry] = useState(0);
  const host = useRef<HTMLDivElement>(null);
  const opener = useRef<HTMLElement | null>(null);
  const theme = useGraphTheme();
  const model = useMemo(() => overviewGraphModel(projects, rest?.overview_graph, projectId, { focus, includeResting, folded, pinned: entry ? [entry.id] : [], selected: highlightedSession }), [projects, rest?.overview_graph, projectId, focus, includeResting, folded, entry, highlightedSession]);
  useEffect(() => { if ([...folded].some((id) => !model.folded.has(id))) setFolded(new Set(model.folded)); }, [folded, model.folded]);
  const layout = useOverviewGraphLayout(model, host, retry);
  const flow = useReactFlow();
  const { fitView, zoomIn, zoomOut } = flow;
  const nodesInitialized = useNodesInitialized();
  const cameraInitialized = useRef(false);
  const current = model.allAgents.find((row) => row.id === entry?.id && row.agent.session_id === entry.native);
  const shownProject = (id: string) => !hiddenProjects.has(id);
  const centerOn = useCallback((ids: readonly string[]) => { if (ids.length) void fitView({ nodes: ids.map((id) => ({ id })), padding: 0.2, maxZoom: 1, duration: 0 }); }, [fitView]);
  const centerProject = useCallback((id: string) => centerOn(model.visible.filter((row) => row.projectId === id).map((row) => row.id)), [centerOn, model.visible]);
  const restoreCamera = useCallback(() => {
    const canvas = host.current?.querySelector(".og-canvas")?.getBoundingClientRect();
    const bounds = flow.getNodesBounds(flow.getNodes());
    const zoom = canvas && bounds.width > 0 && bounds.height > 0 ? Math.min(canvas.width / bounds.width, canvas.height / bounds.height) : 1;
    if (zoom >= READABLE_ZOOM || !layout.size) { void fitView({ padding: 0.08, duration: 0, maxZoom: 1 }); return; }
    // A map too large to read whole starts at the current session, or the operator.
    const node = current ? flow.getInternalNode(current.id) : undefined;
    const point = node ? { x: node.internals.positionAbsolute.x + (node.measured.width ?? 0) / 2, y: node.internals.positionAbsolute.y + (node.measured.height ?? 0) / 2 } : { x: 0, y: 0 };
    void flow.setCenter(point.x, point.y, { zoom: 0.8, duration: 0 });
  }, [current, layout.size, fitView, flow]);
  useEffect(() => {
    if (!nodesInitialized || layout.phase !== "ready" || cameraInitialized.current) return;
    cameraInitialized.current = true;
    restoreCamera();
  }, [nodesInitialized, layout.phase, restoreCamera]);
  useEffect(() => {
    const admitted = new Set([...model.projects.map((group) => group.id), ...model.allAgents.map((row) => row.id)]);
    setOffsets((previous) => [...previous.keys()].some((id) => !admitted.has(id)) ? new Map([...previous].filter(([id]) => admitted.has(id))) : previous);
  }, [model.structure, model.projects, model.allAgents]);
  const rememberOpener = useCallback((target?: HTMLElement) => {
    const active = target ?? document.activeElement;
    if (active instanceof HTMLElement && active !== document.body && !active.closest(".og-details")) opener.current = active;
  }, []);
  const highlight = useCallback((id: string | null, keyboard: boolean) => { if (keyboard) setFocused(id); else setHovered(id); }, []);
  const fold = useCallback((id: string) => setFolded((previous) => { const next = new Set(previous); if (next.has(id)) next.delete(id); else next.add(id); return next; }), []);
  const chooseSession = useCallback((id: string, target?: HTMLElement) => {
    const row = model.allAgents.find((row) => row.id === id);
    // A session outside the graph budget has a deterministic route back to a graph.
    // Scope navigation is explicit user selection and does not move the existing map.
    if (row && !model.projects.some((group) => group.id === row.projectId)) { revealProjectSession(row); return; }
    if (row && hiddenProjects.has(row.projectId)) setHiddenProjects((previous) => new Set([...previous].filter((project) => project !== row.projectId)));
    rememberOpener(target); setHighlightedSession(id); setSelection(id);
  }, [rememberOpener, model.allAgents, model.projects, revealProjectSession, hiddenProjects, setHiddenProjects]);
  const returnFocus = useReturnFocus(selection !== null, { target: () => opener.current, within: () => host.current?.querySelector<HTMLElement>(".og-details") ?? null });
  const close = useCallback(() => {
    returnFocus(new Event("closeAutoFocus", { cancelable: true }));
    setSelection(null);
  }, [returnFocus]);
  useEscapeLayer(selection !== null, close);
  const agent = selection ? [...model.agents, ...model.visible].find((row) => row.id === selection) : undefined;
  useEffect(() => { if (selection && (!agent || hiddenProjects.has(agent.projectId))) close(); }, [selection, agent, hiddenProjects, close]);
  useEffect(() => {
    const row = model.allAgents.find((row) => row.id === highlightedSession);
    if (highlightedSession && (!model.agents.some((row) => row.id === highlightedSession) || highlightedSession === entry?.id && !current || row && hiddenProjects.has(row.projectId))) setHighlightedSession(null);
  }, [highlightedSession, model.agents, model.allAgents, entry?.id, current, hiddenProjects]);
  const toggleProject = useCallback((id: string) => setHiddenProjects((previous) => { const next = new Set(previous); if (next.has(id)) next.delete(id); else next.add(id); return next; }), [setHiddenProjects]);
  const openRecord = onOpenRecord ?? actions.openSessionRecord;
  const activeSession = selection ?? highlightedSession;
  const placed = layout.placement;
  const size = layout.size;
  // React Flow requires parents before their relative-positioned children.
  const nodes: Node[] = [];
  const orbs = new Map<string, Orb>();
  const teamOf = (id: string) => placed?.nodes.get(id)?.team ?? placed?.nodes.get(model.projects.find((group) => group.revealed?.id === id)?.revealSlot ?? "")?.team ?? id;
  const chosenTeam = selection ? teamOf(selection) : null;
  const lineageTeam = selection ? model.teams.get(selection) ?? selection : null;
  const counterparts = new Set(model.communication.flatMap((pair) => pair.source === selection ? [pair.target] : pair.target === selection ? [pair.source] : []));
  const pointed = hovered ?? focused;
  const pointedRelated = new Set(pointed ? [pointed, ...model.communication.flatMap((pair) => pair.source === pointed ? [pair.target] : pair.target === pointed ? [pair.source] : [])] : []);
  if (placed && size) {
    nodes.push({ id: OPERATOR, type: "operator", position: placed.operator, width: size.operator, height: size.operator, measured: { width: size.operator, height: size.operator }, data: { label: t("overviewGraph.operator") }, draggable: false, connectable: false, focusable: false, selectable: false, zIndex: 2 } satisfies OperatorNode);
    for (const group of model.projects) {
      const box = placed.groups.get(group.id);
      if (!box || !shownProject(group.id)) continue;
      const offset = offsets.get(group.id);
      nodes.push({ id: group.id, type: "project", position: { x: box.x + (offset?.x ?? 0), y: box.y + (offset?.y ?? 0) }, width: box.width, height: box.height, measured: { width: box.width, height: box.height }, style: { width: box.width, height: box.height }, data: { group, current: current?.projectId === group.id, highlighted: Boolean(activeSession && model.allAgents.find((row) => row.id === activeSession)?.projectId === group.id) }, dragHandle: ".og-project-drag", draggable: true, connectable: false, focusable: false, selectable: false, zIndex: 1 } satisfies ProjectNode);
    }
    const sessions: SessionNode[] = [];
    for (const row of model.visible) {
      const group = model.projects.find((group) => group.id === row.projectId);
      const box = placed.groups.get(row.projectId);
      const point = placed.nodes.get(row.id) ?? (group?.revealSlot ? placed.nodes.get(group.revealSlot) : undefined);
      if (!point || !box || !shownProject(row.projectId)) continue;
      const shape = point.ring ? size.root : size.child;
      const position = { x: point.x + (offsets.get(row.id)?.x ?? 0), y: point.y + (offsets.get(row.id)?.y ?? 0) };
      const parent = { x: box.x + (offsets.get(row.projectId)?.x ?? 0), y: box.y + (offsets.get(row.projectId)?.y ?? 0) };
      orbs.set(row.id, { x: parent.x + position.x + shape.width / 2, y: parent.y + position.y + shape.orb / 2, r: shape.orb / 2 });
      const related = selection === row.id || teamOf(row.id) === chosenTeam || model.teams.get(row.id) === lineageTeam || counterparts.has(row.id);
      // The layout owns these fixed dimensions. Retaining them also preserves React Flow's
      // measured handle bounds when hover/focus replaces a controlled node object.
      const measured = { width: shape.width, height: shape.height };
      sessions.push({ id: row.id, parentId: row.projectId, type: "session", position, ...measured, measured, selected: activeSession === row.id, data: { row, ring: point.ring, context: model.context.has(row.id), muted: selection !== null && !related, highlighted: pointedRelated.has(row.id), current: current?.id === row.id, quietCount: model.quietBranches.get(row.id) ?? 0, folded: model.folded.has(row.id), fold, highlight, choose: chooseSession }, dragHandle: ".og-session-drag", draggable: true, connectable: false, focusable: false, selectable: false, zIndex: 2 });
    }
    // A team outline follows its members wherever they are dragged.
    const teams = new Map<string, Orb[]>();
    for (const node of sessions) { const orb = orbs.get(node.id)!; const team = teamOf(node.id); teams.set(team, [...(teams.get(team) ?? []), orb]); }
    for (const [team, members] of teams) {
      if (members.length < 2) continue;
      const pad = Math.max(...members.map((orb) => orb.r)) + size.gap;
      const left = Math.min(...members.map((orb) => orb.x)) - pad;
      const top = Math.min(...members.map((orb) => orb.y)) - pad;
      const width = Math.max(...members.map((orb) => orb.x)) + pad - left;
      const height = Math.max(...members.map((orb) => orb.y)) + pad - top;
      nodes.push({ id: `team:${team}`, type: "team", position: { x: left, y: top }, width, height, measured: { width, height }, data: { path: outline(members, pad, left, top), width, height, selected: team === chosenTeam }, draggable: false, connectable: false, focusable: false, selectable: false, zIndex: 0 } satisfies TeamNode);
    }
    nodes.push(...sessions);
    orbs.set(OPERATOR, { x: placed.operator.x + size.operator / 2, y: placed.operator.y + size.operator / 2, r: size.operator / 2 });
  }
  const arrow = (color: string) => ({ type: MarkerType.ArrowClosed, color, width: 18, height: 18, orient: "auto-start-reverse" });
  const clearance = size ? size.gap / 3 : 0;
  const edges: Edge[] = [];
  // Delegation lines appear only for the chosen team; the dashed outline says the rest.
  if (chosenTeam) for (const row of model.visible) {
    const parent = model.layoutParents.get(row.id);
    const from = parent ? orbs.get(parent) : undefined;
    const to = orbs.get(row.id);
    const data = from && to && teamOf(row.id) === chosenTeam ? between(from, to, clearance) : null;
    if (data) edges.push({ id: `lineage:${parent}:${row.id}`, source: parent!, target: row.id, sourceHandle: "source", targetHandle: "target", type: "line", className: "og-lineage", data, focusable: false, selectable: false, zIndex: 0 } satisfies LineEdge);
  }
  // One line per pair: across teams always, inside a team only while that team is chosen.
  for (const pair of model.communication) {
    const from = orbs.get(pair.source);
    const to = orbs.get(pair.target);
    const across = model.teams.get(pair.source) !== model.teams.get(pair.target);
    if (!from || !to || !across && model.teams.get(pair.source) !== lineageTeam) continue;
    const data = between(from, to, clearance);
    if (!data) continue;
    const touches = (id: string | null) => id !== null && (pair.source === id || pair.target === id);
    const active = touches(selection) || touches(pointed) || !across;
    const muted = selection !== null && !active && model.teams.get(pair.source) !== lineageTeam && model.teams.get(pair.target) !== lineageTeam;
    const marker = arrow("var(--subtle-foreground)");
    edges.push({ id: pair.id, source: pair.source, target: pair.target, sourceHandle: "source", targetHandle: "target", type: "line", className: `og-communication${pair.letters === 0 ? " og-direct-input" : ""}${active ? " og-emphasized" : muted ? " og-muted" : ""}`, data, markerEnd: marker, markerStart: pair.directions.length > 1 ? marker : undefined, focusable: false, selectable: false, zIndex: 0 } satisfies LineEdge);
  }
  // Only a root whose turn it is points at the operator.
  const operatorOrb = orbs.get(OPERATOR);
  for (const row of model.visible) {
    const from = orbs.get(row.id);
    if (!from || !operatorOrb || !model.roots.has(row.id) || graphActivity(row.agent) !== "turn") continue;
    const data = between(from, operatorOrb, clearance);
    const muted = selection !== null && model.teams.get(row.id) !== lineageTeam;
    if (data) edges.push({ id: `turn:${row.id}`, source: row.id, target: OPERATOR, sourceHandle: "source", targetHandle: "target", type: "line", className: `og-turn${muted ? " og-muted" : ""}`, data, markerEnd: arrow("var(--warning)"), focusable: false, selectable: false, zIndex: 0 } satisfies LineEdge);
  }
  const positionOf = (id: string) => orbs.get(id) ?? { x: 0, y: 0 };
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
  const showList = mode === "list" || layout.phase === "failed" || !placed;
  return <div className="overview-graph" ref={host} data-overview-graph="true" data-graph-phase={layout.phase}>
    <div className="og-toolbar">
      <div><h2>{t("overviewGraph.title")}</h2></div>
      <div className="og-toolbar-actions">{projectId ? <Button variant="ghost" size="sm" onClick={() => useUiStore.getState().setOverview({ projectId: null })}>{t("overviewGraph.allProjects")}</Button> : null}<div role="group" aria-label={t("overviewGraph.title")}><Button variant={mode === "graph" ? "secondary" : "ghost"} size="sm" aria-pressed={mode === "graph"} onClick={() => setMode("graph")}>{t("overviewGraph.graph")}</Button><Button variant={mode === "list" ? "secondary" : "ghost"} size="sm" aria-pressed={mode === "list"} onClick={() => setMode("list")}>{t("overviewGraph.list")}</Button></div></div>
    </div>
    <div className="og-filters">
      <div role="group" aria-label={t("overviewGraph.filter")}>{(["all", "turn", "working", "results"] as const).map((value) => <Button key={value} variant={focus === value ? "secondary" : "ghost"} size="sm" aria-pressed={focus === value} data-focus={value} onClick={() => setFocus(value)}>{t(`overviewGraph.focus.${value}`)} <span>{model.counts[value]}</span></Button>)}</div>
      <label><input type="checkbox" checked={includeResting} onChange={(event) => setIncludeResting(event.target.checked)} />{t("overviewGraph.includeResting")}</label>
    </div>
    {model.limited ? <p className="og-notice" role="status">{t("overviewGraph.limit", { count: LAYOUT_NODE_LIMIT })}</p> : null}
    {model.agents.length > 0 && layout.phase === "loading" && mode === "graph" ? <p className="og-notice" role="status">{t("overviewGraph.loading")}</p> : null}
    {model.agents.length > 0 && layout.phase === "failed" && mode === "graph" ? <div className="og-notice" role="status"><span>{t("overviewGraph.failed")}</span><Button variant="outline" size="sm" onClick={() => setRetry((value) => value + 1)}>{t("overviewGraph.retry")}</Button></div> : null}
    {model.agents.length === 0 ? <div className="og-empty"><p>{t(model.allAgents.length > 0 ? "overviewGraph.filteredEmpty" : "overviewGraph.empty")}</p>{model.allAgents.length > 0 ? <Button variant="outline" size="sm" onClick={() => { setFocus("all"); setIncludeResting(true); }}>{t("overviewGraph.resetFilters")}</Button> : null}<Button variant="outline" size="sm" onClick={() => actions.openStartPanel(projectId ?? undefined)}>{t("overviewGraph.newAgent")}</Button></div> : null}
    {model.agents.length > 0 ? <div className="og-stage">
      <div className="og-canvas">{!showList ? <ReactFlow zIndexMode="manual" colorMode={theme} nodes={nodes} edges={edges} nodeTypes={NODE_TYPES} edgeTypes={EDGE_TYPES} nodesDraggable onNodesChange={moveNodes} nodesConnectable={false} nodesFocusable={false} edgesFocusable={false} elementsSelectable={false} deleteKeyCode={null} selectionKeyCode={null} multiSelectionKeyCode={null} panOnDrag panOnScroll zoomOnScroll defaultViewport={{ x: 24, y: 24, zoom: 1 }} minZoom={0.05} maxZoom={2} onPaneClick={() => { close(); setHighlightedSession(null); }} ariaLabelConfig={{ "node.a11yDescription.default": t("overviewGraph.nodeHelp"), "node.a11yDescription.keyboardDisabled": t("overviewGraph.nodeHelp") }}>
        <div className="og-zoom nopan nodrag"><Hint label={t("overviewGraph.zoomOut")}><Button variant="ghost" size="icon-sm" onClick={() => void zoomOut({ duration: 0 })}><MinusIcon /></Button></Hint><Hint label={t("overviewGraph.zoomIn")}><Button variant="ghost" size="icon-sm" onClick={() => void zoomIn({ duration: 0 })}><PlusIcon /></Button></Hint><Hint label={t("overviewGraph.fit")}><Button variant="ghost" size="icon-sm" onClick={() => void fitView({ padding: 0.08, duration: 0, maxZoom: 1 })}><MaximizeIcon /></Button></Hint><Hint label={t("overviewGraph.currentPane")}><Button variant="ghost" size="icon-sm" disabled={!current} onClick={() => { if (current) { setHighlightedSession(current.id); if (orbs.has(current.id)) { const point = positionOf(current.id); void flow.setCenter(point.x, point.y, { zoom: 1, duration: 0 }); } else centerProject(current.projectId); } }}><LocateFixedIcon /></Button></Hint><Hint label={t("overviewGraph.resetLayout")}><Button variant="ghost" size="icon-sm" onClick={() => { setOffsets(new Map()); cameraInitialized.current = false; }}><RotateCcwIcon /></Button></Hint></div>
      </ReactFlow> : <SessionList key={`${focus}:${includeResting}`} rows={model.agents} selected={activeSession} choose={chooseSession} />}</div>
      {agent ? <SessionPreview row={agent} model={model} actions={actions} openRecord={openRecord} close={close} choose={chooseSession} /> : <ProjectColumn model={model} query={projectQuery} setQuery={setProjectQuery} hidden={hiddenProjects} toggle={toggleProject} locate={(summary) => summary.admitted ? centerProject(summary.id) : useUiStore.getState().setOverview({ projectId: summary.project.workspace.id })} choose={chooseSession} actions={actions} />}
    </div> : null}
  </div>;
}

/** Who to answer for a root whose turn it is: the root itself when it asks, else the first descendant that does. */
function askingPane(row: GraphAgent, model: OverviewGraphModel): string {
  if (row.agent.demand !== "none") return row.agent.pane_id;
  return overviewGraphTeam(row, model).find((member) => member.row.agent.mark === "turn" && member.row.agent.demand !== "none")?.row.agent.pane_id ?? row.agent.pane_id;
}

/** With nothing chosen, the side column lists the projects with their states, then what waits on the operator. */
function ProjectColumn({ model, query, setQuery, hidden, toggle, locate, choose, actions }: { model: OverviewGraphModel; query: string; setQuery: (value: string) => void; hidden: ReadonlySet<string>; toggle: (id: string) => void; locate: (summary: ProjectSummary) => void; choose: (id: string, target: HTMLElement) => void; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const needle = query.trim().toLocaleLowerCase();
  const waiting = model.visible.filter((row) => model.roots.has(row.id) && graphActivity(row.agent) === "turn" && !hidden.has(row.projectId));
  return <aside className="og-navigator" aria-label={t("overviewGraph.projects")} data-graph-projects="true">
    <Input aria-label={t("overviewGraph.findProject")} placeholder={t("overviewGraph.findProject")} value={query} onChange={(event) => setQuery(event.target.value)} />
    <div className="og-project-list">{model.summaries.filter((summary) => summary.project.workspace.label.toLocaleLowerCase().includes(needle)).sort((a, b) => a.project.workspace.label.localeCompare(b.project.workspace.label, undefined, { numeric: true }) || a.id.localeCompare(b.id)).map((summary) => {
      const label = summary.project.workspace.label;
      const off = hidden.has(summary.id);
      return <div key={summary.id} data-graph-project-row={summary.project.workspace.id} data-hidden={off}>
        <button type="button" aria-label={t("overviewGraph.locateProject", { project: label })} disabled={off} onClick={() => locate(summary)}><strong>{label}</strong><GraphStateCounts summary={summary} /><span className="og-count">{summary.sessions}</span></button>
        <Button variant="ghost" size="sm" aria-pressed={off} aria-label={t(off ? "overviewGraph.revealProject" : "overviewGraph.hideProject", { project: label })} disabled={!summary.admitted} onClick={() => toggle(summary.id)}>{t(off ? "overviewGraph.reveal" : "overviewGraph.hide")}</Button>
      </div>;
    })}</div>
    {waiting.length > 0 ? <section className="og-waiting" aria-label={t("overviewGraph.waiting")}>
      <h3>{t("overviewGraph.waiting")} <span>{waiting.length}</span></h3>
      {waiting.map((row) => <div key={row.id} className="og-ask" data-graph-ask={row.agent.pane_id}>
        <button type="button" className="og-ask-who" aria-label={rowAccessibleName(t, row.agent, row.device)} onClick={(event) => choose(row.id, event.currentTarget)}><AgentMark kind={row.agent.agent_kind} /><span><AgentRecordTitle agent={row.agent} /></span><small>{row.project.label}</small></button>
        <p>{row.agent.state.ask ? askWhat(t, row.agent.state.ask.verb, row.agent.state.ask.what) : <GraphLine agent={row.agent} />}</p>
        <Button variant="secondary" size="sm" onClick={() => actions.openAgent(askingPane(row, model))}>{t("overviewGraph.answer")}</Button>
      </div>)}
    </section> : null}
  </aside>;
}

/** A pair's counts as the chosen session sees them: what it sent and what it received. */
function exchange(pair: Communication, self: string) {
  const sent = pair.directions.filter((row) => row.from === self).reduce((total, row) => ({ letters: total.letters + row.letters, direct: total.direct + row.direct }), { letters: 0, direct: 0 });
  return { other: pair.source === self ? pair.target : pair.source, sent: sent.letters + sent.direct, received: pair.letters + pair.direct - sent.letters - sent.direct, letters: pair.letters, direct: pair.direct };
}

function SessionPreview({ row, model, actions, openRecord, close, choose }: { row: GraphAgent; model: OverviewGraphModel; actions: Actions; openRecord: (session: WorkSession) => void; close: () => void; choose: (id: string, target: HTMLElement) => void }) {
  const { t } = useInterfaceTranslation();
  const session = sessionOfAgentOn(row.agent, row.deviceId);
  const team = overviewGraphTeam(row, model);
  const rows = new Map(model.allAgents.map((one) => [one.id, one]));
  const talks = model.communication.filter((pair) => pair.source === row.id || pair.target === row.id).map((pair) => exchange(pair, row.id)).filter((talk) => rows.has(talk.other));
  const ask = row.agent.state.ask;
  return <aside className="og-details overview-narrow:absolute overview-narrow:inset-y-0 overview-narrow:right-0 overview-narrow:shadow-lg" aria-label={t("overviewGraph.preview")} data-graph-preview="true">
    <div className="og-details-head"><strong><AgentMark kind={row.agent.agent_kind} /><AgentRecordTitle agent={row.agent} />{starred(row.agent) ? <RevisitStar className="ml-xxs" /> : null}</strong><Hint label={t("overviewGraph.close")}><Button variant="ghost" size="icon-sm" onClick={close}><XIcon /></Button></Hint></div>
    <p className="og-detail-status"><StatusMark mark={row.agent.mark} />{markWords(t, row.agent)}</p>
    <p className="og-context">{row.project.label} · {row.checkout.branch ?? row.checkout.label}{row.device ? ` · ${row.device}` : ""}</p>
    <p className="og-context">{agentProductName(row.agent.agent_kind)}{row.launchModel ? ` · ${t("overviewGraph.launchModel", { model: row.launchModel })}` : ""}</p>
    {!ask && hasGraphLine(row.agent) ? <div className="og-now"><b>{t("overviewGraph.now")}</b><GraphLine agent={row.agent} /></div> : null}
    {ask ? <div className="og-ask"><small>{t("overviewGraph.asks")}</small><p>{askWhat(t, ask.verb, ask.what)}</p></div> : null}
    <div className="og-details-actions"><Button variant="outline" size="sm" onClick={() => actions.openAgent(row.agent.pane_id)}>{t("overviewGraph.openSession")}</Button>{session ? <Button variant="secondary" size="sm" onClick={() => openRecord(session)}>{t("overviewGraph.openRecord")}</Button> : null}</div>
    {team.length > 0 ? <section className="og-team-list" aria-label={t("overviewGraph.team", { count: team.length })}><h3>{t("overviewGraph.team", { count: team.length })}</h3>{team.map(({ row: member, depth }) => <button key={member.id} type="button" style={{ paddingInlineStart: `calc(var(--spacing-md) * ${depth - 1})` }} aria-label={rowAccessibleName(t, member.agent, member.device)} onClick={(event) => choose(member.id, event.currentTarget)}><StatusMark mark={member.agent.mark} /><span><AgentRecordTitle agent={member.agent} /></span><small>{markWords(t, member.agent)}</small></button>)}</section> : null}
    {talks.length > 0 ? <section className="og-talks" aria-label={t("overviewGraph.counterparts")}><h3>{t("overviewGraph.counterparts")}</h3>{talks.map((talk) => { const other = rows.get(talk.other)!; return <div key={talk.other} title={t("overviewGraph.exchange", { letters: talk.letters, direct: talk.direct })}><AgentMark kind={other.agent.agent_kind} /><span><AgentRecordTitle agent={other.agent} /></span><small>{[talk.sent ? t("overviewGraph.sent", { count: talk.sent }) : null, talk.received ? t("overviewGraph.received", { count: talk.received }) : null].filter(Boolean).join(" · ")}</small></div>; })}</section> : null}
  </aside>;
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
    {filtered.slice(current * PAGE_SIZE, (current + 1) * PAGE_SIZE).map((row) => <button key={row.id} type="button" aria-label={[rowAccessibleName(t, row.agent, row.device), starred(row.agent) ? t("taskLens.revisit.starred") : null, row.project.label, row.checkout.label].filter(Boolean).join(", ")} aria-pressed={selected === row.id} onClick={(event) => choose(row.id, event.currentTarget)}><AgentMark kind={row.agent.agent_kind} /><span className="og-list-name"><strong><AgentRecordTitle agent={row.agent} />{starred(row.agent) ? <RevisitStar className="ml-xxs" /> : null}</strong><small>{row.project.label} · {row.checkout.label}</small></span><StatusMark mark={row.agent.mark} /><span>{markWords(t, row.agent)}</span></button>)}
    {pages > 1 ? <div className="og-pagination"><Button variant="ghost" size="sm" disabled={current === 0} onClick={() => setPage(current - 1)}>{t("overviewGraph.previous")}</Button><span>{t("overviewGraph.page", { page: current + 1, pages })}</span><Button variant="ghost" size="sm" disabled={current + 1 === pages} onClick={() => setPage(current + 1)}>{t("overviewGraph.next")}</Button></div> : null}
  </div>;
}
