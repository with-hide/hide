// The practice window never reads or writes the operator's clipboard. The
// shell's own copies and pastes go through the clipboard its actions were made
// with (`practiceClipboard`), and the page's own ⌘C, ⌘X and ⌘V, which the
// browser would answer from the system clipboard, are taken before anything on
// the page hears them (`holdPageClipboard`).

import type { Clipboard } from "../clipboard";
import { pasteClipboard, terminalSelectionText } from "../terminals";
import { PRACTICE_COPY, type CoreEvent } from "./world";

/** A clipboard that keeps the last copy for a paste to take back, and tells the world of each copy. */
export function practiceClipboard(send: (event: CoreEvent) => void): Clipboard {
  let copied: string | null = null;
  return {
    async write(text, what) {
      copied = text;
      send({ kind: PRACTICE_COPY, payload: { text, what } });
      return true;
    },
    read: async () => (copied === null ? null : { kind: "text", text: copied }),
  };
}

const paneOf = (target: EventTarget | null) => (target instanceof Element ? target.closest("[data-pane-view]")?.getAttribute("data-pane-view") ?? null : null);
const fieldOf = (target: EventTarget | null) => (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement ? target : null);

/** `text` in place of the field's selection, heard by the field's own input handler as typing is. */
function replaceSelection(field: HTMLInputElement | HTMLTextAreaElement, text: string) {
  field.setRangeText(text, field.selectionStart ?? field.value.length, field.selectionEnd ?? field.value.length, "end");
  field.dispatchEvent(new Event("input", { bubbles: true }));
}

/** What a copy on the page would take: a terminal's selection, a field's selected text, or the page's own selection. */
function selected(target: EventTarget | null): string {
  const pane = paneOf(target);
  if (pane) return terminalSelectionText(pane) ?? "";
  const field = fieldOf(target);
  if (field) return field.value.slice(field.selectionStart ?? 0, field.selectionEnd ?? 0);
  return window.getSelection()?.toString() ?? "";
}

/**
 * Takes the page's copy, cut and paste before anything on it hears them, so
 * the browser never answers them from the operator's clipboard: a copy goes
 * to `clipboard`, a paste into a terminal or a field takes what `clipboard`
 * holds. With no clipboard (the chapter list) they do nothing. Returns the
 * function that lets them go again.
 */
export function holdPageClipboard(clipboard: Clipboard | null): () => void {
  const copy = (event: Event) => {
    event.preventDefault();
    event.stopImmediatePropagation();
    const text = selected(event.target);
    if (!clipboard || !text) return;
    void clipboard.write(text, "selection");
    const field = paneOf(event.target) ? null : fieldOf(event.target);
    if (event.type === "cut" && field) replaceSelection(field, "");
  };
  const paste = (event: Event) => {
    event.preventDefault();
    event.stopImmediatePropagation();
    if (!clipboard) return;
    const pane = paneOf(event.target);
    // The terminal's own paste takes the shell's actions, whose clipboard is this one.
    if (pane) return void pasteClipboard(pane, "practice");
    const field = fieldOf(event.target);
    if (!field) return;
    void clipboard.read("practice").then((content) => {
      if (content?.kind === "text") replaceSelection(field, content.text);
    });
  };
  document.addEventListener("copy", copy, true);
  document.addEventListener("cut", copy, true);
  document.addEventListener("paste", paste, true);
  return () => {
    document.removeEventListener("copy", copy, true);
    document.removeEventListener("cut", copy, true);
    document.removeEventListener("paste", paste, true);
  };
}
