import { BaseEdge, EdgeLabelRenderer, Handle, Position, getBezierPath, useReactFlow, type Edge, type EdgeProps, type Node, type NodeProps } from "@xyflow/react";
import { ChevronDownIcon, ChevronRightIcon, FolderIcon, GripIcon } from "lucide-react";
import type { TFunction } from "i18next";
import { AgentMark } from "./AgentMark";
import { agentProductName } from "./agentAdapters";
import { badgeParts, markName, markWords } from "./agentRow";
import { StatusMark } from "./components/status-mark";
import { useInterfaceTranslation } from "./i18n/client";
import type { Communication, GraphAgent, GraphProject } from "./overviewGraphModel";
import type { WorkProgress } from "./snapshot";

/** Custom button focus follows the same viewport rule as React Flow's node focus. */
function useRevealNode(id: string) {
  const flow = useReactFlow();
  return (target: HTMLElement) => {
    const canvas = target.closest(".react-flow")?.getBoundingClientRect();
    const box = target.getBoundingClientRect();
    if (!canvas || box.left >= canvas.left && box.right <= canvas.right && box.top >= canvas.top && box.bottom <= canvas.bottom) return;
    const node = flow.getInternalNode(id);
    if (!node) return;
    const { x, y } = node.internals.positionAbsolute;
    void flow.setCenter(x + (node.measured.width ?? 0) / 2, y + (node.measured.height ?? 0) / 2, { zoom: flow.getViewport().zoom, duration: 0 });
  };
}

export type ProjectNode = Node<{ group: GraphProject; current: boolean; highlighted: boolean; highlight: (id: string | null, focus: boolean) => void; toggle: (id: string) => void }, "project">;
export type SessionNode = Node<{ row: GraphAgent; root: boolean; progress: WorkProgress | null; context: boolean; highlighted: boolean; current: boolean; quietCount: number; folded: boolean; fold: (id: string) => void; highlight: (id: string | null, focus: boolean) => void; choose: (id: string, target: HTMLElement) => void }, "session">;
export type LetterEdge = Edge<{ bundle?: Communication; lineage?: { from: string; to: string; count: number }; label: string; playback: number; emphasized: boolean; paired: boolean; laneOffset: number; highlight: (id: string | null, focus: boolean) => void; choose: (id: string, target?: HTMLElement) => void }, "letters">;

/** Handles are invisible routing anchors; no connection can be edited. */
function Ports() {
  return <><Handle id="target-left" type="target" position={Position.Left} /><Handle id="target-right" type="target" position={Position.Right} /><Handle id="source-left" type="source" position={Position.Left} /><Handle id="source-right" type="source" position={Position.Right} /></>;
}

export function GraphStateCounts({ agents }: { agents: readonly GraphAgent[] }) {
  const { t } = useInterfaceTranslation();
  const counts = { turn: 0, going: 0, done: 0, unread: 0 };
  for (const { agent } of agents) {
    if (agent.mark === "waiting") counts.going += 1;
    else if (agent.mark in counts) counts[agent.mark as keyof typeof counts] += 1;
  }
  return <span className="og-state-counts">{badgeParts(counts).map((part) => <span key={part.state}><StatusMark mark={part.mark} /><span>{markName(t, part.mark as "turn" | "going" | "done" | "unread")} <b>{part.count}</b></span></span>)}</span>;
}

export function ProjectGraphNode({ id, data }: NodeProps<ProjectNode>) {
  const { t } = useInterfaceTranslation();
  const { group, toggle, highlight } = data;
  const reveal = useRevealNode(id);
  const hidden = group.agents.length - group.shown.length;
  return <div className="og-project" data-graph-project={group.project.workspace.id} data-expanded={group.expanded} data-current={data.current} data-highlighted={data.highlighted}>
    <div className="og-project-center">
      <Ports />
      <button type="button" className="og-drag-handle og-project-drag nopan" aria-label={t("overviewGraph.dragProject", { project: group.project.workspace.label })}><GripIcon /></button>
      <button type="button" className="og-project-heading nodrag nopan" title={group.project.workspace.label} aria-expanded={group.expanded} aria-label={t(group.expanded ? "overviewGraph.collapse" : "overviewGraph.expand", { project: group.project.workspace.label })} onPointerEnter={() => highlight(id, false)} onPointerLeave={() => highlight(null, false)} onFocus={(event) => { if (event.currentTarget.matches(":focus-visible")) reveal(event.currentTarget); highlight(id, true); }} onBlur={() => highlight(null, true)} onClick={() => toggle(group.id)}>
        <FolderIcon /><strong>{group.project.workspace.label}</strong><span className="og-count">{group.matches}</span>
      </button>
      <div className="og-project-summary"><GraphStateCounts agents={group.agents} /></div>
      <button type="button" className="og-project-more nodrag nopan" aria-expanded={group.expanded} onClick={() => toggle(group.id)}>{group.expanded ? <ChevronDownIcon /> : <ChevronRightIcon />}{hidden > 0 ? t("overviewGraph.moreSessions", { count: hidden }) : t(group.expanded ? "overviewGraph.collapseAll" : "overviewGraph.expandShort")}</button>
    </div>
  </div>;
}

