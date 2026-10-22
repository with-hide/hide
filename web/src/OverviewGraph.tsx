import { MarkerType, ReactFlow, ReactFlowProvider, useReactFlow, useNodesInitialized, type Edge, type Node, type NodeTypes, type EdgeTypes } from "@xyflow/react";
import { EyeIcon, EyeOffIcon, MaximizeIcon, MinusIcon, PlusIcon, XIcon, LocateFixedIcon, RotateCcwIcon } from "lucide-react";
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
import { between, cloud, communicationLines, lineageLines, projectAccent, teamOutline, turnLines, type Area, type Orb } from "./overviewGraphLines";
import { Hint } from "./components/ui/tooltip";
import { StatusMark } from "./components/status-mark";
import { useInterfaceTranslation } from "./i18n/client";
import { frontDeviceId } from "./devices";
import { Input } from "./components/ui/input";
import { LAYOUT_NODE_LIMIT } from "./dependencyLayout";
import { GRAPH_CHOICES_DEFAULTS, graphActivity, graphProjectIdOf, graphProjectKey, graphSessionId, graphSessionIdOf, graphSessionKey, overviewGraphModel, overviewGraphTeam, sameGraphScope, type Communication, type GraphAgent, type GraphFocus, type OverviewGraphModel, type ProjectSummary } from "./overviewGraphModel";
import { projectDropPlaces, useGraphTheme, useOverviewGraphLayout, type RememberedPlaces } from "./overviewGraphLayout";
import { ClusterGraphNode, GlowGraphNode, GraphLine, GraphStateCounts, hasGraphLine, LineGraphEdge, OperatorGraphNode, ProjectGraphNode, SessionGraphNode, TeamGraphNode, type ClusterNode, type GlowNode, type LineEdge, type OperatorNode, type ProjectNode, type SessionNode, type TeamNode } from "./overviewGraphNodes";
import type { BoardProject } from "./projectBoard";
import { sessionOfAgentOn } from "./sessionWork";
import type { GraphScope, OverviewGraphChoices, WorkSession } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import "@xyflow/react/dist/base.css";
import "./overviewGraph.css";

type Props = { projects: BoardProject[]; deviceId: string; actions: Actions; onOpenRecord?: (session: WorkSession) => void };
const NODE_TYPES: NodeTypes = { operator: OperatorGraphNode, project: ProjectGraphNode, glow: GlowGraphNode, cluster: ClusterGraphNode, team: TeamGraphNode, session: SessionGraphNode };
const EDGE_TYPES: EdgeTypes = { line: LineGraphEdge };
const PAGE_SIZE = 50;
const OPERATOR = "operator";
/** Below this zoom the whole map is unreadable, so the first camera frames the current session instead. */
const READABLE_ZOOM = 0.5;

/** Production Overview owns the graph; the Factory continues to own its separate worker. */
export function OverviewGraph(props: Props) {
  const projectId = useUiStore((state) => state.overview.projectId);
  const [scopedSelection, setScopedSelection] = useState<{ deviceId: string; projectId: string; sessionId: string } | null>(null);
  const revealProjectSession = (row: GraphAgent) => {
    setScopedSelection({ deviceId: props.deviceId, projectId: row.project.id, sessionId: row.id });
    useUiStore.getState().setOverview({ projectId: row.project.id });
  };
  const initialSelection = scopedSelection?.deviceId === props.deviceId && scopedSelection.projectId === projectId ? scopedSelection.sessionId : null;
  useEffect(() => { if (initialSelection) setScopedSelection(null); }, [initialSelection]);
  return <ReactFlowProvider key={`${props.deviceId}:${projectId ?? "all"}`}><WorkGraph {...props} projectId={projectId} initialSelection={initialSelection} revealProjectSession={revealProjectSession} /></ReactFlowProvider>;
}

type GraphProps = Props & { projectId: string | null; initialSelection: string | null; revealProjectSession: (row: GraphAgent) => void };

