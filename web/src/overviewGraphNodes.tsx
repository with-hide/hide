import { BaseEdge, EdgeLabelRenderer, Handle, Position, getBezierPath, useReactFlow, type Edge, type EdgeProps, type Node, type NodeProps } from "@xyflow/react";
import { ChevronDownIcon, ChevronRightIcon, FolderIcon } from "lucide-react";
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

export type ProjectNode = Node<{ group: GraphProject; frameless: boolean; highlight: (id: string | null, focus: boolean) => void; toggle: (id: string) => void }, "project">;
export type SessionNode = Node<{ row: GraphAgent; root: boolean; progress: WorkProgress | null; context: boolean; highlighted: boolean; quietCount: number; folded: boolean; fold: (id: string) => void; highlight: (id: string | null, focus: boolean) => void; choose: (id: string, target: HTMLElement) => void }, "session">;
export type LetterEdge = Edge<{ bundle: Communication; label: string; emphasized: boolean; highlight: (id: string | null, focus: boolean) => void; choose: (id: string, target?: HTMLElement) => void }, "letters">;

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
  const { group, toggle, frameless, highlight } = data;
  const reveal = useRevealNode(id);
  if (frameless) return <div className="og-project og-project-frameless" data-graph-project={group.project.workspace.id}><Ports /></div>;
  return <div className="og-project" data-graph-project={group.project.workspace.id} data-expanded={group.expanded}>
    <Ports />
    <button type="button" className="og-project-heading nodrag nopan" aria-expanded={group.expanded} aria-label={t(group.expanded ? "overviewGraph.collapse" : "overviewGraph.expand", { project: group.project.workspace.label })} onPointerEnter={() => highlight(id, false)} onPointerLeave={() => highlight(null, false)} onFocus={(event) => { reveal(event.currentTarget); highlight(id, true); }} onBlur={() => highlight(null, true)} onClick={() => toggle(group.id)}>
      {group.expanded ? <ChevronDownIcon /> : <ChevronRightIcon />}<FolderIcon />
      <strong>{group.project.workspace.label}</strong><span className="og-count">{group.matches}</span>
    </button>
    <div className="og-project-summary"><GraphStateCounts agents={group.agents} /></div>
    {!group.expanded ? <div className="og-project-footer"><span>{t("overviewGraph.roots", { roots: group.roots, children: group.agents.length - group.roots })}</span><span>{t("overviewGraph.expandShort")} ↘</span></div> : null}
    {group.agents.length === 0 ? <span className="og-empty-project">{t("overviewGraph.emptyProject")}</span> : null}
  </div>;
}

export function SessionGraphNode({ id, data, selected }: NodeProps<SessionNode>) {
  const { t } = useInterfaceTranslation();
  const { row, root, progress, choose, highlight } = data;
  const reveal = useRevealNode(id);
  return <div className="og-session" data-root={root} data-state={row.bucket} data-selected={selected} data-context={data.context} data-highlighted={data.highlighted} data-graph-session={row.agent.pane_id}>
    <Ports />
    <button type="button" className="og-session-target nodrag nopan" aria-label={[row.agent.identity_label, data.context ? t("overviewGraph.context") : null, agentProductName(row.agent.agent_kind), markWords(t, row.agent), row.launchModel ? t("overviewGraph.launchModel", { model: row.launchModel }) : null].filter(Boolean).join(", ")} aria-pressed={selected} onPointerEnter={() => highlight(row.id, false)} onPointerLeave={() => highlight(null, false)} onFocus={(event) => { reveal(event.currentTarget); highlight(row.id, true); }} onBlur={() => highlight(null, true)} onClick={(event) => choose(row.id, event.currentTarget)}>
      <span className="og-orb"><AgentMark kind={row.agent.agent_kind} className="og-provider" /><span className="og-orb-state"><StatusMark mark={row.agent.mark} /></span></span>
      <strong className="og-session-title">{row.agent.identity_label}</strong>
      <span className="og-model">{agentProductName(row.agent.agent_kind)}{row.launchModel ? <><br />{t("overviewGraph.launchModel", { model: row.launchModel })}</> : null}</span>
      <span className="og-session-status">{markWords(t, row.agent)}</span>
      {data.context ? <span className="og-session-status">{t("overviewGraph.context")}</span> : null}
      {progress && progress.total > 0 ? <span className="og-progress">{t("overviewGraph.progress", { done: progress.done, total: progress.total })}</span> : null}
    </button>
    {data.quietCount > 0 ? <button type="button" className="og-fold nodrag nopan" aria-expanded={!data.folded} onClick={() => data.fold(row.id)}>{t(data.folded ? "overviewGraph.unfoldQuiet" : "overviewGraph.foldQuiet", { count: data.quietCount })}</button> : null}
  </div>;
}

export function CommunicationGraphEdge(props: EdgeProps<LetterEdge>) {
  const { t } = useInterfaceTranslation();
  const [path, x, y] = getBezierPath(props);
  const bundle = props.data?.bundle;
  if (!bundle) return null;
  return <>
    <BaseEdge id={props.id} path={path} interactionWidth={20} style={props.style} markerEnd={props.markerEnd} markerStart={props.markerStart} />
    <EdgeLabelRenderer><button type="button" className="og-letter-label nodrag nopan" data-emphasized={props.data?.emphasized} onPointerEnter={() => props.data?.highlight(props.id, false)} onPointerLeave={() => props.data?.highlight(null, false)} onFocus={() => props.data?.highlight(props.id, true)} onBlur={() => props.data?.highlight(null, true)} style={{ transform: `translate(-50%, -50%) translate(${x}px, ${y}px)` }} aria-pressed={props.selected} aria-label={props.data?.label} onClick={(event) => props.data?.choose(bundle.id, event.currentTarget)}>{bundle.directions.length > 1 ? "↔" : "→"} {t("overviewGraph.letters", { count: bundle.count })}</button></EdgeLabelRenderer>
  </>;
}

export function letterKind(t: TFunction<"translation">, kind: string): string {
  switch (kind) {
    case "request": return t("overviewGraph.kind.request");
    case "block": return t("overviewGraph.kind.block");
    case "report": return t("overviewGraph.kind.report");
    case "progress": return t("overviewGraph.kind.progress");
    case "reply": return t("overviewGraph.kind.reply");
    default: return t("overviewGraph.kind.other");
  }
}
