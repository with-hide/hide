// Flow 5: starting the day from the Overview. What is going on (Work), what
// to take up next (Backlog; Hide AI's 제안 in the lane suggests the same
// kind of work), and finding what you did before (History, or ⌘K) and
// bringing it back with Restore session.

import { chapter } from "../chapter";
import { DAY_WORLD, findsLogin, ISSUE_KEY, openedLogin, readsIssue, restoredLogin, type DayModel } from "../worlds/day";

const QUERY = { type: { input: "[data-history-query]", text: "login" } };

export default chapter<DayModel>({
  id: "day",
  number: 5,
  version: 1,
  world: DAY_WORLD,
  quest: true,
  preview: "search",
  steps: [
    // The Overview is a page of its own over the screen; Esc or ⇧⌘O puts it away.
    { id: "open", quest: true, target: "[data-open-overview]", side: "right", command: "overview", shown: "[data-overview-page]" },
    { id: "work", target: "[data-overview-body]", side: "center" },
    { id: "tabs", target: "[data-overview-navigation]", side: "bottom" },
    { id: "backlog", quest: true, target: '[data-main-tab="backlog"]', side: "bottom", gesture: { click: true }, shown: "[data-overview-backlog]" },
    { id: "issue", quest: true, target: `[data-backlog-row="${ISSUE_KEY}"]`, side: "bottom", gesture: { click: true }, done: readsIssue },
    { id: "history", quest: true, target: '[data-main-tab="history"]', side: "bottom", gesture: { click: true }, shown: "[data-overview-history]" },
    // Any word of a session finds it, not only the one the bubble suggests.
    { id: "search", quest: true, target: "[data-history-query]", side: "bottom", gesture: QUERY, done: findsLogin },
    { id: "record", quest: true, target: "[data-history-session] button", side: "bottom", gesture: { click: true }, done: openedLogin },
    { id: "restore", quest: true, target: "[data-record-restore-button]", side: "left", gesture: { click: true }, done: restoredLogin },
    { id: "end", target: "[data-overview-navigation]", side: "bottom" },
  ],
});
