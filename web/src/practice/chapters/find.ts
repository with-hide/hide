// Chapter 5: finding a session from yesterday with ⌘K, reading what was asked
// of it, opening the pull request it made, and keeping a task to look at again.

import { chapter } from "../chapter";
import { FIND_WORLD, LOGIN_SESSION, PR_NUMBER, shownDetail, type FindModel } from "../worlds/find";

const FIELD = "[data-palette-input]";

export default chapter<FindModel>({
  id: "find",
  number: 5,
  version: 1,
  world: FIND_WORLD,
  quest: true,
  steps: [
    { id: "open", quest: true, target: "[data-sidebar-search]", side: "right", command: "search", gesture: { click: true }, done: (model) => model.searched },
    { id: "type", quest: true, target: "[data-palette]", side: "left", gesture: { type: { input: FIELD, text: "login" } }, done: (model) => model.query === "login" },
    // The palette holds the keyboard: ⏎ there opens the session it highlights.
    { id: "pick", target: "[data-palette]", side: "left", done: (model) => { const top = shownDetail(model); return top?.kind === "record" && top.session.session_id === LOGIN_SESSION.session_id; } },
    { id: "record", target: '[data-lane="detail"]', side: "left" },
    { id: "pr", quest: true, target: "[data-agent-area]", side: "inside", command: "search", gesture: { type: { input: FIELD, text: `#${PR_NUMBER}` } }, done: (model) => model.query === `#${PR_NUMBER}` },
    { id: "prPick", target: "[data-palette]", side: "left", done: (model) => { const top = shownDetail(model); return top?.kind === "pull_request" && top.number === PR_NUMBER; } },
    { id: "card", target: '[data-lane="detail"]', side: "left" },
    { id: "star", quest: true, target: '[data-task-row="copy"]', side: "right", gesture: { menu: { at: '[data-task-row="copy"]', path: ['[data-menu-item="revisit"]'] } }, done: (model) => model.starred.includes("copy") },
    { id: "revisit", target: '[data-task-state="revisit"]', side: "right" },
    { id: "end", target: "[data-sidebar-search]", side: "right" },
  ],
});
