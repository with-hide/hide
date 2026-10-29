import { BaseEdge, EdgeLabelRenderer, Handle, Position, useReactFlow, type Edge, type EdgeProps, type Node, type NodeProps } from "@xyflow/react";
import { ChevronsDownUpIcon, GitBranchIcon, MailIcon, TerminalIcon } from "lucide-react";
import { useId } from "react";
import { AgentMark } from "./AgentMark";
import { badgeParts, lineParts, markName, rowAccessibleName, rowLine, sidebarLine } from "./agentRow";
import { AgentRecordTitle } from "./components/agent-record-title";
import { AskLine } from "./components/agent-tree";
import { RevisitStar, starred } from "./components/revisit-mark";
import { RowLineText } from "./components/row-line-text";
import { StatusMark } from "./components/status-mark";
import { Hint } from "./components/ui/tooltip";
import { useInterfaceTranslation } from "./i18n/client";
import type { GraphAgent, GraphProject, ProjectSummary } from "./overviewGraphModel";
import type { ProjectAccent } from "./overviewGraphLines";
import type { AgentRow } from "./snapshot";

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

export type OperatorNode = Node<{ label: string }, "operator">;
/** A project's name above its sessions, in the project's color; dragging it moves the whole project. */
/** `muted`: the side column's search found nothing of this project. */
export type ProjectNode = Node<{ group: GraphProject; accent: ProjectAccent; current: boolean; emphasized: boolean; muted: boolean; fold: (id: string) => void; point: (id: string | null) => void }, "project">;
/** The soft light behind a project's sessions, fading to nothing at its edge. */
export type GlowNode = Node<{ accent: ProjectAccent; emphasized: boolean }, "glow">;
/** A folded project: one circle with its name and counts. */
export type ClusterNode = Node<{ summary: ProjectSummary; accent: ProjectAccent; orb: number; emphasized: boolean; muted: boolean; unfold: (id: string) => void; point: (id: string | null) => void }, "cluster">;
export type TeamNode = Node<{ path: string; width: number; height: number; selected: boolean }, "team">;
/** A team root's fold: how many descendants, their states, and whether they are folded away. */
export type TeamFold = { count: number; turn: number; going: number; done: number; folded: boolean; toggle: () => void };
export type SessionNode = Node<{ row: GraphAgent; root: boolean; branch: string | null; context: boolean; muted: boolean; highlighted: boolean; current: boolean; team: TeamFold | null; highlight: (id: string | null, focus: boolean) => void; choose: (id: string, target: HTMLElement) => void }, "session">;
/** A straight relationship between two orbs, already trimmed to their edges; it carries no words. */
/** A line's letters and Herdr inputs, summed over both directions, and its hint saying them per direction. */
export type ExchangeBadge = { letters: number; direct: number; label: string };
export type LineEdge = Edge<{ x1: number; y1: number; x2: number; y2: number; exchange?: ExchangeBadge }, "line">;

/** Handles are invisible routing anchors; no connection can be edited. */
function Ports() {
  return <><Handle id="target" type="target" position={Position.Left} /><Handle id="source" type="source" position={Position.Right} /></>;
}

/** A mark and count for each nonzero state a project's sessions are in: 내 차례, 진행 중, 읽을 것. */
export function GraphStateCounts({ summary }: { summary: Pick<ProjectSummary, "turn" | "working" | "results"> }) {
  const { t } = useInterfaceTranslation();
  return <span className="og-state-counts">{badgeParts({ turn: summary.turn, going: summary.working, done: summary.results }).map((part) => <span key={part.state} data-state={part.state}><StatusMark mark={part.mark} /><span>{markName(t, part.mark as "turn" | "going" | "done")} <b>{part.count}</b></span></span>)}</span>;
}

/** The line a session's second row reads from: its sidebar line, or its quiet sentence. */
const graphRowLine = (agent: AgentRow) => sidebarLine(agent) ?? rowLine(agent);

/** Whether a session has anything to say on its second row. */
export const hasGraphLine = (agent: AgentRow) => Boolean(agent.state.ask || graphRowLine(agent));

/** The model and effort Hide launched a session with, as chosen ("opus · high"); null without a proved launch. */
export const launchWords = (row: Pick<GraphAgent, "launchModel" | "launchEffort">) => [row.launchModel, row.launchEffort].filter(Boolean).join(" · ") || null;

