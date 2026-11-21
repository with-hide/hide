// Flow 7: focus mode, in five steps. Two agents work side by side and one has
// opened the page it is building. The learner turns focus mode on, so that
// agent's terminal stands on the left and its page beside it; looks at the band
// that says so; watches the agent change the page's heading and the page change
// with it; folds the desk with the desk's ✕ or ⌘E, so the terminal takes
// the width; and leaves the mode, so both agents are side by side again.

import { chapter } from "../chapter";
import { BUILDER, deskFolded, focusEnded, focusedOnBuilder, pageChanged, FOCUS_WORLD, type FocusModel } from "../worlds/focus";

export default chapter<FocusModel>({
  id: "focus",
  number: 7,
  version: 1,
  since: "1.2.11",
  world: FOCUS_WORLD,
  quest: true,
  preview: "focus",
  steps: [
    // Only the agent with a View to show wears the ◧ on its pane head; ⌘. does the same for the pane the keyboard is in.
    { id: "enter", quest: true, target: `[data-pane-view="${BUILDER}"] [data-pane-pair]`, side: "bottom", command: "pair_view", gesture: { click: true }, done: focusedOnBuilder },
    // The band in the tab row's place says the mode is on; the page the agent opened comes up beside the terminal by itself.
    { id: "band", target: "[data-pair-strip]", side: "bottom" },
    // The agent edits the page a few seconds after the mode was entered; the desk beside the terminal shows the new heading.
    { id: "live", target: "[data-editor-body]", side: "inside", wait: true, done: pageChanged },
    // The desk's ✕ is ⌘E in the mode, the same action: the desk folds and the terminal takes the width, and ⌘E again stands it.
    { id: "fold", quest: true, target: "[data-lane-documents-close]", side: "bottom", command: "toggle_lane", gesture: { click: true }, done: deskFolded },
    // The band's way back, or the chord again: the lane and the tab are as they were before the mode.
    { id: "leave", quest: true, target: "[data-pair-leave]", side: "bottom", command: "pair_view", gesture: { click: true }, done: focusEnded },
  ],
});
