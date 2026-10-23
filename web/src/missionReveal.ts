import { firstPlans, missionOfPane } from "./missions";
import { allAgents } from "./paneAgents";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { windowIsActive } from "./viewFocus";

// A mission's first plan shows on its Conductor pane (PRD mission-pane-chip
// D-03, D-06, B12-B14): only when the operator is looking at that pane does
// its chip's small map open pinned, once, with the keyboard left in the
// terminal; anywhere else only the chip changes. The rule reads two
// consecutive frames this page received while live: a full snapshot, as after
// a load or a restart, and the first frame after a reconnect, which resumes
// from the page's cursor as a delta (`hided/src/server.rs`, and `ws.ts` marks
// the link live only after applying it), are never a plan that landed while
// the operator watched.

/** Whether the operator looks at `paneId`: the keyboard pane of the Workspace in front, in a window that is shown and focused. */
function looking(paneId: string | null): paneId is string {
  const ui = useUiStore.getState();
  return paneId !== null && ui.screen?.kind === "workspace" && !ui.overviewOpen && document.visibilityState === "visible" && windowIsActive();
}

type ShellState = ReturnType<typeof useShellStore.getState>;

export function revealFirstPlan(state: ShellState, previous: ShellState): void {
  if (state.rest === previous.rest || previous.connection !== "live" || state.viewGeneration !== previous.viewGeneration) return;
  const landed = firstPlans(previous.rest, state.rest);
  if (landed.length === 0) return;
  const paneId = state.focusedPaneId;
  if (!looking(paneId)) return;
  const mission = missionOfPane(state.rest, allAgents(state.agents, state.rest?.status?.remote), paneId);
  if (mission && landed.some((row) => row.id === mission.id)) useUiStore.getState().setPlanPane(paneId);
}

useShellStore.subscribe(revealFirstPlan);
