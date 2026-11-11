import { useMemo, type ReactNode } from "react";
import { DependencyGraphView, type EdgeLook, type OverlayEdge } from "./DependencyGraph";
import type { GraphEdge } from "./dependencyLayout";
import { missionOpen, neighbours } from "./missions";
import { layerDependencies } from "./projectBoard";
import type { Mission, MissionMapFlow, MissionNode } from "./snapshot";

const REPORT = "#report";
const NO_DEFECTS: GraphEdge[] = [];

// The desktop and phone share the nodes, dependency layout, report sink and edge styling.
type GraphCard<N> = { key: string; node: N | null };

type GraphNode = Pick<MissionNode, "key" | "depends_on" | "status" | "base_pr"> & { prs: { number: number }[] };

export function MissionGraph<N extends GraphNode>({ mission, flow, selected, onSelect, defects = NO_DEFECTS, drawNode, report }: {
  mission: Pick<Mission, "status"> & { nodes: N[] };
  flow: MissionMapFlow;
  selected: string | null;
  onSelect: (key: string) => void;
  defects?: GraphEdge[];
  drawNode: (node: N, selected: boolean, dimmed: boolean, onSelect: () => void) => ReactNode;
  report: (dimmed: boolean) => ReactNode;
}) {
  const nodes = mission.nodes;
  const graph = useMemo(() => {
    const shown = new Set(nodes.map((node) => node.key));
    const cards: GraphCard<N>[] = [...nodes.map((node) => ({ key: node.key, node })), { key: REPORT, node: null }];
    const byKey = new Map(cards.map((card) => [card.key, card]));
    const depended = new Set(nodes.flatMap((node) => node.depends_on));
    const blockers = (card: GraphCard<N>): GraphCard<N>[] => {
      // Every node no other node waits on ends in the report.
      if (!card.node) return nodes.filter((node) => !depended.has(node.key)).map((node) => byKey.get(node.key)!);
      return card.node.depends_on.filter((key) => shown.has(key)).map((key) => byKey.get(key)!);
    };
    return layerDependencies(cards, (card) => card.key, blockers);
  }, [nodes]);
  const byKey = useMemo(() => new Map(nodes.map((node) => [node.key, node])), [nodes]);
  // A pick keeps itself and every card one edge away lit, the defect edges included (B30).
  const lit = selected ? neighbours([...graph.edges, ...defects], selected) : null;
  const away = (edge: GraphEdge) => selected !== null && edge.from !== selected && edge.to !== selected;
  const edgeLook = (edge: GraphEdge): EdgeLook => {
    const from = byKey.get(edge.from);
    const to = byKey.get(edge.to);
    const dimmed = away(edge);
    // A stack is the one purple dashed edge: the pull request below is the one above's base (B27).
    if (from && to && to.base_pr != null && from.prs.some((pr) => pr.number === to.base_pr)) return { tone: "text-mission-review", dashed: true, dimmed };
    // A line from a node at work is lit only while the mission is open; a cancelled mission's work stopped.
    if (from && from.status === "working" && missionOpen(mission)) return { tone: "text-mission-working", dimmed };
    return { tone: "text-border", dimmed };
  };
  // A defect a later node found is a red dashed edge back to the node it opened, until its line passes (B21).
  const overlay: OverlayEdge[] = defects.map((edge) => ({ ...edge, look: { tone: "text-mission-failed", dashed: true, dimmed: away(edge) } }));
  return (
    <DependencyGraphView
      graph={graph}
      idOf={(card) => card.key}
      flow={flow}
      gaps={{ layers: "--size-mission-layer-gap", nodes: "--size-mission-node-gap" }}
      edgeLook={edgeLook}
      overlay={overlay}
      draw={(card) =>
        card.node ? (
          <div key={card.key}>{drawNode(card.node, selected === card.key, lit !== null && !lit.has(card.key), () => onSelect(card.key))}</div>
        ) : (
          <div key={card.key} data-dependency-node={REPORT}>{report(lit !== null && !lit.has(REPORT))}</div>
        )
      }
    />
  );
}
