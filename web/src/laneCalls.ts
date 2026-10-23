import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { workspaceKey } from "./viewLayout";
import { pageCalled, readCalls, workspaceViewOf } from "./workspace";

// A lane call (PRD right-lane B16, B17): an open, a tool, a reveal, a
// History selection or a Recent Panels display, from this page or not (a CLI
// open from the operator's pane, another page), opens the lane in the core,
// and every such call reaches the page the same way: the core numbers it and
// publishes the front Workspace's last number. This reads that number on
// every snapshot, whether or not the Workspace screen is drawn, so a call
// made while the Overview showed still counts, and draws the column it
// brought forward in a stacked lane. A call that put a page in front also
// asks the Workspace screen to widen the lane for it (PRD browser-width-zoom
// D-04).

useShellStore.subscribe((state, previous) => {
  const view = workspaceViewOf(state.rest);
  const before = workspaceViewOf(previous.rest);
  if (view === before) return;
  const ui = useUiStore.getState();
  const { seen, call } = readCalls(ui.laneCallsSeen, view);
  if (seen !== ui.laneCallsSeen) useUiStore.setState({ laneCallsSeen: seen });
  if (!call || !view) return;
  // A call draws the column it brought forward in a stacked lane (PRD
  // lane-two-columns B19), as the core recorded it: a reveal or a tool the
  // tool column, an open or the 보기 slot the viewing column.
  ui.setStackFront(view.lane_called_side);
  if (pageCalled(before, view)) ui.setPageWiden(workspaceKey(view));
});
