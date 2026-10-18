import { MarkerType, ReactFlow, ReactFlowProvider, useReactFlow, type Edge, type Node, type NodeTypes, type EdgeTypes } from "@xyflow/react";
import { MaximizeIcon, MinusIcon, PlusIcon, XIcon } from "lucide-react";
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
import { LAYOUT_NODE_LIMIT } from "./dependencyLayout";
import { initialGraphExpansion, overviewGraphFamily, overviewGraphModel, type Communication, type GraphAgent } from "./overviewGraphModel";
import { useGraphReducedMotion, useGraphTheme, useOverviewGraphLayout } from "./overviewGraphLayout";
import { CommunicationGraphEdge, GraphStateCounts, ProjectGraphNode, SessionGraphNode, letterKind, type LetterEdge, type ProjectNode, type SessionNode } from "./overviewGraphNodes";
import type { BoardProject } from "./projectBoard";
import { sessionOfAgentOn, workKey } from "./sessionWork";
import type { WorkProgress, WorkSession } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import "@xyflow/react/dist/base.css";
import "./overviewGraph.css";

type Props = { projects: BoardProject[]; deviceId: string; actions: Actions; onOpenRecord?: (session: WorkSession) => void };
type Selection = { kind: "session" | "letters"; id: string } | null;
const NODE_TYPES: NodeTypes = { project: ProjectGraphNode, session: SessionGraphNode };
const EDGE_TYPES: EdgeTypes = { letters: CommunicationGraphEdge };
const PAGE_SIZE = 50;

/** Production Overview owns the graph; the Factory continues to own its separate worker. */
export function OverviewGraph(props: Props) {
  const projectId = useUiStore((state) => state.overview.projectId);
  return <ReactFlowProvider key={`${props.deviceId}:${projectId ?? "all"}`}><WorkGraph {...props} projectId={projectId} /></ReactFlowProvider>;
}

