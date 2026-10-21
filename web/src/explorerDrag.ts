import { holdShellDrag } from "./shellDrag";
import type { ExplorerClipboard } from "./ui";

/** One browser drag, qualified by the machine and checkout that own the row. */
export type ExplorerDrag = { path: string; directory: boolean; device: string; root: string };
export const explorerDrag: { current: ExplorerDrag | null } = { current: null };

/** A row can disappear during the drag, so its native dragend owns cleanup. */
export function beginExplorerDrag(item: ExplorerDrag, source: HTMLElement): void {
  explorerDrag.current = item;
  const release = holdShellDrag("file");
  source.addEventListener("dragend", () => {
    if (explorerDrag.current === item) explorerDrag.current = null;
    release();
  }, { once: true });
}

/** The same quoted spelling as terminal file attachments, without Enter. */
export function terminalPathText(path: string): string {
  const control = Array.from(path).some((character) => {
    const code = character.codePointAt(0)!;
    return code < 32 || (code >= 127 && code <= 159);
  });
  if (path.length > 4096 || !/^(?:\/|[A-Za-z]:\/)/.test(path) || control) {
    throw new Error("invalid_path");
  }
  return `"${path.replace(/[\\"$`]/g, "\\$&")}" `;
}

/** Image files use the existing binary attachment ingress, preserving format. */
export function isImageDrag(item: ExplorerDrag): boolean {
  return !item.directory && /\.(png|jpe?g|gif|webp|bmp|tiff?|ico|avif)$/i.test(item.path);
}

/** Only the path this window's Copy wrote receives file-paste treatment. */
export function copiedExplorerFile(held: ExplorerClipboard | null, text: string): ExplorerDrag | null {
  if (!held?.terminalCopy || held.mode !== "copy" || held.isDirectory || held.path !== text) return null;
  return { path: held.path, directory: false, device: held.device, root: held.root };
}
