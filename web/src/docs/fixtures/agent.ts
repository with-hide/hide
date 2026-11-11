// A complete agent row, typed as the wire types it, and the one place a part gets its rows from. Every required
// field of `AgentRow` and `AgentState` is written out here, so a field the product adds as required stops this
// file compiling instead of leaving a part that renders wrong; a part names only the facts that differ and
// spreads them over the base (`docsAgent`).
//
// Time is a fixed value, never the wall clock: a docs picture is the same every time it is drawn (`clock.ts`).
import type { AgentRow, AgentState } from "../../snapshot";
import { NOW } from "../now";

export { NOW };
export const minutesAgo = (minutes: number) => NOW - minutes * 60_000;

const QUIET_STATE: AgentState = {
  session: { group: "idle", line: null, unfinished: false },
  attention: false, needs_you: false, root: true,
  title_emphasized: false, selection_emphasizes_title: true,
  asking: false, working: false, waits_on_children: false,
  chip_tone: { kind: "subtle", read: false }, mark_tone: { kind: "subtle", read: false },
  line: null, branch_badge: null, bucket: "resting", attention_rank: 4, search_tone: "muted", subtree: "quiet",
  link: "idle", link_rank: 2, verb: "idle", request_todo: false, descendant_asking: 0, request_since: null, ask: null, tree_rank: 3,
};

const BASE: AgentRow = {
  state: QUIET_STATE,
  id: "docs-agent", pane_id: "docs-agent", identity_label: "docs-agent", agent_kind: "claude",
  device_id: "local", device_pane_id: "docs-agent",
  mark: "quiet", group: "idle", status_code: "idle",
  changed_at_unix_ms: null, emphasized: false, unread: false,
};

/** The base state with the facts a row differs in. */
export const docsState = (fields: Partial<AgentState>): AgentState => ({ ...QUIET_STATE, ...fields });

export function docsAgent(paneId: string, fields: Partial<AgentRow>): AgentRow {
  return { ...BASE, id: paneId, pane_id: paneId, device_pane_id: paneId, ...fields };
}
