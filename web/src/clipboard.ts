// The one place the shell reaches the system clipboard. Everything else copies
// and pastes through the `Clipboard` its actions were made with, so a screen
// that must leave the operator's clipboard alone (the practice window) gives
// its actions a clipboard of its own and nothing on it can reach the real one.

/** What a paste takes from the clipboard: an image, the way ⌘V of one attaches it, or else its text. */
export type ClipboardContent = { kind: "image"; file: File } | { kind: "text"; text: string };

export type Clipboard = {
  /** Puts `text` on the clipboard; false when it was refused, which goes to the log under `what`. */
  write(text: string, what: string): Promise<boolean>;
  /** What the clipboard holds, or null when it cannot be read, which goes to the log under `what`. */
  read(what: string): Promise<ClipboardContent | null>;
};

/** What a pane's program wrote to the clipboard (OSC 52), as hided hands it to every window once. */
export type ProgramCopy = { pane_id: string; text_base64: string };

/**
 * Puts a program's copy on the clipboard when this window is the one in
 * front and shows the pane, as a terminal writes it for the client in front
 * and no other: every window of the machine is handed every pane's copy, and
 * only the one the operator works in takes it, from a pane on its screen.
 * A pane kept attached behind another tab, another screen or a zoom writes
 * nothing. Either refusal is the log's (design principle 13).
 */
export function takeProgramCopy(
  copy: ProgramCopy,
  window: { inFront: boolean; shows: (paneId: string) => boolean },
  write: (text: string, what: string) => Promise<boolean>,
  diagnostic: (message: string) => void,
): Promise<boolean> {
  if (!window.inFront) {
    diagnostic(`copy from pane ${copy.pane_id}: this window is not in front`);
    return Promise.resolve(false);
  }
  if (!window.shows(copy.pane_id)) {
    diagnostic(`copy from pane ${copy.pane_id}: the pane is not on this window's screen`);
    return Promise.resolve(false);
  }
  const bytes = Uint8Array.from(atob(copy.text_base64), (char) => char.charCodeAt(0));
  return write(new TextDecoder().decode(bytes), `pane ${copy.pane_id} copy`);
}

/** The system clipboard, through the page's Clipboard API. */
export function browserClipboard(diagnostic: (message: string) => void): Clipboard {
  return {
    async write(text, what) {
      if (!navigator.clipboard) {
        diagnostic(`copy ${what}: the clipboard is unavailable`);
        return false;
      }
      try {
        await navigator.clipboard.writeText(text);
        return true;
      } catch (error) {
        diagnostic(`copy ${what}: ${String(error)}`);
        return false;
      }
    },
    async read(what) {
      if (!navigator.clipboard) {
        diagnostic(`paste ${what}: the clipboard is unavailable`);
        return null;
      }
      try {
        for (const item of navigator.clipboard.read ? await navigator.clipboard.read() : []) {
          const type = item.types.find((name) => name.startsWith("image/"));
          if (!type) continue;
          const image = await item.getType(type);
          return { kind: "image", file: new File([image], `clipboard.${type.slice("image/".length)}`, { type }) };
        }
        return { kind: "text", text: await navigator.clipboard.readText() };
      } catch (error) {
        diagnostic(`paste ${what}: ${String(error)}`);
        return null;
      }
    },
  };
}