/** What a session does now, as its sidebar row says it, or what it asks the operator. */
export function GraphLine({ agent, className }: { agent: AgentRow; className?: string }) {
  const { t } = useInterfaceTranslation();
  if (agent.state.ask) return <AskLine ask={agent.state.ask} className={className} />;
  const line = graphRowLine(agent);
  return line ? <RowLineText data-agent-line={line.mode} parts={lineParts(t, agent, line)} className={className} /> : null;
}

export function OperatorGraphNode({ data }: NodeProps<OperatorNode>) {
  return <div className="og-operator" role="img" aria-label={data.label}><Ports /><span aria-hidden="true">{data.label}</span></div>;
}

export function ProjectGraphNode({ data }: NodeProps<ProjectNode>) {
  const { t } = useInterfaceTranslation();
  const label = data.group.project.workspace.label;
  const id = data.group.id;
  // The name drags the project; the fold button beside it is not part of the drag handle. Folding
  // replaces this node under the pointer, so no pointer leave arrives and the emphasis ends with the fold.
  return <div className="og-project" data-graph-project={data.group.project.workspace.id} data-current={data.current} data-emphasized={data.emphasized} data-muted={data.muted} data-accent={data.accent} onPointerEnter={() => data.point(id)} onPointerLeave={() => data.point(null)} onFocus={() => data.point(id)} onBlur={() => data.point(null)}>
    <span className="og-project-name og-project-drag" title={t("overviewGraph.dragProject", { project: label })}>{label}</span>
    <button type="button" className="og-project-fold nodrag nopan" aria-label={t("overviewGraph.foldProject", { project: label })} title={t("overviewGraph.foldProject", { project: label })} onClick={() => { data.point(null); data.fold(id); }}><ChevronsDownUpIcon aria-hidden="true" /></button>
  </div>;
}

export function GlowGraphNode({ id, data, width, height }: NodeProps<GlowNode>) {
  const gradient = `og-glow-${useId().replace(/:/g, "")}`;
  return <svg className="og-glow" width={width} height={height} aria-hidden="true" data-glow={id} data-accent={data.accent} data-emphasized={data.emphasized}>
    <defs><radialGradient id={gradient}><stop offset="0%" className="og-glow-core" /><stop offset="100%" className="og-glow-edge" /></radialGradient></defs>
    <circle cx={(width ?? 0) / 2} cy={(height ?? 0) / 2} r={(width ?? 0) / 2} fill={`url(#${gradient})`} />
  </svg>;
}

/** A folded project stands as one circle in its project's colour, with its glow and its name at the
 * expanded size; the operator's turn inside is a badge on the circle, so the border keeps the colour.
 * Choosing it unfolds the project, dragging it moves the project. */
export function ClusterGraphNode({ data }: NodeProps<ClusterNode>) {
  const { t } = useInterfaceTranslation();
  const { summary } = data;
  const label = summary.project.workspace.label;
  return <div className="og-cluster" data-graph-cluster={summary.project.workspace.id} data-accent={data.accent} data-emphasized={data.emphasized} data-muted={data.muted} onPointerEnter={() => data.point(summary.id)} onPointerLeave={() => data.point(null)} onFocus={() => data.point(summary.id)} onBlur={() => data.point(null)}>
    <Ports />
    <button type="button" className="og-cluster-orb og-cluster-drag nopan" style={{ width: data.orb, height: data.orb }} aria-label={t("overviewGraph.unfoldProject", { project: label, count: summary.sessions })} onClick={() => { data.point(null); data.unfold(summary.id); }}>{summary.sessions}{summary.turn > 0 ? <span className="og-orb-state"><StatusMark mark="turn" /></span> : null}</button>
    <strong>{label}</strong>
    <GraphStateCounts summary={summary} />
  </div>;
}

/** The dashed outline around a chosen team: its root and every delegated descendant. */
export function TeamGraphNode({ data }: NodeProps<TeamNode>) {
  return <svg className="og-team" data-selected={data.selected} width={data.width} height={data.height} aria-hidden="true"><path d={data.path} /></svg>;
}

