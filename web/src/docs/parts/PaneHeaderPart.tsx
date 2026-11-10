// An agent pane's header: who the agent is, who started it and the helpers it started. It is the product's own
// `PaneView` over four invented rows (a parent, the pane's agent and its two helpers); no terminal is attached.
// `state=asking` shows the same pane asking its parent's operator a question.
import { useLayoutEffect, useMemo } from "react";
import { createActions } from "../../actions";
import { markTone } from "../../agentRow";
import { PaneView } from "../../PaneView";
import type { AgentRow, PaneHeader, PaneRow } from "../../snapshot";
import { useShellStore } from "../../store";
import { docsAgent, docsState, minutesAgo } from "../fixtures/agent";
import { docsProject } from "../fixtures/project";
import { PartFrame } from "../frame";
import type { PartProps } from "../renderers";

const PANE = "a1c3";

function fixture(asking: boolean) {
  const { rest, checkout } = docsProject();
  const working = docsState({
    working: true, root: false, bucket: "working", attention_rank: 3, search_tone: "working", subtree: "working", link: "working", link_rank: 1, verb: "working",
    chip_tone: { kind: "working", read: false }, mark_tone: { kind: "working", read: false },
  });
  const parent: AgentRow = docsAgent("a1", { identity_label: "Ship orders v2", lineage_child_pane_ids: [PANE], lineage_collapsed: true });
  const pane: AgentRow = docsAgent(PANE, {
    identity_label: "Fix the checkout bug", group: "working", mark: "going", status_code: "working", activity: "working", delegated: true, state: working,
    lineage_parent_pane_id: "a1", lineage_path_pane_ids: ["a1", PANE], lineage_child_pane_ids: ["a1g1", "a1g2"], direct_child_counts: { error: 0, approval: 0, question: 0, working: 1, done: 1 },
  });
  const helpers = [
    docsAgent("a1g1", { identity_label: "Write the tests", group: "working", mark: "going", status_code: "working", activity: "working", delegated: true, state: { ...working, tree_rank: 1 }, lineage_parent_pane_id: PANE, lineage_path_pane_ids: ["a1", PANE, "a1g1"] }),
    docsAgent("a1g2", { identity_label: "Update the docs", group: "seen", mark: "quiet", status_code: "done", activity: "stopped", delegated: true, state: docsState({ root: false, tree_rank: 2 }), lineage_parent_pane_id: PANE, lineage_path_pane_ids: ["a1", PANE, "a1g2"] }),
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
