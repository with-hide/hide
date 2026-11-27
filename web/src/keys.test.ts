import { describe, expect, it } from "vitest";
import { COMMAND_DELETE, LINE_END, LINE_START, SHIFT_ENTER, terminalKey } from "./keys";

function event(partial: Partial<KeyboardEvent>): KeyboardEvent {
  return {
    key: "a",
    code: "KeyA",
    shiftKey: false,
    metaKey: false,
    ctrlKey: false,
    altKey: false,
    isComposing: false,
    ...partial,
  } as KeyboardEvent;
}

const bytes = (value: Uint8Array) => ({ kind: "bytes", bytes: value });

describe("terminalKey on macOS", () => {
  it("maps Shift+Enter to ESC CR", () => {
    expect(terminalKey(event({ key: "Enter", code: "Enter", shiftKey: true }), "mac", false)).toEqual(bytes(SHIFT_ENTER));
  });

  it("maps meta Backspace to ^U and meta arrows to line start and end", () => {
    expect(terminalKey(event({ key: "Backspace", metaKey: true }), "mac", false)).toEqual(bytes(COMMAND_DELETE));
    expect(terminalKey(event({ key: "ArrowLeft", metaKey: true }), "mac", false)).toEqual(bytes(LINE_START));
    expect(terminalKey(event({ key: "ArrowRight", metaKey: true }), "mac", false)).toEqual(bytes(LINE_END));
  });

  it("leaves copy, paste and Ctrl+C to the menu and the program", () => {
    expect(terminalKey(event({ key: "c", code: "KeyC", metaKey: true }), "mac", true)).toBeNull();
    expect(terminalKey(event({ key: "c", code: "KeyC", ctrlKey: true }), "mac", true)).toBeNull();
  });

  it("skips mappings while composing and leaves plain keys alone", () => {
    expect(terminalKey(event({ key: "Enter", shiftKey: true }), "mac", false, true, true)).toBeNull();
    // A key the input method is still processing reads 229 even when
    // Chromium reports `isComposing` false.
    expect(terminalKey(event({ key: "Enter", shiftKey: true, keyCode: 229 }), "mac", false)).toBeNull();
    expect(terminalKey(event({ key: "Backspace", metaKey: true, keyCode: 229 }), "mac", false)).toBeNull();
    expect(terminalKey(event({ key: "Enter" }), "mac", false)).toBeNull();
    expect(terminalKey(event({ key: "a" }), "mac", false)).toBeNull();
  });
});

describe("terminalKey on Windows and Linux", () => {
  const ctrlC = event({ key: "c", code: "KeyC", ctrlKey: true });

  it("copies on Ctrl+C only while there is a selection, so Ctrl+C interrupts otherwise, as Windows Terminal does", () => {
    expect(terminalKey(ctrlC, "pc", true)).toEqual({ kind: "copy" });
    expect(terminalKey(ctrlC, "pc", false)).toBeNull();
  });

  it("copies on Ctrl+Shift+C and pastes on Ctrl+Shift+V, by physical key on any layout", () => {
    expect(terminalKey(event({ key: "C", code: "KeyC", ctrlKey: true, shiftKey: true }), "pc", true)).toEqual({ kind: "copy" });
    expect(terminalKey(event({ key: "С", code: "KeyC", ctrlKey: true, shiftKey: true }), "pc", true)).toEqual({ kind: "copy" });
    // Nothing selected copies nothing, but the chord stays the terminal's (Chrome's element inspector never opens).
    expect(terminalKey(event({ key: "C", code: "KeyC", ctrlKey: true, shiftKey: true }), "pc", false)).toEqual({ kind: "copy" });
    expect(terminalKey(event({ key: "V", code: "KeyV", ctrlKey: true, shiftKey: true }), "pc", false)).toEqual({ kind: "paste" });
  });

  // Another machine's view reads no clipboard, so its paste chord is left to
  // the page's own paste, which carries what this computer copied (#3097);
  // copy stays the terminal's there.
  it("leaves Ctrl+Shift+V to the page's own paste where the page reads no clipboard", () => {
    expect(terminalKey(event({ key: "V", code: "KeyV", ctrlKey: true, shiftKey: true }), "pc", false, false)).toBeNull();
    expect(terminalKey(event({ key: "C", code: "KeyC", ctrlKey: true, shiftKey: true }), "pc", true, false)).toEqual({ kind: "copy" });
    expect(terminalKey(event({ key: "v", code: "KeyV", metaKey: true }), "mac", false, false)).toBeNull();
  });

  it("leaves Ctrl+V, Ctrl+Backspace and the arrows to the program, and keeps Shift+Enter", () => {
    expect(terminalKey(event({ key: "v", code: "KeyV", ctrlKey: true }), "pc", false)).toBeNull();
    expect(terminalKey(event({ key: "Backspace", code: "Backspace", ctrlKey: true }), "pc", false)).toBeNull();
    expect(terminalKey(event({ key: "Backspace", code: "Backspace", ctrlKey: true, shiftKey: true }), "pc", false)).toBeNull();
    expect(terminalKey(event({ key: "ArrowLeft", code: "ArrowLeft", metaKey: true }), "pc", false)).toBeNull();
    expect(terminalKey(event({ key: "Enter", code: "Enter", shiftKey: true }), "pc", false)).toEqual(bytes(SHIFT_ENTER));
  });
});