function WorkGraph({ projects, deviceId, actions, onOpenRecord, projectId }: Props & { projectId: string | null }) {
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
  const [mode, setMode] = useState<"graph" | "list">("graph");
  const [selection, setSelection] = useState<Selection>(null);
  const [preview, setPreview] = useState<string | null>(null);
  const [edgeHover, setEdgeHover] = useState<string | null>(null);
  const [retry, setRetry] = useState(0);
  const host = useRef<HTMLDivElement>(null);
  const opener = useRef<HTMLElement | null>(null);
  const returningFocus = useRef(false);
  const reduced = useGraphReducedMotion();
  const theme = useGraphTheme();
  const model = useMemo(() => overviewGraphModel(projects, rest?.overview_graph, expanded, projectId), [projects, rest?.overview_graph, expanded, projectId]);
  const layout = useOverviewGraphLayout(model, host, retry);
  const { fitView, zoomIn, zoomOut } = useReactFlow();
  const progress = useMemo(() => new Map((summariesEnabled(rest?.status?.background_ai) ? summaries?.progress ?? [] : []).map((row) => [workKey(row.session), row])), [summaries, rest?.status?.background_ai]);
  const toggle = useCallback((id: string) => setExpanded((previous) => { const next = new Set(previous); if (next.has(id)) next.delete(id); else next.add(id); return next; }), []);
  const rememberOpener = useCallback((target?: HTMLElement) => {
    const active = target ?? document.activeElement;
    if (active instanceof HTMLElement && active !== document.body && !active.closest(".og-details")) opener.current = active;
  }, []);
  const previewSession = useCallback((id: string, target: HTMLElement) => { if (!returningFocus.current && !selection) { rememberOpener(target); setPreview(id); } }, [rememberOpener, selection]);
  const chooseSession = useCallback((id: string, target?: HTMLElement) => { rememberOpener(target); setSelection({ kind: "session", id }); setPreview(null); }, [rememberOpener]);
  const chooseLetters = useCallback((id: string, target?: HTMLElement) => { rememberOpener(target); setSelection({ kind: "letters", id }); setPreview(null); }, [rememberOpener]);
  const returnFocus = useReturnFocus(selection !== null || preview !== null, { target: () => opener.current, within: () => host.current?.querySelector<HTMLElement>(".og-details") ?? null });
  const close = useCallback(() => {
    returningFocus.current = true;
    try { returnFocus(new Event("closeAutoFocus", { cancelable: true })); }
    finally { returningFocus.current = false; }
    setSelection(null);
    setPreview(null);
  }, [returnFocus]);
  useEscapeLayer(selection !== null || preview !== null, close);
  const agent = model.agents.find((row) => row.id === (selection?.kind === "session" ? selection.id : selection ? null : preview));
  const bundle = selection?.kind === "letters" ? model.communication.find((row) => row.id === selection.id) : undefined;
  const openRecord = onOpenRecord ?? actions.openSessionRecord;
  const progressFor = (row: GraphAgent) => { const session = sessionOfAgentOn(row.agent, row.deviceId); return session ? progress.get(workKey(session)) ?? null : null; };
  // React Flow requires parents before their relative-positioned children.
  const nodes: Node[] = [];
  const placed = layout.placement;
  if (placed) {
    for (const group of model.projects) {
      const box = placed.groups.get(group.id);
      if (!box) continue;
      const project: ProjectNode = { id: group.id, type: "project", position: { x: box.x, y: box.y }, style: { width: box.width, height: box.height }, data: { group, toggle }, draggable: false, connectable: false, focusable: false, selectable: false, zIndex: 0 };
      nodes.push(project);
    }
    for (const row of model.visible) {
      const point = placed.nodes.get(row.id);
      if (!point) continue;
      const root = !model.lineage.some((edge) => edge.to === row.id);
      const node: SessionNode = { id: row.id, parentId: row.projectId, extent: "parent", type: "session", position: { x: point.x, y: point.y }, style: { width: `var(--size-overview-graph-${root ? "root" : "child"}-width)`, height: `var(--size-overview-graph-${root ? "root" : "child"}-height)` }, selected: selection?.kind === "session" && selection.id === row.id, data: { row, root, progress: progressFor(row), preview: previewSession, choose: chooseSession }, draggable: false, connectable: false, focusable: false, selectable: false, zIndex: 2 };
      nodes.push(node);
    }
  }
  const names = new Map([...model.projects.map((group): [string, string] => [group.id, group.project.workspace.label]), ...model.agents.map((row): [string, string] => [row.id, row.agent.identity_label])]);
  const edges: Edge[] = model.lineage.map((edge) => ({ id: `lineage:${edge.from}:${edge.to}`, source: edge.from, target: edge.to, sourceHandle: "source-right", targetHandle: "target-left", type: "default", className: "og-lineage", ariaLabel: `${t("overviewGraph.lineage")}: ${names.get(edge.from)} → ${names.get(edge.to)}`, focusable: false, selectable: false, zIndex: 1 }));
  const arrow = { type: MarkerType.ArrowClosed, color: "var(--agent-working)", orient: "auto-start-reverse" };
  const projectOf = new Map(model.visible.map((row) => [row.id, row.projectId]));
  const endpointX = (id: string) => {
    const group = placed?.groups.get(id);
    if (group) return group.x + group.width / 2;
    return (placed?.groups.get(projectOf.get(id) ?? "")?.x ?? 0) + (placed?.nodes.get(id)?.x ?? 0);
  };
  edges.push(...model.communication.map((edge): LetterEdge => {
    const leftward = endpointX(edge.source) > endpointX(edge.target);
    const label = `${names.get(edge.source)} ${edge.directions.length > 1 ? "↔" : "→"} ${names.get(edge.target)}: ${t("overviewGraph.letters", { count: edge.count })}`;
    return { id: edge.id, source: edge.source, target: edge.target, sourceHandle: leftward ? "source-left" : "source-right", targetHandle: leftward ? "target-right" : "target-left", type: "letters", className: "og-communication", ariaLabel: label, data: { bundle: edge, label, choose: chooseLetters }, markerEnd: arrow, markerStart: edge.directions.length > 1 ? arrow : undefined, selected: selection?.kind === "letters" && selection.id === edge.id, animated: !reduced && (edgeHover === edge.id || selection?.kind === "letters" && selection.id === edge.id), focusable: false, selectable: false, zIndex: 3 };
  }));
  const known = new Set(nodes.map((node) => node.id));
  const visibleEdges = edges.filter((edge) => known.has(edge.source) && known.has(edge.target));
  const showList = mode === "list" || layout.phase === "failed" || !placed;
  return <div className="overview-graph" ref={host} data-overview-graph="true" data-graph-phase={layout.phase}>
    <div className="og-toolbar">
      <div><h2>{t("overviewGraph.title")}</h2><GraphStateCounts agents={model.agents} /></div>
      <div className="og-toolbar-actions"><Button variant="ghost" size="sm" onClick={() => setExpanded(new Set())}>{t("overviewGraph.collapseAll")}</Button><div role="group" aria-label={t("overviewGraph.title")}><Button variant={mode === "graph" ? "secondary" : "ghost"} size="sm" aria-pressed={mode === "graph"} onClick={() => setMode("graph")}>{t("overviewGraph.graph")}</Button><Button variant={mode === "list" ? "secondary" : "ghost"} size="sm" aria-pressed={mode === "list"} onClick={() => setMode("list")}>{t("overviewGraph.list")}</Button></div></div>
    </div>
    {model.limited ? <p className="og-notice" role="status">{t("overviewGraph.limit", { count: LAYOUT_NODE_LIMIT })}</p> : null}
    <div className="og-legend"><span>{t("overviewGraph.legend")}</span><span>{rest?.overview_graph?.truncated ? t("overviewGraph.letterLimit", { count: rest.overview_graph.limit }) : t("overviewGraph.letterRange")}</span>{rest?.overview_graph?.available !== true ? <span>{t("overviewGraph.lettersUnavailable")}</span> : null}</div>
    {layout.phase === "loading" && mode === "graph" ? <p className="og-notice" role="status">{t("overviewGraph.loading")}</p> : null}
    {layout.phase === "failed" && mode === "graph" ? <div className="og-notice" role="status"><span>{t("overviewGraph.failed")}</span><Button variant="outline" size="sm" onClick={() => setRetry((value) => value + 1)}>{t("overviewGraph.retry")}</Button></div> : null}
    <div className="og-stage">
      {!showList ? <ReactFlow colorMode={theme} nodes={nodes} edges={visibleEdges} nodeTypes={NODE_TYPES} edgeTypes={EDGE_TYPES} nodesDraggable={false} nodesConnectable={false} nodesFocusable={false} edgesFocusable={false} elementsSelectable={false} deleteKeyCode={null} selectionKeyCode={null} multiSelectionKeyCode={null} panOnDrag panOnScroll zoomOnScroll fitView fitViewOptions={{ padding: 0.08, maxZoom: 1 }} minZoom={0.12} maxZoom={2} onPaneClick={close} onEdgeMouseEnter={(_, edge) => setEdgeHover(edge.id)} onEdgeMouseLeave={() => setEdgeHover(null)} onEdgeClick={(_, edge) => { if (edge.type === "letters") chooseLetters(edge.id); }} ariaLabelConfig={{ "node.a11yDescription.default": t("overviewGraph.nodeHelp"), "node.a11yDescription.keyboardDisabled": t("overviewGraph.nodeHelp"), "edge.a11yDescription.default": t("overviewGraph.edgeHelp") }}>
        <div className="og-zoom nopan nodrag"><Hint label={t("overviewGraph.zoomOut")}><Button variant="ghost" size="icon-sm" onClick={() => void zoomOut({ duration: 0 })}><MinusIcon /></Button></Hint><Hint label={t("overviewGraph.zoomIn")}><Button variant="ghost" size="icon-sm" onClick={() => void zoomIn({ duration: 0 })}><PlusIcon /></Button></Hint><Hint label={t("overviewGraph.fit")}><Button variant="ghost" size="icon-sm" onClick={() => void fitView({ padding: 0.08, duration: 0, maxZoom: 1 })}><MaximizeIcon /></Button></Hint></div>
      </ReactFlow> : <SessionList rows={model.agents} choose={chooseSession} />}
      {agent ? <SessionPreview row={agent} family={overviewGraphFamily(agent, model.agents)} progress={progressFor(agent)} actions={actions} openRecord={openRecord} close={close} /> : bundle ? <Correspondence bundle={bundle} names={names} close={close} /> : null}
    </div>
    {model.agents.length === 0 ? <div className="og-empty"><p>{t("overviewGraph.empty")}</p><Button variant="outline" size="sm" onClick={() => actions.openStartPanel(projectId ?? undefined)}>{t("overviewGraph.newAgent")}</Button></div> : null}
  </div>;
}

