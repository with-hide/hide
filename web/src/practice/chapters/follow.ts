// Chapter 3: following agents. The task list's states, a pane head, the turn
// card under the operator's request, a child opened from the head and the way
// back to its parent, and the whole team on the Overview's map.

import { chapter } from "../chapter";
import { FOLLOW_WORLD, focusedOn, type FollowModel } from "../worlds/follow";

const LEAD = '[data-pane-view="lead"]';

export default chapter<FollowModel>({
  id: "follow",
  number: 3,
  version: 1,
  world: FOLLOW_WORLD,
  quest: true,
  steps: [
    { id: "tasks", target: "[data-task-list]", side: "right" },
    { id: "header", target: `${LEAD} header, ${LEAD} [data-pane-header-band]`, side: "bottom" },
    { id: "request", quest: true, target: '[data-pane-turn-request="lead"]', side: "right", gesture: { click: true }, shown: '[data-pane-turn-card="lead"]' },
    { id: "card", target: '[data-pane-turn-card="lead"]', side: "right" },
    { id: "children", quest: true, target: '[data-pane-tree="lead"]', side: "right", gesture: { click: true }, shown: '[data-agent-tree="lead"]' },
    { id: "child", quest: true, target: '[data-agent-child="repro"]', side: "right", gesture: { click: true }, done: (model) => focusedOn(model) === "repro" },
    { id: "parent", quest: true, target: '[data-pane-return="lead"]', side: "bottom", gesture: { click: true }, done: (model) => focusedOn(model) === "lead" },
    { id: "answered", target: `${LEAD} [data-pane-header-band]`, side: "bottom" },
    { id: "overview", quest: true, target: `${LEAD} header`, side: "bottom", command: "overview", shown: "[data-overview-graph]" },
    { id: "map", target: "[data-overview-graph]", side: "inside" },
    { id: "end", side: "center" },
  ],
});