export function SessionGraphNode({ id, data, selected }: NodeProps<SessionNode>) {
  const { t } = useInterfaceTranslation();
  const { row, root, progress, choose, highlight } = data;
  const reveal = useRevealNode(id);
  return <div className="og-session" data-root={root} data-state={row.bucket} data-selected={selected} data-current={data.current} data-context={data.context} data-highlighted={data.highlighted} data-graph-session={row.agent.pane_id}>
    <Ports />
    <button type="button" className="og-drag-handle og-session-drag nopan" aria-label={t("overviewGraph.dragSession", { session: row.agent.identity_label })}><GripIcon /></button>
    <button type="button" className="og-session-target nodrag nopan" aria-label={[row.agent.identity_label, data.context ? t("overviewGraph.context") : null, agentProductName(row.agent.agent_kind), markWords(t, row.agent), row.launchModel ? t("overviewGraph.launchModel", { model: row.launchModel }) : null].filter(Boolean).join(", ")} aria-pressed={selected} onPointerEnter={() => highlight(row.id, false)} onPointerLeave={() => highlight(null, false)} onFocus={(event) => { if (event.currentTarget.matches(":focus-visible")) reveal(event.currentTarget); highlight(row.id, true); }} onBlur={() => highlight(null, true)} onClick={(event) => choose(row.id, event.currentTarget)}>
      <span className="og-orb"><AgentMark kind={row.agent.agent_kind} className="og-provider" /><span className="og-orb-state"><StatusMark mark={row.agent.mark} /></span></span>
      <strong className="og-session-title" title={row.agent.identity_label}>{row.agent.identity_label}</strong>
      <span className="og-model">{agentProductName(row.agent.agent_kind)}{row.launchModel ? <><br />{t("overviewGraph.launchModel", { model: row.launchModel })}</> : null}</span>
      <span className="og-session-status">{markWords(t, row.agent)}{data.current ? ` · ${t("overviewGraph.currentPane")}` : ""}</span>
      {data.context ? <span className="og-session-status">{t("overviewGraph.context")}</span> : null}
      {progress && progress.total > 0 ? <span className="og-progress">{t("overviewGraph.progress", { done: progress.done, total: progress.total })}</span> : null}
    </button>
    {data.quietCount > 0 ? <button type="button" className="og-fold nodrag nopan" aria-expanded={!data.folded} onClick={() => data.fold(row.id)}>{t(data.folded ? "overviewGraph.unfoldQuiet" : "overviewGraph.foldQuiet", { count: data.quietCount })}</button> : null}
  </div>;
}

export function CommunicationGraphEdge(props: EdgeProps<LetterEdge>) {
  const { t } = useInterfaceTranslation();
  let [path, x, y] = getBezierPath(props);
  const bundle = props.data?.bundle;
  if (!props.data) return null;
  const offset = props.data?.laneOffset ?? 0;
  if (offset !== 0) {
    // Keep the actual handles and arrows, separating the curve's midpoint only.
    // A fixed screen-side lane also works when the two kinds run in opposite directions.
    const reach = Math.max(Math.abs(props.targetX - props.sourceX) / 2, Math.abs(props.targetY - props.sourceY) / 4, Math.abs(offset));
    const startX = props.sourceX + (props.sourcePosition === Position.Left ? -reach : reach);
    const endX = props.targetX + (props.targetPosition === Position.Right ? reach : -reach);
    const startY = props.sourceY + offset * 4 / 3;
    const endY = props.targetY + offset * 4 / 3;
    path = `M ${props.sourceX},${props.sourceY} C ${startX},${startY} ${endX},${endY} ${props.targetX},${props.targetY}`;
    x = (props.sourceX + 3 * startX + 3 * endX + props.targetX) / 8;
    y = (props.sourceY + props.targetY) / 2 + offset;
  }
  return <>
    <path className="og-edge-underlay" d={path} fill="none" />
    <BaseEdge key={props.data.playback} id={props.id} path={path} interactionWidth={20} style={props.style} markerEnd={props.markerEnd} markerStart={props.markerStart} />
    <EdgeLabelRenderer><button type="button" className="og-letter-label nodrag nopan" data-kind={bundle?.kind ?? "lineage"} data-paired={props.data?.paired} data-emphasized={props.data?.emphasized} onPointerEnter={() => props.data?.highlight(props.id, false)} onPointerLeave={() => props.data?.highlight(null, false)} onFocus={() => props.data?.highlight(props.id, true)} onBlur={() => props.data?.highlight(null, true)} style={{ transform: `translate(-50%, -50%) translate(${x}px, ${y}px)` }} aria-pressed={props.selected} aria-label={props.data?.label} onClick={(event) => props.data?.choose(props.id, event.currentTarget)}>{bundle && bundle.directions.length > 1 ? "↔" : "→"} {bundle ? t(bundle.kind === "herdr_direct" ? "overviewGraph.directInputs" : "overviewGraph.letters", { count: bundle.count }) : t("overviewGraph.delegations", { count: props.data.lineage?.count ?? 1 })}</button></EdgeLabelRenderer>
  </>;
}

export function letterKind(t: TFunction<"translation">, kind: string): string {
  switch (kind) {
    case "herdr_direct": return t("overviewGraph.directKind");
    case "request": return t("overviewGraph.kind.request");
    case "block": return t("overviewGraph.kind.block");
    case "report": return t("overviewGraph.kind.report");
    case "progress": return t("overviewGraph.kind.progress");
    case "reply": return t("overviewGraph.kind.reply");
    default: return t("overviewGraph.kind.other");
  }
}
