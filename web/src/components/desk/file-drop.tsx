import { createContext, useContext, useState, type DragEvent, type ReactNode, type Ref } from "react";
import { dragHasFiles, type ComposerTray } from "../../deskComposer";

const DropOver = createContext(false);

/**
 * Whether files are dragged over the zone the composer stands in: the
 * composer lights its border then, and nothing else changes (Doop L).
 */
export function useFileDropOver(): boolean {
  return useContext(DropOver);
}

/**
 * A whole conversation column that takes dropped files into its composer's
 * tray (feedback 2 frame L): Desk's column (its head, conversation and box)
 * and the session panel beside it. Only a drag of files from the system is
 * taken, so text, links and Hide's own rows keep their own drop; the
 * dragged file looks as the system draws it, and the composer inside lights
 * the border it has for a drag. Without a `tray` (no box to attach to, as a
 * sleeping session's wake line stands in its place) the zone takes nothing.
 * `dragging` draws a drag in progress, for review.
 */
export function FileDropZone({ tray, dragging = false, className, ref, children, ...data }: { tray: ComposerTray | null; dragging?: boolean; className?: string; ref?: Ref<HTMLDivElement>; children: ReactNode } & Record<`data-${string}`, string>) {
  const [over, setOver] = useState(dragging);
  const dragOver = (event: DragEvent<HTMLElement>) => {
    if (!tray || !dragHasFiles([...event.dataTransfer.types])) return;
    event.preventDefault();
    event.dataTransfer.dropEffect = "copy";
    setOver(true);
  };
  const dragLeave = (event: DragEvent<HTMLElement>) => {
    if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setOver(false);
  };
  const drop = (event: DragEvent<HTMLElement>) => {
    setOver(false);
    if (!tray || event.dataTransfer.files.length === 0) return;
    event.preventDefault();
    event.stopPropagation();
    tray.add([...event.dataTransfer.files]);
  };
  return (
    <DropOver.Provider value={tray !== null && (over || dragging)}>
      <div ref={ref} className={className} onDragEnter={dragOver} onDragOver={dragOver} onDragLeave={dragLeave} onDrop={drop} {...data}>{children}</div>
    </DropOver.Provider>
  );
}
