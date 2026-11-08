// Flow 2: running several agents yourself. A Claude Code the lead is asked in words to open beside it, the
// learner's own splits, a job given to the lead whose child is listed from the chip on its pane head, what
// lands in Needs You (a question) and what goes to Unread (finished work), a file an agent opened for the
// learner to look at, closing an agent and reopening it, and the keyboard going back to the agent used before.

import { chapter } from "../chapter";
import { AGENTS_WORLD, inReviewer, LEAD, leadTabPanes, REV, reopenedAsClosed, type AgentsModel } from "../worlds/agents";

const pane = (id: string) => `[data-pane-view="${id}"]`;
const row = (id: string) => `[data-task-row="${id}"]`;
/** The reviewer's row, or, once it was closed and reopened, the row of the pane that brought it back (`ag<n>`). */
const REVIEWER_ROW = `${row(REV)}, [data-task-row^="ag"]`;
/** The words typed in the learner's language, a catalog key read when they are typed. */
const words = (pane: string, key: string) => ({ type: { pane, words: key } });
const say = (pane: string, text: string) => ({ type: { pane, text } });

export default chapter<AgentsModel>({
  id: "agents",
  number: 2,
  version: 5,
  world: AGENTS_WORLD,
  quest: true,
  preview: "rows",
  steps: [
    // Any words ask the lead to open an agent: it opens one in a pane beside itself, an independent agent of the learner's.
    { id: "words", quest: true, target: pane(LEAD), side: "inside", gesture: words(LEAD, "practice.agents.words.example"), done: (model) => model.stage >= 1 },
    // Hide's own splits open an empty pane, and an agent is started in it by hand.
    { id: "right", quest: true, target: pane(LEAD), side: "inside", command: "split_right", done: (model) => leadTabPanes(model) >= 3 },
    { id: "down", quest: true, target: pane(LEAD), side: "inside", command: "split_down", done: (model) => leadTabPanes(model) >= 4 },
    // A job given to the lead: it starts a child for part of it, and the chip on its pane head lists its children.
    { id: "job", quest: true, target: pane(LEAD), side: "inside", gesture: words(LEAD, "practice.agents.job.example"), done: (model) => model.kid },
    { id: "children", quest: true, target: `[data-pane-tree="${LEAD}"]`, side: "right", gesture: { click: true }, shown: '[data-agent-child="kid"]' },
    // The docs agent finishes into Unread while the reviewer asks into Needs You.
    { id: "arrives", target: "[data-task-list]", side: "right", wait: true, done: (model) => model.stage >= 3 },
    { id: "ask", quest: true, target: row(REV), side: "right", gesture: { click: true }, done: (model) => model.looked || model.stage >= 4 },
    { id: "answer", quest: true, target: pane(REV), side: "inside", gesture: say(REV, "Yes, run it"), done: (model) => model.stage >= 4 },
    { id: "finishes", target: row(REV), side: "right", wait: true, done: (model) => model.stage >= 5 },
    // The reviewer is finished, so closing it asks nothing; Reopen closed tab brings it back with its conversation, its View still waiting.
    { id: "close", quest: true, target: pane(REV), side: "inside", command: "close_pane", done: (model) => model.closed.length > 0 },
    { id: "reopen", quest: true, target: "[data-agent-area]", side: "inside", command: "reopen_closed_tab", done: reopenedAsClosed },
    { id: "switch", quest: true, target: "[data-agent-area]", side: "inside", command: "recent_area_tab", done: (model) => model.switched && !inReviewer(model) },
    // The row marked 볼 것 brings the agent and the file it opened forward in the lane.
    { id: "look", quest: true, target: REVIEWER_ROW, side: "right", gesture: { click: true }, done: (model) => model.viewed },
    // Pressing the row opened what the agent opened in Documents; the step names it and what it can show.
    { id: "viewer", target: '[data-lane-tool="documents"]', side: "left" },
  ],
});
