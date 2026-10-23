// Chapter 5: finding a session from yesterday with ⌘K, reading what was asked
// of it, opening the pull request it made, and keeping a task to look at again.

import { chapter } from "../chapter";
import { FIND_WORLD, findsLogin, findsPr, LOGIN_SESSION, PR_NUMBER, shownDetail, type FindModel } from "../worlds/find";

const FIELD = "[data-palette-input]";
const LOGIN = { type: { input: FIELD, text: "login" } };
const PR = { type: { input: FIELD, text: `#${PR_NUMBER}` } };

// Every step done inside ⌘K names the command that opens it and the words to
// type: ⏎ opens the palette Esc closed, types into its empty field, and past
// words is the palette's own ⏎, which opens the row it highlights.
export default chapter<FindModel>({
  id: "find",
  number: 5,
  version: 2,
  world: FIND_WORLD,
  quest: true,
  steps: [
    { id: "open", quest: true, target: "[data-sidebar-search]", side: "right", command: "search", gesture: { click: true }, done: (model) => model.searched },
    // Any word of yesterday's session finds it, not only the one the bubble suggests.
    { id: "type", quest: true, target: "[data-palette]", side: "left", command: "search", gesture: LOGIN, done: findsLogin },
    { id: "pick", target: "[data-palette]", side: "left", command: "search", gesture: LOGIN, done: (model) => { const top = shownDetail(model); return top?.kind === "record" && top.session.session_id === LOGIN_SESSION.session_id; } },
    { id: "record", target: '[data-lane="detail"]', side: "left" },
    // By number or by a word of its title, as ⌘K lists a pull request.
    { id: "pr", quest: true, target: "[data-agent-area]", side: "inside", command: "search", gesture: PR, done: findsPr },
    { id: "prPick", target: "[data-palette]", side: "left", command: "search", gesture: PR, done: (model) => { const top = shownDetail(model); return top?.kind === "pull_request" && top.number === PR_NUMBER; } },
    { id: "card", target: '[data-lane="detail"]', side: "left" },
    { id: "star", quest: true, target: '[data-task-row="copy"]', side: "right", gesture: { menu: { at: '[data-task-row="copy"]', path: ['[data-menu-item="revisit"]'] } }, done: (model) => model.starred.includes("copy") },
    { id: "revisit", target: '[data-task-state="revisit"]', side: "right" },
    { id: "end", target: "[data-sidebar-search]", side: "right" },
  ],
});