function SessionList({ rows, choose }: { rows: readonly GraphAgent[]; choose: (id: string, target: HTMLElement) => void }) {
  const { t } = useInterfaceTranslation();
  const [page, setPage] = useState(0);
  const pages = Math.max(1, Math.ceil(rows.length / PAGE_SIZE));
  const current = Math.min(page, pages - 1);
  return <div className="og-list" data-graph-session-list="true">
    {rows.slice(current * PAGE_SIZE, (current + 1) * PAGE_SIZE).map((row) => <button key={row.id} type="button" onClick={(event) => choose(row.id, event.currentTarget)}><AgentMark kind={row.agent.agent_kind} /><span className="og-list-name"><strong>{row.agent.identity_label}</strong><small>{row.project.label} · {row.checkout.label}</small></span><StatusMark mark={row.agent.mark} /><span>{markWords(t, row.agent)}</span></button>)}
    {pages > 1 ? <div className="og-pagination"><Button variant="ghost" size="sm" disabled={current === 0} onClick={() => setPage(current - 1)}>{t("overviewGraph.previous")}</Button><span>{t("overviewGraph.page", { page: current + 1, pages })}</span><Button variant="ghost" size="sm" disabled={current + 1 === pages} onClick={() => setPage(current + 1)}>{t("overviewGraph.next")}</Button></div> : null}
  </div>;
}

