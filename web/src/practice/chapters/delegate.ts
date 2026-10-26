// Chapter 4: three ways to hand work to another agent, all by saying it to the
// lead: a child it starts and keeps talking to, an open agent it finds by what
// it does, and an agent named by the name and ID the lead pane's @ types in.

import { chapter } from "../chapter";
import { CHILD, DELEGATE_WORLD, DOCS, LEAD, SIGNUP, SIGNUP_REFERENCE, type DelegateModel } from "../worlds/delegate";

const pane = (id: string) => `[data-pane-view="${id}"]`;
const say = (text: string) => ({ type: { pane: LEAD, text } });

export default chapter<DelegateModel>({
  id: "delegate",
  number: 4,
  version: 2,
  world: DELEGATE_WORLD,
  quest: true,
  steps: [
    { id: "intro", target: pane(LEAD), side: "inside" },
    { id: "spawn", quest: true, target: pane(LEAD), side: "inside", gesture: say("Hand the tests to a Codex child"), done: (model) => model.stage >= 1 },
    { id: "child", target: `[data-agent-open="${CHILD}"]`, side: "right" },
    { id: "tell", quest: true, target: pane(LEAD), side: "inside", gesture: say("Tell the child to use sqlite"), done: (model) => model.stage >= 2 },
    { id: "report", target: pane(LEAD), side: "inside" },
    { id: "letter", quest: true, target: pane(LEAD), side: "inside", gesture: say("Tell the docs writer the timeout changed"), done: (model) => model.stage >= 3 },
    { id: "received", target: pane(DOCS), side: "left" },
    // The lead's @ types the signup agent's reference into the lead's prompt with no Enter; a sidebar row dropped on
    // the terminal, or Copy name and ID then Paste, types the same reference.
    {
      id: "mention", quest: true, target: `${pane(LEAD)} [data-pane-mention], [data-pane-mention-list]`, side: "bottom",
      gesture: { pick: { at: `${pane(LEAD)} [data-pane-mention]`, path: [`[data-pane-mention-choice="${SIGNUP}"]`] } },
      done: (model) => model.draft.includes(`pane ${SIGNUP}`),
    },
    { id: "send", quest: true, target: pane(LEAD), side: "inside", gesture: say(`${SIGNUP_REFERENCE} update the signup copy for the new timeout`), done: (model) => model.stage >= 4 },
    { id: "end", target: "[data-sidebar-box] [data-sidebar]", side: "right" },
  ],
});
