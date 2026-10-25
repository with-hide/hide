// A drag the shell draws (a tab, a divider, the side panel's edge, an
// Explorer item) marks the document root while it runs. The mark sets the
// pointer and stops text selection (`index.css`), and it is the one signal
// the browser displays freeze on: a native page is drawn above the shell, so
// it gives its place to its still until the drag ends (`browserViews.ts`).

/** The root attributes a shell drag sets, one per column its drag belongs to. */
export const SHELL_DRAG_ATTRIBUTES = ["data-view-drag", "data-agent-drag"] as const;

export type ShellDragAttribute = (typeof SHELL_DRAG_ATTRIBUTES)[number];

/**
 * What a drag is doing, as the pointer shows it: moving a tab, a place that
 * would not land, a divider along either axis, or a file the OS drag carries.
 */
export type ShellDrag = "move" | "forbidden" | "col-resize" | "row-resize" | "file";

/** The drags holding each attribute, latest last; the root shows the latest. */
const holders = new Map<ShellDragAttribute, { drag: ShellDrag }[]>();

/**
 * Marks the root for a shell drag until the returned release runs. Drags that
 * overlap (two pointers, or a tab drag's cursor changing) each hold their
 * own mark, so one ending never clears another's.
 */
export function holdShellDrag(drag: ShellDrag, attribute: ShellDragAttribute = "data-view-drag"): () => void {
  const held = holders.get(attribute) ?? [];
  const holder = { drag };
  held.push(holder);
  holders.set(attribute, held);
  document.documentElement.setAttribute(attribute, drag);
  return () => {
    const at = held.indexOf(holder);
    if (at < 0) return;
    held.splice(at, 1);
    const latest = held.at(-1);
    if (latest) document.documentElement.setAttribute(attribute, latest.drag);
    else document.documentElement.removeAttribute(attribute);
  };
}

/** The chip a dragged pane or agent shows under the pointer, one look for both drags. */
export const DRAG_CHIP_CLASS = "max-w-(--size-search-sheet-w) truncate rounded-sm border border-border bg-popover px-sm py-xxs text-caption text-popover-foreground shadow-sm";

/** Whether a shell drag is running now. */
export function shellDragging(): boolean {
  const root = document.documentElement;
  return SHELL_DRAG_ATTRIBUTES.some((attribute) => root.hasAttribute(attribute));
}