/** The filter, the hidden and folded projects, the hand team folds and the dropped places are the core's (PRD overview-graph-remembered D-05): every change is one event and the view draws what the snapshot says. */
function WorkGraph({ projects, deviceId, actions, onOpenRecord, projectId, initialSelection, revealProjectSession }: GraphProps) {
  const { t } = useInterfaceTranslation();
  const rest = useShellStore((state) => state.rest);
  const choices: OverviewGraphChoices = rest?.ui_state?.overview_graph ?? GRAPH_CHOICES_DEFAULTS;
  const focus: GraphFocus = choices.focus;
  const includeResting = choices.include_resting;
  const hiddenProjects = useMemo<ReadonlySet<string>>(() => new Set(choices.hidden_projects.map(graphProjectIdOf)), [choices.hidden_projects]);
  const foldedProjects = useMemo<ReadonlySet<string>>(() => new Set(choices.folded_projects.map(graphProjectIdOf)), [choices.folded_projects]);
  // A team's fold the operator chose; a team nobody chose for opens folded when every descendant sleeps.
  const teamFolds = useMemo<ReadonlyMap<string, boolean>>(() => new Map(choices.team_folds.map((fold) => [graphSessionIdOf(fold.session), fold.folded])), [choices.team_folds]);
  const scopedDevice = projectId ? projects.find((project) => project.workspace.id === projectId)?.workspace.device_id ?? deviceId : deviceId;
  const scope = useMemo<GraphScope>(() => projectId ? { kind: "project", device_id: scopedDevice, workspace_id: projectId } : { kind: "all", device_id: deviceId }, [projectId, scopedDevice, deviceId]);
  const remembered = useMemo<RememberedPlaces>(() => ({
    anchors: new Map(choices.project_places.filter((place) => sameGraphScope(place.scope, scope)).map((place) => [graphProjectIdOf(place.key), { x: place.x, y: place.y }])),
    pins: new Map(choices.session_places.filter((place) => sameGraphScope(place.scope, scope)).map((place) => [graphSessionIdOf(place.key), { x: place.x, y: place.y }])),
  }), [choices.project_places, choices.session_places, scope]);
  const setFocus = (value: GraphFocus) => actions.changeOverviewGraph({ kind: "focus", focus: value });
  const setIncludeResting = (value: boolean) => actions.changeOverviewGraph({ kind: "focus", include_resting: value });
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
  const [mode, setMode] = useState<"graph" | "list">("graph");
  const [selection, setSelection] = useState<string | null>(initialSelection);
  const [hovered, setHovered] = useState<string | null>(null);
  const [focused, setFocused] = useState<string | null>(null);
  // Each drag step redraws from the live map.
  const [, setMoved] = useState(0);
  const [pointedProject, setPointedProject] = useState<string | null>(null);
  const projectDrag = useRef<{ project: string; from: { x: number; y: number }; anchor: { x: number; y: number } } | null>(null);
  const [retry, setRetry] = useState(0);
  const host = useRef<HTMLDivElement>(null);
  const opener = useRef<HTMLElement | null>(null);
  const theme = useGraphTheme();
  const model = useMemo(() => overviewGraphModel(projects, rest?.overview_graph, projectId, { focus, includeResting, pinned: entry ? [entry.id] : [], selected: highlightedSession }), [projects, rest?.overview_graph, projectId, focus, includeResting, entry, highlightedSession]);
  const layout = useOverviewGraphLayout(model, host, retry, remembered);
  const flow = useReactFlow();
  const { fitView, zoomIn, zoomOut } = flow;
  const nodesInitialized = useNodesInitialized();
  const cameraInitialized = useRef(false);
  const current = model.allAgents.find((row) => row.id === entry?.id && row.agent.session_id === entry.native);
  const shownProject = (id: string) => !hiddenProjects.has(id);
  const centerOn = useCallback((ids: readonly string[]) => { if (ids.length) void fitView({ nodes: ids.map((id) => ({ id })), padding: 0.2, maxZoom: 1, duration: 0 }); }, [fitView]);
  const centerProject = useCallback((id: string) => centerOn(model.visible.filter((row) => row.projectId === id).map((row) => row.id)), [centerOn, model.visible]);
  // The whole map, with the operator at the center of the view rather than the middle of the map's bounds.
  const operatorZoom = useCallback(() => {
    const canvas = host.current?.querySelector(".og-canvas")?.getBoundingClientRect();
    // A glow reaches past its sessions and is not part of what must fit.
    const bounds = flow.getNodesBounds(flow.getNodes().filter((node) => node.type !== "glow"));
    const across = Math.max(-bounds.x, bounds.x + bounds.width);
    const down = Math.max(-bounds.y, bounds.y + bounds.height);
    return canvas && across > 0 && down > 0 ? Math.min(1, Math.min(canvas.width / (2 * across), canvas.height / (2 * down)) / 1.16) : 1;
  }, [flow]);
  const fitOperator = useCallback(() => void flow.setCenter(0, 0, { zoom: operatorZoom(), duration: 0 }), [flow, operatorZoom]);
  const restoreCamera = useCallback(() => {
    const zoom = operatorZoom();
    if (zoom >= READABLE_ZOOM || !layout.size) { fitOperator(); return; }
    // A map too large to read whole starts at the current session, or the operator.
    const node = current ? flow.getInternalNode(current.id) : undefined;
    const point = node ? { x: node.internals.positionAbsolute.x + (node.measured.width ?? 0) / 2, y: node.internals.positionAbsolute.y + (node.measured.height ?? 0) / 2 } : { x: 0, y: 0 };
    void flow.setCenter(point.x, point.y, { zoom: 0.8, duration: 0 });
  }, [current, layout.size, fitOperator, operatorZoom, flow]);
  useEffect(() => {
    if (!nodesInitialized || layout.phase !== "ready" || cameraInitialized.current) return;
    cameraInitialized.current = true;
    restoreCamera();
  }, [nodesInitialized, layout.phase, restoreCamera]);
  const rememberOpener = useCallback((target?: HTMLElement) => {
    const active = target ?? document.activeElement;
    if (active instanceof HTMLElement && active !== document.body && !active.closest(".og-details")) opener.current = active;
  }, []);
  const highlight = useCallback((id: string | null, keyboard: boolean) => { if (keyboard) setFocused(id); else setHovered(id); }, []);
  const teamFolded = useCallback((root: string) => teamFolds.get(root) ?? model.quietBranches.has(root), [teamFolds, model.quietBranches]);
  const sessionKeyOf = useCallback((id: string) => { const row = model.allAgents.find((row) => row.id === id); return row ? graphSessionKey(row) : undefined; }, [model.allAgents]);
  const projectKeyOf = useCallback((id: string) => { const summary = model.summaries.find((summary) => summary.id === id); return summary ? graphProjectKey(summary.project.workspace) : undefined; }, [model.summaries]);
  const toggleTeam = useCallback((root: string) => { const session = sessionKeyOf(root); if (session) actions.changeOverviewGraph({ kind: "team_folded", session, folded: !teamFolded(root) }); }, [actions, sessionKeyOf, teamFolded]);
  const toggleFoldProject = useCallback((id: string) => { const project = projectKeyOf(id); if (project) actions.changeOverviewGraph({ kind: "project_folded", project, folded: !foldedProjects.has(id) }); }, [actions, projectKeyOf, foldedProjects]);
  const foldAll = () => actions.changeOverviewGraph({ kind: "fold_all", projects: model.projects.map((group) => graphProjectKey(group.project.workspace)) });
  const revealing = useRef<string | null>(null);
  useEffect(() => { if (revealing.current && !hiddenProjects.has(revealing.current)) revealing.current = null; }, [hiddenProjects]);
  const hiddenNow = useCallback((project: string) => hiddenProjects.has(project) && revealing.current !== project, [hiddenProjects]);
  const chooseSession = useCallback((id: string, target?: HTMLElement) => {
    const row = model.allAgents.find((row) => row.id === id);
    // A session outside the graph budget has a deterministic route back to a graph.
    // Scope navigation is explicit user selection and does not move the existing map.
    if (row && !model.projects.some((group) => group.id === row.projectId)) { revealProjectSession(row); return; }
    if (row && hiddenProjects.has(row.projectId)) {
      // The core shows the project again on its next answer; until then its chosen session stays chosen.
      revealing.current = row.projectId;
      actions.changeOverviewGraph({ kind: "project_hidden", project: graphProjectKey(row.project), hidden: false });
    }
    rememberOpener(target); setHighlightedSession(id); setSelection(id);
  }, [rememberOpener, model.allAgents, model.projects, revealProjectSession, hiddenProjects, actions]);
  const returnFocus = useReturnFocus(selection !== null, { target: () => opener.current, within: () => host.current?.querySelector<HTMLElement>(".og-details") ?? null });
  const close = useCallback(() => {
    returnFocus(new Event("closeAutoFocus", { cancelable: true }));
    setSelection(null);
  }, [returnFocus]);
  useEscapeLayer(selection !== null, close);
  // A resting team member is chosen from the team list even while it is not drawn.
  const agent = selection ? model.allAgents.find((row) => row.id === selection) : undefined;
  useEffect(() => { if (selection && (!agent || hiddenNow(agent.projectId))) close(); }, [selection, agent, hiddenNow, close]);
  useEffect(() => {
    const row = model.allAgents.find((row) => row.id === highlightedSession);
    if (highlightedSession && (!row || highlightedSession === entry?.id && !current || row && hiddenNow(row.projectId))) setHighlightedSession(null);
  }, [highlightedSession, model.agents, model.allAgents, entry?.id, current, hiddenNow]);
  const toggleProject = useCallback((id: string) => { const project = projectKeyOf(id); if (project) actions.changeOverviewGraph({ kind: "project_hidden", project, hidden: !hiddenProjects.has(id) }); }, [actions, projectKeyOf, hiddenProjects]);
  const openRecord = onOpenRecord ?? actions.openSessionRecord;
  const activeSession = selection ?? highlightedSession;
  const size = layout.size;
  const map = layout.map;
  // Positions come from the live map, so a drag step shows at once.
  const points = map?.positions() ?? layout.points;
  const nodes: Node[] = [];
  const orbs = new Map<string, Orb>();
  const projectsById = new Map(model.projects.map((group) => [group.id, group]));
  const agentsById = new Map(model.allAgents.map((row) => [row.id, row]));
  const pointOf = (id: string) => points.get(id) ?? points.get(projectsById.get(agentsById.get(id)?.projectId ?? "")?.revealSlot ?? "");
  const teamOf = (id: string) => pointOf(id)?.team ?? id;
  const chosenTeam = selection ? teamOf(selection) : null;
  const lineageTeam = selection ? model.teams.get(selection) ?? selection : null;
  const counterparts = new Set(model.communication.flatMap((pair) => pair.source === selection ? [pair.target] : pair.target === selection ? [pair.source] : []));
  const pointed = hovered ?? focused;
  const pointedRelated = new Set(pointed ? [pointed, ...model.communication.flatMap((pair) => pair.source === pointed ? [pair.target] : pair.target === pointed ? [pair.source] : [])] : []);
  const summariesById = new Map(model.summaries.map((summary) => [summary.id, summary]));
  // What each placed session is drawn as: itself, its folded team's root, or its folded project's circle.
  const endpoint = new Map<string, string>();
  const drawnRoots = new Set<string>();
  const clusterProject = new Map<string, string>();
  const cluster = (id: string) => `cluster:${id}`;
  const visibleByProject = new Map<string, GraphAgent[]>();
  for (const row of model.visible) visibleByProject.set(row.projectId, [...(visibleByProject.get(row.projectId) ?? []), row]);
  if (size && layout.phase === "ready") {
    nodes.push({ id: OPERATOR, type: "operator", position: { x: -size.operator / 2, y: -size.operator / 2 }, width: size.operator, height: size.operator, measured: { width: size.operator, height: size.operator }, data: { label: t("overviewGraph.operator") }, draggable: false, connectable: false, focusable: false, selectable: false, style: { pointerEvents: "none" }, zIndex: 3 } satisfies OperatorNode);
    orbs.set(OPERATOR, { x: 0, y: 0, r: size.operator / 2 });
    // A project stands out while its name or row is pointed at, or one of its sessions is chosen and its
    // details are open (PRD D-17); the current pane's highlight at entry is not a choice and lights nothing.
    const chosenProject = selection ? agentsById.get(selection)?.projectId : undefined;
    const emphasizedProject = (id: string) => pointedProject === id || chosenProject === id;
    const sessions: SessionNode[] = [];
    for (const group of model.projects) {
      if (!shownProject(group.id)) continue;
      const members = (visibleByProject.get(group.id) ?? []).filter((row) => pointOf(row.id));
      if (!members.length) continue;
      const accent = projectAccent(group.id);
      const middle = cloud(members.map((row) => pointOf(row.id)!), size.root.width / 2);
      // Folded or not, the project keeps its glow, so its circle reads as the same project (PRD D-19).
      const glow = middle.r * 2;
      nodes.push({ id: `glow:${group.id}`, type: "glow", position: { x: middle.x - middle.r, y: middle.y - middle.r }, width: glow, height: glow, measured: { width: glow, height: glow }, data: { accent, emphasized: emphasizedProject(group.id) }, draggable: false, connectable: false, focusable: false, selectable: false, style: { pointerEvents: "none" }, zIndex: 0 } satisfies GlowNode);
      if (foldedProjects.has(group.id)) {
        // A folded project keeps its place as one circle where its sessions were.
        const summary = summariesById.get(group.id);
        if (!summary) continue;
        const orb = size.root.orb * 1.5;
        const box = { width: size.label.width, height: orb + size.gap * 7 };
        nodes.push({ id: cluster(group.id), type: "cluster", position: { x: middle.x - box.width / 2, y: middle.y - orb / 2 }, ...box, measured: box, data: { summary, accent, orb, emphasized: emphasizedProject(group.id), unfold: toggleFoldProject, point: setPointedProject }, dragHandle: ".og-cluster-drag", draggable: true, connectable: false, focusable: false, selectable: false, zIndex: 2 } satisfies ClusterNode);
        orbs.set(cluster(group.id), { x: middle.x, y: middle.y, r: orb / 2 });
        for (const row of members) endpoint.set(row.id, cluster(group.id));
        clusterProject.set(cluster(group.id), group.id);
        continue;
      }
      const top = Math.min(...members.map((row) => pointOf(row.id)!.y)) - size.root.orb / 2 - size.gap * 2 - size.label.height;
      nodes.push({ id: group.id, type: "project", position: { x: middle.x - size.label.width / 2, y: top }, width: size.label.width, height: size.label.height, measured: size.label, data: { group, accent, current: current?.projectId === group.id, emphasized: emphasizedProject(group.id), fold: toggleFoldProject, point: setPointedProject }, dragHandle: ".og-project-drag", draggable: true, connectable: false, focusable: false, selectable: false, zIndex: 1 } satisfies ProjectNode);
      const drawnHere = new Set(members.map((row) => row.id));
      for (const row of members) {
        const point = pointOf(row.id)!;
        const root = point.team === row.id || row.id === group.revealed?.id;
        // A folded team's descendants are drawn as their root.
        if (!root && drawnHere.has(point.team) && teamFolded(point.team)) { endpoint.set(row.id, point.team); continue; }
        endpoint.set(row.id, row.id);
        if (root) drawnRoots.add(row.id);
        const shape = root ? size.root : size.child;
        orbs.set(row.id, { x: point.x, y: point.y, r: shape.orb / 2 });
        const descendants = root ? members.filter((member) => member.id !== row.id && pointOf(member.id)!.team === row.id) : [];
        const activity = descendants.map((member) => graphActivity(member.agent));
        const team = descendants.length ? { count: descendants.length, turn: activity.filter((value) => value === "turn").length, going: activity.filter((value) => value === "working").length, done: activity.filter((value) => value === "results").length, folded: teamFolded(row.id), toggle: () => toggleTeam(row.id) } : null;
        const related = selection === row.id || teamOf(row.id) === chosenTeam || model.teams.get(row.id) === lineageTeam || counterparts.has(row.id);
        // The layout owns these fixed dimensions. Retaining them also preserves React Flow's
        // measured handle bounds when hover/focus replaces a controlled node object.
        const measured = { width: shape.width, height: shape.height };
        sessions.push({ id: row.id, type: "session", position: { x: point.x - shape.width / 2, y: point.y - shape.orb / 2 }, ...measured, measured, selected: activeSession === row.id, data: { row, root, context: model.context.has(row.id), muted: selection !== null && !related, highlighted: pointedRelated.has(row.id), current: current?.id === row.id, team, highlight, choose: chooseSession }, dragHandle: ".og-session-drag", draggable: true, connectable: false, focusable: false, selectable: false, zIndex: 2 });
      }
    }
    // The chosen team's dashed outline goes round its circles and words.
    if (chosenTeam) {
      const members = sessions.filter((node) => teamOf(node.id) === chosenTeam).map((node): Area => ({ left: node.position.x, top: node.position.y, right: node.position.x + node.width!, bottom: node.position.y + node.height! }));
      if (members.length > 1) {
        const shape = teamOutline(members, size.gap / 2);
        nodes.push({ id: `team:${chosenTeam}`, type: "team", position: { x: shape.left, y: shape.top }, width: shape.width, height: shape.height, measured: { width: shape.width, height: shape.height }, data: { path: shape.path, width: shape.width, height: shape.height, selected: true }, draggable: false, connectable: false, focusable: false, selectable: false, style: { pointerEvents: "none" }, zIndex: 1 } satisfies TeamNode);
      }
    }
    nodes.push(...sessions);
  }
  const arrow = (color: string) => ({ type: MarkerType.ArrowClosed, color, width: 18, height: 18, orient: "auto-start-reverse" });
  const clearance = size ? size.gap / 3 : 0;
  const edges: Edge[] = [];
  const line = (id: string, from: string, to: string, className: string, extra: Partial<LineEdge> = {}) => {
    const a = orbs.get(from);
    const b = orbs.get(to);
    const data = a && b ? between(a, b, clearance) : null;
    if (data) edges.push({ id, source: from, target: to, sourceHandle: "source", targetHandle: "target", type: "line", className, data, focusable: false, selectable: false, zIndex: 0, ...extra } satisfies LineEdge);
  };
  const lineState = { selection, lineageTeam, pointed, teams: model.teams };
  // Every drawn root, and every folded project, is tied faintly to the operator; one holding the
  // operator's turn, its own or a folded member's, points at it in amber instead, muted outside the chosen team.
  const tied = new Map<string, boolean>();
  for (const [id, at] of endpoint) {
    if (at === id && !drawnRoots.has(id)) continue;
    tied.set(at, (tied.get(at) ?? false) || graphActivity(agentsById.get(id)!.agent) === "turn");
  }
  const turns = new Map(turnLines([...tied].map(([id, turn]) => ({ id, turn })), new Set(tied.keys()), lineState).map((turn) => [turn.id, turn.muted]));
  const selectedProject = selection ? agentsById.get(selection)?.projectId : undefined;
  for (const id of tied.keys()) {
    // A folded project's arrow stays bright while the chosen session is inside it.
    const project = clusterProject.get(id);
    const muted = project !== undefined && turns.has(id) ? selection !== null && selectedProject !== project : turns.get(id);
    if (muted === undefined) line(`mine:${id}`, OPERATOR, id, "og-mine");
    else line(`turn:${id}`, id, OPERATOR, `og-turn${muted ? " og-muted" : ""}`, { markerEnd: arrow("var(--warning)") });
  }
  for (const { parent, child, emphasized } of lineageLines(model.visible.filter((row) => endpoint.get(row.id) === row.id).map((row) => row.id), model.layoutParents, teamOf, chosenTeam)) {
    if (endpoint.get(parent) === parent) line(`lineage:${parent}:${child}`, parent, child, `og-lineage${emphasized ? " og-emphasized" : ""}`);
  }
  // A pair is drawn between what its sessions are drawn as; pairs folded onto the same two ends are
  // one line that is emphasised, lettered or two-way when any of them is; two ends folded into one draw nothing.
  const merged = new Map<string, { id: string; from: string; to: string; active: boolean; muted: boolean; letters: boolean; directions: Set<string> }>();
  for (const { pair, active, muted } of communicationLines(model.communication, (id) => endpoint.has(id), lineState)) {
    const from = endpoint.get(pair.source)!;
    const to = endpoint.get(pair.target)!;
    if (from === to) continue;
    const key = JSON.stringify([from, to].sort());
    const entry = merged.get(key) ?? { id: pair.id, from, to, active: false, muted: true, letters: false, directions: new Set<string>() };
    entry.active ||= active;
    entry.muted &&= muted;
    entry.letters ||= pair.letters > 0;
    for (const direction of pair.directions) entry.directions.add(`${endpoint.get(direction.from)}>${endpoint.get(direction.to)}`);
    merged.set(key, entry);
  }
  for (const { id, from, to, active, muted, letters, directions } of merged.values()) {
    const marker = arrow("var(--subtle-foreground)");
    const forward = directions.has(`${from}>${to}`);
    const [start, end] = forward ? [from, to] : [to, from];
    line(id, start, end, `og-communication${letters ? "" : " og-direct-input"}${active ? " og-emphasized" : muted ? " og-muted" : ""}`, { markerEnd: marker, markerStart: directions.size > 1 ? marker : undefined });
  }
  const positionOf = (id: string) => orbs.get(id) ?? { x: 0, y: 0 };
  // A session follows the hand through the physics; a project's name or folded circle carries every session of the project.
  const nodeById = new Map(nodes.map((node) => [node.id, node]));
  const projectOfHandle = (node: Node) => node.type === "project" ? node.id : node.type === "cluster" ? (node as ClusterNode).data.summary.id : null;
  const dragStart = (_: unknown, node: Node) => {
    if (node.type === "session") map?.grab(node.id);
    const project = projectOfHandle(node);
    const anchor = project ? map?.anchorOf(project) : undefined;
    projectDrag.current = project && anchor ? { project, from: { ...node.position }, anchor } : null;
  };
  const dragging = (_: unknown, node: Node) => {
    const shown = nodeById.get(node.id);
    if (!map || !shown) return;
    if (node.type === "session") map.drag(node.id, { x: node.position.x + (orbs.get(node.id)!.x - shown.position.x), y: node.position.y + (orbs.get(node.id)!.y - shown.position.y) });
    // The project follows the hand's whole travel since the grab, measured against the live map, so a
    // step that arrives before the last one was drawn is never counted twice.
    const held = projectDrag.current;
    const now = held ? map.anchorOf(held.project) : undefined;
    if (held && now) map.moveProject(held.project, { x: held.anchor.x + node.position.x - held.from.x - now.x, y: held.anchor.y + node.position.y - held.from.y - now.y });
    setMoved((value) => value + 1);
  };
  // A drop is one event: the session where it was put, or the project with the pinned sessions it carried (D-10).
  const unscaled = (point: { x: number; y: number }) => ({ x: point.x / layout.scale, y: point.y / layout.scale });
  const dragStop = (_: unknown, node: Node) => {
    if (node.type === "session") {
      map?.drop(node.id);
      const pin = map?.pinOf(node.id);
      const session = sessionKeyOf(node.id);
      if (pin && session) actions.changeOverviewGraph({ kind: "place", scope, projects: [], sessions: [{ key: session, ...unscaled(pin) }] });
    } else if (projectDrag.current) {
      const id = projectDrag.current.project;
      map?.dropProject(id);
      const dropped = map ? projectDropPlaces(map, id, remembered, layout.scale) : null;
      const project = projectKeyOf(id);
      const sessions = (dropped?.pins ?? []).flatMap(({ id: pinned, at }) => { const key = sessionKeyOf(pinned); return key ? [{ key, ...at }] : []; });
      if (dropped && project) actions.changeOverviewGraph({ kind: "place", scope, projects: [{ key: project, ...dropped.anchor }], sessions });
    }
    projectDrag.current = null;
    setMoved((value) => value + 1);
  };
  // Unfold all opens the map's projects and every team drawn under a root.
  const unfoldAll = () => actions.changeOverviewGraph({
    kind: "unfold_all",
    projects: model.projects.map((group) => graphProjectKey(group.project.workspace)),
    teams: [...new Set([...[...points].flatMap(([id, point]) => point.team !== id ? [point.team] : []), ...model.quietBranches.keys()])].flatMap((root) => { const key = sessionKeyOf(root); return key ? [key] : []; }),
  });
  const resetLayout = () => { layout.reset(); cameraInitialized.current = false; actions.changeOverviewGraph({ kind: "reset_places", scope }); };
  const showList = mode === "list" || layout.phase === "failed" || !size;
  return <div className="overview-graph" ref={host} data-overview-graph="true" data-graph-phase={layout.phase}>
    <div className="og-toolbar">
      <div><h2>{t("overviewGraph.title")}</h2></div>
      <div className="og-toolbar-actions">{mode === "graph" ? <><Button variant="ghost" size="sm" onClick={foldAll}>{t("overviewGraph.foldAll")}</Button><Button variant="ghost" size="sm" onClick={unfoldAll}>{t("overviewGraph.unfoldAll")}</Button></> : null}{projectId ? <Button variant="ghost" size="sm" onClick={() => useUiStore.getState().setOverview({ projectId: null })}>{t("overviewGraph.allProjects")}</Button> : null}<div role="group" aria-label={t("overviewGraph.title")}><Button variant={mode === "graph" ? "secondary" : "ghost"} size="sm" aria-pressed={mode === "graph"} onClick={() => setMode("graph")}>{t("overviewGraph.graph")}</Button><Button variant={mode === "list" ? "secondary" : "ghost"} size="sm" aria-pressed={mode === "list"} onClick={() => setMode("list")}>{t("overviewGraph.list")}</Button></div></div>
    </div>
    <div className="og-filters">
      <div role="group" aria-label={t("overviewGraph.filter")}>{(["all", "turn", "working", "results"] as const).map((value) => <Button key={value} variant={focus === value ? "secondary" : "ghost"} size="sm" aria-pressed={focus === value} data-focus={value} onClick={() => setFocus(value)}>{t(`overviewGraph.focus.${value}`)} <span>{model.counts[value]}</span></Button>)}</div>
      <label><input type="checkbox" checked={includeResting} onChange={(event) => setIncludeResting(event.target.checked)} />{t("overviewGraph.includeResting")}</label>
    </div>
    {model.limited ? <p className="og-notice" role="status">{t("overviewGraph.limit", { count: LAYOUT_NODE_LIMIT })}</p> : null}
    {model.agents.length > 0 && layout.phase === "loading" && mode === "graph" ? <p className="og-notice" role="status">{t("overviewGraph.loading")}</p> : null}
    {model.agents.length > 0 && layout.phase === "failed" && mode === "graph" ? <div className="og-notice" role="status"><span>{t("overviewGraph.failed")}</span><Button variant="outline" size="sm" onClick={() => setRetry((value) => value + 1)}>{t("overviewGraph.retry")}</Button></div> : null}
    {model.agents.length === 0 ? <div className="og-empty"><p>{t(model.allAgents.length > 0 ? "overviewGraph.filteredEmpty" : "overviewGraph.empty")}</p>{model.allAgents.length > 0 ? <Button variant="outline" size="sm" onClick={() => actions.changeOverviewGraph({ kind: "focus", focus: "all", include_resting: true })}>{t("overviewGraph.resetFilters")}</Button> : null}<Button variant="outline" size="sm" onClick={() => actions.openStartPanel(projectId ?? undefined)}>{t("overviewGraph.newAgent")}</Button></div> : null}
    {model.agents.length > 0 ? <div className="og-stage">
      <div className="og-canvas">{!showList ? <ReactFlow zIndexMode="manual" colorMode={theme} nodes={nodes} edges={edges} nodeTypes={NODE_TYPES} edgeTypes={EDGE_TYPES} nodesDraggable onNodeDragStart={dragStart} onNodeDrag={dragging} onNodeDragStop={dragStop} nodesConnectable={false} nodesFocusable={false} edgesFocusable={false} elementsSelectable={false} deleteKeyCode={null} selectionKeyCode={null} multiSelectionKeyCode={null} panOnDrag panOnScroll zoomOnScroll defaultViewport={{ x: 24, y: 24, zoom: 1 }} minZoom={0.05} maxZoom={2} onPaneClick={() => { close(); setHighlightedSession(null); }} ariaLabelConfig={{ "node.a11yDescription.default": t("overviewGraph.nodeHelp"), "node.a11yDescription.keyboardDisabled": t("overviewGraph.nodeHelp") }}>
        <div className="og-zoom nopan nodrag"><Hint label={t("overviewGraph.zoomOut")}><Button variant="ghost" size="icon-sm" onClick={() => void zoomOut({ duration: 0 })}><MinusIcon /></Button></Hint><Hint label={t("overviewGraph.zoomIn")}><Button variant="ghost" size="icon-sm" onClick={() => void zoomIn({ duration: 0 })}><PlusIcon /></Button></Hint><Hint label={t("overviewGraph.fit")}><Button variant="ghost" size="icon-sm" onClick={fitOperator}><MaximizeIcon /></Button></Hint><Hint label={t("overviewGraph.currentPane")}><Button variant="ghost" size="icon-sm" disabled={!current} onClick={() => { if (current) { setHighlightedSession(current.id); if (orbs.has(current.id)) { const point = positionOf(current.id); void flow.setCenter(point.x, point.y, { zoom: 1, duration: 0 }); } else centerProject(current.projectId); } }}><LocateFixedIcon /></Button></Hint><Hint label={t("overviewGraph.resetLayout")}><Button variant="ghost" size="icon-sm" onClick={resetLayout}><RotateCcwIcon /></Button></Hint></div>
      </ReactFlow> : <SessionList key={`${focus}:${includeResting}`} rows={model.agents} selected={activeSession} choose={chooseSession} />}</div>
      {agent ? <SessionPreview row={agent} model={model} actions={actions} openRecord={openRecord} close={close} choose={chooseSession} /> : <ProjectColumn model={model} query={projectQuery} setQuery={setProjectQuery} hidden={hiddenProjects} toggle={toggleProject} point={setPointedProject} locate={(summary) => summary.admitted ? centerProject(summary.id) : useUiStore.getState().setOverview({ projectId: summary.project.workspace.id })} choose={chooseSession} actions={actions} />}
    </div> : null}
  </div>;
}