export function SessionGraphNode({ id, data, selected }: NodeProps<SessionNode>) {
  const { t } = useInterfaceTranslation();
  const { row, root, branch, choose, highlight, team } = data;
  const reveal = useRevealNode(id);
  const asking = row.agent.mark === "turn" || row.agent.mark === "look";
  const launch = launchWords(row);
  return <div className="og-session" data-root={root} data-mark={row.agent.mark} data-selected={selected} data-current={data.current} data-context={data.context} data-muted={data.muted} data-highlighted={data.highlighted} data-folded-turn={team?.folded === true && team.turn > 0} data-graph-session={row.agent.pane_id}>
    <Ports />
    <button type="button" className="og-session-target nopan" aria-label={[rowAccessibleName(t, row.agent, row.device), starred(row.agent) ? t("taskLens.revisit.starred") : null, data.current ? t("overviewGraph.currentPane") : null, data.context ? t("overviewGraph.context") : null, launch ? t("overviewGraph.launchModel", { model: launch }) : null, branch].filter(Boolean).join(", ")} aria-pressed={selected} onPointerEnter={() => highlight(row.id, false)} onPointerLeave={() => highlight(null, false)} onFocus={(event) => { if (event.currentTarget.matches(":focus-visible")) reveal(event.currentTarget); highlight(row.id, true); }} onBlur={() => highlight(null, true)} onClick={(event) => choose(row.id, event.currentTarget)}>
      {/* A delegated session is the same circle, smaller (its size comes from data-root). */}
      <span className="og-orb og-session-drag" title={t("overviewGraph.dragSession", { session: row.agent.identity_label })}><AgentMark kind={row.agent.agent_kind} className="og-provider" /><span className="og-orb-state"><StatusMark mark={row.agent.mark} /></span></span>
      <strong className="og-session-title"><AgentRecordTitle agent={row.agent} />{starred(row.agent) ? <RevisitStar className="ml-xxs" /> : null}</strong>
      {launch ? <span className="og-session-launch" aria-hidden="true">{launch}</span> : null}
      {branch ? <span className="og-session-branch" aria-hidden="true"><GitBranchIcon aria-hidden="true" /><span>{branch}</span></span> : null}
      {root || asking ? <GraphLine agent={row.agent} className={asking ? "og-session-line og-asking" : "og-session-line"} /> : null}
    </button>
    {team ? <button type="button" className="og-team-chip nodrag nopan" data-folded={team.folded} data-turn={team.turn > 0} aria-expanded={!team.folded} aria-label={t(team.folded ? "overviewGraph.unfoldTeam" : "overviewGraph.foldTeam", { count: team.count })} onClick={team.toggle}>
      <span>{team.folded ? `+${team.count}` : team.count}</span>
      {team.folded ? <span className="og-team-states">{badgeParts({ turn: team.turn, going: team.going, done: team.done }).map((part) => <StatusMark key={part.state} mark={part.mark} />)}</span> : null}
    </button> : null}
  </div>;
}

export function LineGraphEdge(props: EdgeProps<LineEdge>) {
  if (!props.data) return null;
  const { x1, y1, x2, y2 } = props.data;
  const path = `M ${x1},${y1} L ${x2},${y2}`;
  return <>
    <path className="og-edge-underlay" d={path} fill="none" />
    <BaseEdge id={props.id} path={path} interactionWidth={0} style={props.style} markerEnd={props.markerEnd} markerStart={props.markerStart} />
    {props.data.exchange ? <EdgeLabelRenderer><ExchangeCount {...props.data.exchange} x={(x1 + x2) / 2} y={(y1 + y2) / 2} /></EdgeLabelRenderer> : null}
  </>;
}

/** The badge at a line's middle: an envelope with its letters, a prompt with its Herdr inputs, each only when nonzero. */
function ExchangeCount({ letters, direct, label, x, y }: ExchangeBadge & { x: number; y: number }) {
  return <Hint label={label} contentClassName="whitespace-pre-line">
    <span role="img" className="og-exchange nodrag nopan" style={{ transform: `translate(-50%, -50%) translate(${x}px, ${y}px)` }} data-graph-exchange="true">
      {letters ? <span><MailIcon aria-hidden="true" />{letters}</span> : null}
      {direct ? <span><TerminalIcon aria-hidden="true" />{direct}</span> : null}
    </span>
  </Hint>;
}
