// Chapter 1: splitting, jumping, zooming and moving panes, and opening a tab.

import { chapter } from "../chapter";
import { focusedPane, panesOf, tabById, type DeskModel } from "../worlds/kit";
import { SCREEN_WORLD } from "../worlds/screen";

const t1 = (model: DeskModel) => tabById(model, "t1");
const SHELL = '[data-pane-view^="sh"]';

export default chapter<DeskModel>({
  id: "screen",
  number: 1,
  version: 3,
  world: SCREEN_WORLD,
  quest: true,
  steps: [
    { id: "split", quest: true, target: '[data-pane-view="lead"]', side: "inside", command: "split_right", done: (model) => panesOf(t1(model).root).length === 2 },
    { id: "jump", quest: true, target: '[data-pane-view="lead"]', side: "inside", command: "select_agent_1", done: (model) => panesOf(t1(model).root).length === 2 && focusedPane(model) === "lead" },
    { id: "zoom", quest: true, target: '[data-pane-view="lead"] [data-pane-zoom]', side: "bottom", command: "toggle_zoom", gesture: { click: true }, done: (model) => t1(model).zoomed },
    { id: "unzoom", quest: true, target: '[data-pane-view="lead"] [data-pane-zoom]', side: "bottom", command: "toggle_zoom", gesture: { click: true }, done: (model) => !t1(model).zoomed },
    { id: "move", quest: true, target: `${SHELL} header`, side: "bottom", gesture: { drag: { from: `${SHELL} header`, to: '[data-pane-view="lead"]', edge: "left" } }, done: (model) => panesOf(t1(model).root)[0]!.startsWith("sh") },
    { id: "tab", quest: true, target: "[data-new-agent-tab]", side: "bottom", command: "new_tab", done: (model) => model.tabs.length === 2 },
    { id: "end", target: "[data-checkout-agents-open]", side: "right" },
  ],
});
