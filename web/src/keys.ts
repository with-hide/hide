/** The modified-key input policy, for xterm.js attachCustomKeyEventHandler. */

import { modChord, TERMINAL_COPY, TERMINAL_PASTE, type Chord, type KeySystem } from "./shortcuts";

export const SHIFT_ENTER = new Uint8Array([0x1b, 0x0d]);
export const COMMAND_DELETE = new Uint8Array([0x15]);
export const LINE_START = new Uint8Array([0x01]);
export const LINE_END = new Uint8Array([0x05]);

/**
 * What a modified key does in a terminal on `system`, or null when xterm
 * handles it as typed input:
 * - bytes the shell reads for a macOS text-editing key (⇧↩ on every system;
 *   ⌘⌫, ⌘← and ⌘→ on macOS, which Windows and Linux reach with Ctrl+U,
 *   Home and End, keys xterm already sends);
 * - a copy of the selection: ⌘C on macOS is the menu's own; elsewhere
 *   Ctrl+Shift+C, which is the terminal's even with nothing selected, and
 *   Ctrl+C while there is a selection, as Windows Terminal does, so Ctrl+C
 *   still interrupts when nothing is selected;
 * - a paste: Ctrl+Shift+V elsewhere.
 */
export type TerminalKey = { kind: "bytes"; bytes: Uint8Array } | { kind: "copy" } | { kind: "paste" };

type KeyEventLike = Pick<KeyboardEvent, "key" | "code" | "shiftKey" | "metaKey" | "ctrlKey" | "altKey" | "isComposing"> & Partial<Pick<KeyboardEvent, "keyCode">>;

function presses(event: KeyEventLike, chord: Chord): boolean {
  return event.code === chord.code && event.metaKey === !!chord.meta && event.ctrlKey === !!chord.ctrl && event.shiftKey === !!chord.shift && event.altKey === !!chord.alt;
}

/**
 * Whether a key belongs to an input method's composition rather than to the
 * field: an Enter that confirms a Korean syllable is the input method's, and a
 * field that acted on it (blurred, submitted) would see the syllable committed
 * twice. 229 is the key an input method is still processing; Chromium can
 * report it with `isComposing` false (a Windows IME's key="Process"), and
 * Safari sends the confirming Enter after `compositionend` with only 229, so
 * every text field asks this one question before it reads a key.
 */
export function composingKey(event: Pick<KeyboardEvent, "isComposing" | "keyCode"> | { nativeEvent: Pick<KeyboardEvent, "isComposing">; keyCode: number }): boolean {
  const composing = "nativeEvent" in event ? event.nativeEvent.isComposing : event.isComposing;
  return composing || event.keyCode === 229;
}

export function terminalKey(event: KeyEventLike, system: KeySystem, selection: boolean, composing = composingKey({ isComposing: event.isComposing, keyCode: event.keyCode ?? 0 })): TerminalKey | null {
  if (composing) return null;
  const bytes = (value: Uint8Array): TerminalKey => ({ kind: "bytes", bytes: value });
  const chord = !event.ctrlKey && !event.altKey;
  if (event.key === "Enter" && event.shiftKey && !event.metaKey && chord) return bytes(SHIFT_ENTER);
  if (system === "mac") {
    if (!event.metaKey || event.shiftKey || !chord) return null;
    if (event.key === "Backspace") return bytes(COMMAND_DELETE);
    if (event.key === "ArrowLeft") return bytes(LINE_START);
    if (event.key === "ArrowRight") return bytes(LINE_END);
    return null;
  }
  if (presses(event, modChord(TERMINAL_PASTE, system))) return { kind: "paste" };
  if (presses(event, modChord(TERMINAL_COPY, system)) || (selection && presses(event, { code: "KeyC", ctrl: true }))) return { kind: "copy" };
  return null;
}
