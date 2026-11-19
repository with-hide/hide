import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { workspaceKey } from "./viewLayout";
import { readCalls, readDrawnPage, workspaceViewOf } from "./workspace";

// A lane call (PRD right-lane B16, B17): an open, a tool, a reveal, a
// History selection or a Recent Panels display, from this page or not (a CLI
// open from the operator's pane, another page), opens the lane in the core,
// and every such call reaches the page the same way: the core numbers it and
// publishes the front Workspace's last number. This reads that number on
// every snapshot, whether or not the Workspace screen is drawn, so a call
// made while the Overview showed still counts, and draws the column it
// brought forward in a stacked lane. A call that draws a page its Workspace
// did not draw last also asks the Workspace screen to widen the lane for it
// (PRD browser-width-zoom D-04); every snapshot records the page drawn, call
// or not, so a lane closed and opened again, or a desk folded and unfolded,
// over the same page widens nothing.

useShellStore.subscribe((state, previous) => {
  const view = workspaceViewOf(state.rest);
  if (view === workspaceViewOf(previous.rest)) return;
  const ui = useUiStore.getState();
  const { seen, call } = readCalls(ui.laneCallsSeen, view);
  const { drawn, fresh } = readDrawnPage(ui.drawnPages, view);
  if (seen !== ui.laneCallsSeen || drawn !== ui.drawnPages) useUiStore.setState({ laneCallsSeen: seen, drawnPages: drawn });
  if (!call || !view) return;
  // A call draws the column it brought forward in a stacked lane (PRD
  // lane-two-columns B19), as the core recorded it: a reveal or a tool the
  // tool column, an open, a card or the desk button the viewing column.
  ui.setStackFront(view.lane_called_side);
  if (fresh) ui.setPageWiden(workspaceKey(view));
});
