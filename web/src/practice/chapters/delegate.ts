// Chapter 4: three ways to hand work to another agent, all by saying it to the
// lead: a child it starts and keeps talking to, an open agent it finds by what
// it does, and an agent named by the name and ID copied from its pane's menu.

import { chapter } from "../chapter";
import { CHILD, DELEGATE_WORLD, DOCS, LEAD, SIGNUP, type DelegateModel } from "../worlds/delegate";

const pane = (id: string) => `[data-pane-view="${id}"]`;
const say = (text: string) => ({ type: { pane: LEAD, text } });

export default chapter<DelegateModel>({
  id: "delegate",
  number: 4,
  version: 1,
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
    {
      id: "copy", quest: true, target: `${pane(SIGNUP)} header, [role="menu"]`, side: "left",
      gesture: { menu: { at: `${pane(SIGNUP)} header`, path: ['[data-menu-item="copy_reference"]'] } },
      done: (model) => model.copied?.includes(`pane ${SIGNUP}`) === true,
    },
    { id: "paste", quest: true, target: pane(LEAD), side: "inside", gesture: say(`${SIGNUP} update the signup copy for the new timeout`), done: (model) => model.stage >= 4 },
    { id: "end", target: "[data-sidebar-box] [data-sidebar]", side: "right" },
  ],
});
