// Chapter 1: splitting, jumping and zooming, closing the working agent and
// reopening it, moving a pane, and opening a tab.

import { chapter } from "../chapter";
import { focusedPane, panesOf, tabById } from "../worlds/kit";
import { reopenedAsClosed, SCREEN_WORLD, type ScreenModel } from "../worlds/screen";

const t1 = (model: ScreenModel) => tabById(model, "t1");
const SHELL = '[data-pane-view^="sh"]';
/** The agent's pane: the first one, or the one a reopen brought it back in. */
const AGENT = ':is([data-pane-view="lead"], [data-pane-view^="ag"])';
/** The Stop-work sheet's close, or, when the learner kept the agent open, its pane's close button that asks again. */
const CONFIRM = `[data-confirm-close] [data-slot="alert-dialog-action"], body:not(:has([data-confirm-close])) ${AGENT} header button:has(> svg.lucide-x)`;
const agentClosed = (model: ScreenModel) => model.closed.at(-1)?.pane.agent != null && !model.panes.some((pane) => pane.agent);

export default chapter<ScreenModel>({
  id: "screen",
  number: 1,
  version: 4,
  world: SCREEN_WORLD,
  quest: true,
  steps: [
    { id: "split", quest: true, target: '[data-pane-view="lead"]', side: "inside", command: "split_right", done: (model) => panesOf(t1(model).root).length === 2 },
    { id: "jump", quest: true, target: '[data-pane-view="lead"]', side: "inside", command: "select_agent_1", done: (model) => panesOf(t1(model).root).length === 2 && focusedPane(model) === "lead" },
    { id: "zoom", quest: true, target: '[data-pane-view="lead"] [data-pane-zoom]', side: "bottom", command: "toggle_zoom", gesture: { click: true }, done: (model) => t1(model).zoomed },
    { id: "unzoom", quest: true, target: '[data-pane-view="lead"] [data-pane-zoom]', side: "bottom", command: "toggle_zoom", gesture: { click: true }, done: (model) => !t1(model).zoomed },
    // ⌘W closes the pane holding the keyboard; the agent is working, so the core asks first.
    { id: "close", quest: true, target: AGENT, side: "inside", command: "close_tab", shown: "[data-confirm-close]" },
    { id: "confirm", target: CONFIRM, side: "bottom", gesture: { click: true }, done: agentClosed },
    { id: "reopen", quest: true, target: SHELL, side: "inside", command: "reopen_closed_tab", done: reopenedAsClosed },
    { id: "move", quest: true, target: `${SHELL} header`, side: "bottom", gesture: { drag: { from: `${SHELL} header`, to: AGENT, edge: "left" } }, done: (model) => panesOf(t1(model).root)[0]!.startsWith("sh") },
    { id: "tab", quest: true, target: "[data-new-agent-tab]", side: "bottom", command: "new_tab", done: (model) => model.tabs.length === 2 },
    { id: "end", target: "[data-checkout-agents-open]", side: "right" },
  ],
});