function SessionPreview({ row, family, progress, actions, openRecord, close }: { row: GraphAgent; family: { direct: number; descendants: number }; progress: WorkProgress | null; actions: Actions; openRecord: (session: WorkSession) => void; close: () => void }) {
  const { t } = useInterfaceTranslation();
  const session = sessionOfAgentOn(row.agent, row.deviceId);
  const children = row.agent.direct_child_counts;
  const sentence = rowSentence(t, row.agent);
  return <aside className="og-details" aria-label={t("overviewGraph.preview")} data-graph-preview="true">
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
  return <aside className="og-details" aria-label={t("overviewGraph.communication")} data-graph-correspondence="true">
    <div className="og-details-head"><strong>{t("overviewGraph.communication")}</strong><Hint label={t("overviewGraph.close")}><Button variant="ghost" size="icon-sm" onClick={close}><XIcon /></Button></Hint></div>
    {bundle.directions.map((direction) => <p key={`${direction.from}:${direction.to}`} className="og-direction"><span>{names.get(direction.from)} → {names.get(direction.to)}</span><b>{t("overviewGraph.letters", { count: direction.count })}</b></p>)}
    <dl>{Object.entries(bundle.kinds).map(([kind, count]) => <div key={kind}><dt>{letterKind(t, kind)}</dt><dd>{count}</dd></div>)}<div><dt>{t("overviewGraph.latest")}</dt><dd>{formatDateTime(requireInterfaceLanguage(i18n.language), bundle.latest, { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" })}</dd></div></dl>
  </aside>;
}