/** Who to answer for a root whose turn it is: the root itself when it asks, else the first descendant that does. */
function askingPane(row: GraphAgent, model: OverviewGraphModel): string {
  if (row.agent.demand !== "none") return row.agent.pane_id;
  return overviewGraphTeam(row, model).find((member) => member.row.agent.mark === "turn" && member.row.agent.demand !== "none")?.row.agent.pane_id ?? row.agent.pane_id;
}

/** With nothing chosen, the side column lists the projects with their states, then what waits on the operator. */
function ProjectColumn({ model, query, setQuery, hidden, toggle, point, locate, choose, actions }: { model: OverviewGraphModel; query: string; setQuery: (value: string) => void; hidden: ReadonlySet<string>; toggle: (id: string) => void; point: (id: string | null) => void; locate: (summary: ProjectSummary) => void; choose: (id: string, target: HTMLElement) => void; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const needle = query.trim().toLocaleLowerCase();
  // Every root whose turn it is, drawn, folded or hidden, so the list agrees with the projects' My turn counts.
  const waiting = model.allAgents.filter((row) => model.roots.has(row.id) && graphActivity(row.agent) === "turn");
  return <aside className="og-navigator" aria-label={t("overviewGraph.projects")} data-graph-projects="true">
    <Input aria-label={t("overviewGraph.findProject")} placeholder={t("overviewGraph.findProject")} value={query} onChange={(event) => setQuery(event.target.value)} />
    <div className="og-project-list">{model.summaries.filter((summary) => summary.project.workspace.label.toLocaleLowerCase().includes(needle)).sort((a, b) => a.project.workspace.label.localeCompare(b.project.workspace.label, undefined, { numeric: true }) || a.id.localeCompare(b.id)).map((summary) => {
      const label = summary.project.workspace.label;
      const off = hidden.has(summary.id);
      // Folding is the map's (its name's button, the folded circle, Fold all); the row only hides (PRD overview-graph-remembered D-02, D-03).
      const eye = t(off ? "overviewGraph.revealProject" : "overviewGraph.hideProject", { project: label });
      return <div key={summary.id} data-graph-project-row={summary.project.workspace.id} data-hidden={off} onPointerEnter={() => point(summary.id)} onPointerLeave={() => point(null)} onFocus={() => point(summary.id)} onBlur={() => point(null)}>
        <button type="button" aria-label={t("overviewGraph.locateProject", { project: label })} disabled={off} onClick={() => locate(summary)}><strong>{label}</strong><GraphStateCounts summary={summary} /><span className="og-count">{summary.sessions}</span></button>
        <Hint label={eye}><Button variant="ghost" size="icon-sm" className="og-eye" aria-pressed={off} aria-label={eye} disabled={!summary.admitted} onClick={() => toggle(summary.id)}>{off ? <EyeOffIcon /> : <EyeIcon />}</Button></Hint>
      </div>;
    })}</div>
    {waiting.length > 0 ? <section className="og-waiting" aria-label={t("overviewGraph.waiting")}>
      <h3>{t("overviewGraph.waiting")} <span>{waiting.length}</span></h3>
      {waiting.map((row) => <div key={row.id} className="og-ask" data-graph-ask={row.agent.pane_id}>
        <button type="button" className="og-ask-who" aria-label={rowAccessibleName(t, row.agent, row.device)} onClick={(event) => choose(row.id, event.currentTarget)}><AgentMark kind={row.agent.agent_kind} /><span><AgentRecordTitle agent={row.agent} /></span><small>{row.project.label}</small>{hidden.has(row.projectId) ? <small className="og-hidden-mark">{t("overviewGraph.hiddenProject")}</small> : null}</button>
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
