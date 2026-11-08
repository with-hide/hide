// Flow 2: running several agents at once. A reviewer the lead is asked
// for in words, opened beside it, what lands in Needs You (a question)
// and what goes to Unread (finished work), a file an agent opened for the
// learner to look at, closing an agent and reopening it, the keyboard going
// back to the agent used before, and the files the lead changed.

import { chapter } from "../chapter";
import { AGENTS_WORLD, inReviewer, LEAD, REV, reopenedAsClosed, showsExplorer, type AgentsModel } from "../worlds/agents";

const pane = (id: string) => `[data-pane-view="${id}"]`;
const row = (id: string) => `[data-task-row="${id}"]`;
/** The reviewer's row, or, once it was closed and reopened, the row of the pane that brought it back (`ag<n>`). */
const REVIEWER_ROW = `${row(REV)}, [data-task-row^="ag"]`;
const say = (pane: string, text: string) => ({ type: { pane, text } });

export default chapter<AgentsModel>({
  id: "agents",
  number: 2,
  version: 3,
  world: AGENTS_WORLD,
  quest: true,
  preview: "rows",
  steps: [
    // Any words ask the lead for a reviewer: it opens one in a pane beside itself, as `hide agent spawn --beside` does.
    { id: "words", quest: true, target: pane(LEAD), side: "inside", gesture: say(LEAD, "Start a reviewer beside you for the checkout fix"), done: (model) => model.stage >= 1 },
    // Hide's own split opens an empty pane beside, and an agent is started in it by hand; only shown, never pressed.
    { id: "manual", target: pane(LEAD), side: "inside", shows: "split_right" },
    // The docs agent finishes into Unread while the reviewer asks into Needs You.
    { id: "arrives", target: "[data-task-list]", side: "right", wait: true, done: (model) => model.stage >= 3 },
    { id: "ask", quest: true, target: row(REV), side: "right", gesture: { click: true }, done: (model) => model.looked || model.stage >= 4 },
    { id: "answer", quest: true, target: pane(REV), side: "inside", gesture: say(REV, "Yes, run it"), done: (model) => model.stage >= 4 },
    { id: "finishes", target: row(REV), side: "right", wait: true, done: (model) => model.stage >= 5 },
    // The reviewer is finished, so closing it asks nothing; Reopen closed tab brings it back with its conversation, its View still waiting.
    { id: "close", quest: true, target: pane(REV), side: "inside", command: "close_tab", done: (model) => model.closed.length > 0 },
    { id: "reopen", quest: true, target: "[data-agent-area]", side: "inside", command: "reopen_closed_tab", done: reopenedAsClosed },
    { id: "switch", quest: true, target: "[data-agent-area]", side: "inside", command: "recent_area_tab", done: (model) => model.switched && !inReviewer(model) },
    // The row marked 볼 것 brings the agent and the file it opened forward in the lane.
    { id: "look", quest: true, target: REVIEWER_ROW, side: "right", gesture: { click: true }, done: (model) => model.viewed },
    // Pressing the row opened what the agent opened in Documents; the step names it and what it can show.
    { id: "viewer", target: '[data-lane-tool="documents"]', side: "left" },
    { id: "changes", quest: true, target: '[data-lane-tool="explorer"]', side: "left", command: "toggle_lane_explorer", done: showsExplorer },
  ],
});
