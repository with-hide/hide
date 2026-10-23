// Chapter 1's world: one invented project with one Claude agent at work in
// its only tab, answering splits, focus, zoom, pane moves, tabs and groups.

import type { PracticeWorld } from "../world";
import { agent, applyDesk, deskSnapshot, oneTabDesk, shellScreen, type DeskModel, type PracticePane, type PracticePlace } from "./kit";

const PLACE: PracticePlace = { project: "hide-practice", label: "hide-practice", branch: "main" };

function initial(): DeskModel {
  const lead = agent({ pane_id: "lead", identity_label: "Fix the checkout 500", agent_kind: "claude", group: "working", mark: "going", status_code: "working", activity: "working", elapsed: "1m" });
  return oneTabDesk({ id: "lead", agent: lead, title: "Fix the checkout 500" });
}

/** What each pane's terminal shows when it first appears. */
function paneScreen(pane: PracticePane): string {
  if (!pane.agent) return shellScreen(PLACE);
  return [
    "\x1b[38;5;209m✻\x1b[0m Claude Code",
    "",
    "\x1b[2m>\x1b[0m Checkout returns a 500 when I pay. Fix it",
    "",
    "\x1b[38;5;209m●\x1b[0m Following the payment request through the code.",
    "  \x1b[2m⎿  Reading src/payment/checkout.ts…\x1b[0m",
    "",
    "\x1b[38;5;209m✻\x1b[0m \x1b[2mLooking for the cause… (esc to interrupt)\x1b[0m",
  ].join("\r\n");
}

export const SCREEN_WORLD: PracticeWorld<DeskModel> = {
  initial,
  apply: applyDesk,
  snapshot: (model, nowMs) => deskSnapshot([{ place: PLACE, model }], PLACE.project, nowMs),
  panes: (model) => model.panes.map((pane) => pane.id),
  screen: (model, paneId) => paneScreen(model.panes.find((pane) => pane.id === paneId)!),
  lane: false,
};
