#!/usr/bin/env node
// Types into the pane that holds the page's keyboard through the page, so the
// same input reaches a Herdr pane and an own-engine pane (PRD
// engine-bundle-3-core-owns D-15): the line is CDP `Input.insertText` into the
// terminal's text field, as an input method delivers text, and Enter and
// Ctrl+C are CDP key events the web shell's key path handles. usage: type.mjs <cdp-port> <pane> [--focus] [--interrupt] <line>
// `--interrupt` presses Ctrl+C first; the line is followed by Enter.
// `--focus` first selects the pane and focuses its terminal text field.
// Otherwise refuses when the page's keyboard is in another pane.
import { connectPage } from "./cdp.mjs";

const [port, pane, ...rest] = process.argv.slice(2);
const focus = rest[0] === "--focus";
if (focus) rest.shift();
const interrupt = rest[0] === "--interrupt";
const line = interrupt ? rest[1] : rest[0];
if (!port || !pane || line === undefined) throw new Error("usage: type.mjs <cdp-port> <pane> [--focus] [--interrupt] <line>");
const CTRL = 2;
const page = await connectPage(port);
try {
  if (focus) {
    const point = await page.evaluate(`(() => {
      const terminal = document.querySelector(${JSON.stringify(`[data-pane-view="${pane}"] .xterm`)});
      if (!terminal) throw new Error("the measured terminal is not mounted");
      const rect = terminal.getBoundingClientRect();
      return { x: rect.x + rect.width / 2, y: rect.y + rect.height / 2 };
    })()`);
    await page.send("Input.dispatchMouseEvent", { type: "mousePressed", ...point, button: "left", clickCount: 1 });
    await page.send("Input.dispatchMouseEvent", { type: "mouseReleased", ...point, button: "left", clickCount: 1 });
    const deadline = Date.now() + 30_000;
    while (!await page.evaluate(`(() => {
      const view = document.querySelector(${JSON.stringify(`[data-pane-view="${pane}"]`)});
      const input = view?.querySelector("textarea");
      input?.focus();
      return window.__hideProbe?.paneId() === ${JSON.stringify(pane)} && input && document.activeElement === input;
    })()`)) {
      if (Date.now() >= deadline) throw new Error(`pane ${pane} did not take keyboard focus`);
      await new Promise((resolve) => setTimeout(resolve, 30));
    }
  }
  const holder = await page.evaluate("window.__hideProbe && window.__hideProbe.paneId()");
  if (holder !== pane) throw new Error(`the page's keyboard is in ${holder}, not ${pane}`);
  const key = async (code, keyName, keyCode, modifiers = 0, text) => {
    const base = { code, key: keyName, windowsVirtualKeyCode: keyCode, nativeVirtualKeyCode: keyCode, modifiers };
    await page.send("Input.dispatchKeyEvent", { type: text ? "keyDown" : "rawKeyDown", ...base, ...(text ? { text } : {}) });
    await page.send("Input.dispatchKeyEvent", { type: "keyUp", ...base });
  };
  if (interrupt) await key("KeyC", "c", 67, CTRL);
  await page.send("Input.insertText", { text: line });
  await key("Enter", "Enter", 13, 0, "\r");
} finally {
  page.close();
}
