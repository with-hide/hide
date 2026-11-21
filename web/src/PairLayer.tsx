import type { Actions } from "./actions";
import { PairPaneCanvas } from "./PaneGrid";

// Focus mode (PRD lane-desk-pair-view B17, D-16, D-17, D-58): the body
// between the sidebar and the rail, under the tab row, drawn as the pane's
// own terminal on the left, its head lined in the mode's accent; the lane
// places the tab's desk on the right half, or nothing while the pane is
// drawn alone at the body's full width (D-53, D-57). The tab's own canvas
// stays under it with its layout as it was.

/** The pane: the left half, `paneWidth` wide (0 until the body is measured), or the whole width while `alone`. */
export function PairLayer({ paneId, alone, paneWidth, covered, actions }: { paneId: string; alone: boolean; paneWidth: number; covered: boolean; actions: Actions }) {
  const width = alone ? "100%" : paneWidth > 0 ? paneWidth : "50%";
  return (
    <div className="absolute inset-x-0 bottom-0 top-(--size-tab-strip) z-10 flex min-h-0 min-w-0 flex-col bg-background" inert={covered} data-pair-layer={paneId} data-pair-alone={alone ? "true" : undefined}>
      <div className="min-h-0 flex-1" style={{ width }}>
        <PairPaneCanvas paneId={paneId} actions={actions} />
      </div>
    </div>
  );
}

