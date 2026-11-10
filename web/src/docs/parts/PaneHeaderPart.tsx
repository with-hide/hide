// An agent pane's header: who the agent is, who started it and the helpers it started. It is the product's own
// `PaneView` over four invented rows (a parent, the pane's agent and its two helpers); no terminal is attached.
// `state=asking` shows the same pane asking its parent's operator a question.
import { useLayoutEffect, useMemo } from "react";
import { createActions } from "../../actions";
import { markTone } from "../../agentRow";
import { PaneView } from "../../PaneView";
import { sidebarScene, REFERENCE_FOLDS } from "../../gallery/sceneData";
import type { AgentRow, PaneHeader, PaneRow } from "../../snapshot";
import { useShellStore } from "../../store";
import { NOW, minutesAgo } from "../fixtures/agent";
import { docsProject } from "../fixtures/project";
import { PartFrame } from "../frame";
import type { PartProps } from "../renderers";

const PANE = "a1c3";

function fixture(asking: boolean) {
  const { rest, checkout } = docsProject();
  const scene = sidebarScene("reference", REFERENCE_FOLDS, NOW);
  const own = (id: string) => scene.agents.find((row) => row.pane_id === id)!;
  const quiet = { pulls: null, descendant_mark: null, forked_from: null, revisit: null, raised: undefined, request: undefined } as const;
  const parent: AgentRow = { ...own("a1"), ...quiet, identity_label: "Ship orders v2", lineage_child_pane_ids: [PANE], lineage_collapsed: true, descendant_counts: undefined };
  const pane: AgentRow = {
    ...own(PANE), ...quiet, identity_label: "Fix the checkout bug", group: "working", mark: "going", status_code: "working", activity: "working", wait: undefined, delegated: true,
    lineage_parent_pane_id: "a1", lineage_path_pane_ids: ["a1", PANE], lineage_child_pane_ids: ["a1g1", "a1g2"], direct_child_counts: { error: 0, approval: 0, question: 0, working: 1, done: 1 },
  };
  const helper = (id: string, label: string, over: Partial<AgentRow>): AgentRow => ({ ...own(id), ...quiet, identity_label: label, lineage_parent_pane_id: PANE, lineage_path_pane_ids: ["a1", PANE, id], lineage_child_pane_ids: [], ...over });
  const helpers = [
    helper("a1g1", "Write the tests", { group: "working", mark: "going", status_code: "working", demand: "none" }),
    helper("a1g2", "Update the docs", { group: "seen", mark: "quiet", status_code: "done", activity: "stopped", demand: "none" }),
  ];
  const header: PaneHeader = {
    working: !asking, notice: null, request: { text: "Fix the checkout bug", images: 0 },
    band: asking ? { kind: "answer", reason: "Retry the payment or show an error?", since_unix_ms: minutesAgo(2), read: false, action: null, more: 0 } : null,
    progress: asking ? null : "Splitting the work between two helpers",
  };
  const template = checkout.tabs[0]!.panes[0]!;
  const row: PaneRow = {
    ...template, id: PANE, identity_label: pane.identity_label,
    lineage_path: [{ pane_id: "a1", label: parent.identity_label!, siblings: [] }, { pane_id: PANE, label: pane.identity_label!, siblings: [] }],
  };
  const withHeader = { ...rest, terminal: { headers: { [PANE]: header } }, ui_state: { ...rest.ui_state!, pane_turn_row_mode: "one_line" as const } };
  return { rest: withHeader, agents: [parent, pane, ...helpers], row, pane };
}

export default function PaneHeaderPart({ state }: PartProps) {
  const { rest, agents, row, pane } = useMemo(() => fixture(state === "asking"), [state]);
  const actions = useMemo(() => createActions(() => true), []);
  useLayoutEffect(() => {
    useShellStore.setState({ rest, agents, connection: "gone" });
  }, [rest, agents]);
  return (
    <PartFrame part="pane-header">
      <div className="h-[calc(var(--size-pane-header)*2)] min-w-0 overflow-hidden rounded-md border border-border">
        <PaneView pane={row} transport={undefined} focused={false} scale={1} actions={actions} agentKind="claude" markSymbol={pane.mark} markTone={markTone(pane)} paneCount={2} zoomed={false} />
      </div>
    </PartFrame>
  );
}
