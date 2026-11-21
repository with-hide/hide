// Flow 6: handing a daily job to an automation, in five steps. The learner
// opens the sidebar's 자동화 row, makes the first starting point (the form
// opens filled in), and sees the detail with its next run. The night passes by
// itself: the run starts at its time, its agent reads the issues and opens a
// briefing, and the run stands in 내 차례 marked 볼 것. Pressing that row opens
// the run, with its terminal on the left and the briefing on the right.

import { chapter } from "../chapter";
import { AUTOMATION_WORLD, NIGHT, RUN_PANE, isMade, runFinished, runViewed, type AutomationModel } from "../worlds/automation";

export default chapter<AutomationModel>({
  id: "automation",
  number: 6,
  version: 1,
  since: "1.2.11",
  world: AUTOMATION_WORLD,
  quest: true,
  preview: "automation",
  steps: [
    // The row under 미션 opens the automation screen in the middle; with none made, it welcomes with three starting points.
    { id: "open", quest: true, target: "[data-sidebar-automations]", side: "right", gesture: { click: true }, shown: "[data-automation-screen]" },
    // A starting point opens the form filled in; it is read, changed and made like any other.
    { id: "pick", quest: true, target: '[data-automation-example="issues"]', side: "left", gesture: { click: true }, shown: "[data-automation-form]" },
    // The make is a call the window waits on; the answer opens the new automation's detail, with its next run.
    { id: "make", quest: true, target: "[data-automation-create]", side: "right", gesture: { click: true }, done: isMade },
    // The night passes: the run starts at its time, then its agent opens a briefing and the run ends. The world begins it when the learner comes here.
    { id: "morning", target: "[data-automation-runs]", side: "bottom", wait: true, enter: { kind: NIGHT, payload: {} }, done: runFinished },
    // What waits for the learner is in 내 차례, marked 볼 것; the row opens the run, its terminal beside what its agent opened.
    { id: "result", quest: true, target: `[data-task-row="${RUN_PANE}"]`, side: "right", gesture: { click: true }, done: runViewed },
  ],
});
