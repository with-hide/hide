// with-hide/hide#2952: a paste into a pane reaches its program in bracketed
// paste exactly when the program turned that mode on. The page never sees the
// mode (the holder's screen fragments do not carry it), so the pane's holder
// wraps the paste: the browser to hided to the node and the holder to the PTY.
import { expect, test, type Page } from "@playwright/test";
import { openLayout, startLayoutStack } from "./engine-fixture";

test.describe.configure({ timeout: 90_000 }); // wait-ok: a hided, its engine and a pane's shell started and stopped whole, with two commands typed and read

const paneText = (page: Page, pane: string) => page.evaluate((id) => window.__hideProbe?.paneText(id) ?? "", pane);

/** A paste of `text` on the pane's terminal, as ⌘V or the Edit menu's Paste delivers it. */
const paste = (page: Page, pane: string, text: string) => page.evaluate(({ pane, text }) => {
  const data = new DataTransfer();
  data.setData("text/plain", text);
  document.querySelector(`[data-pane-view="${pane}"] .xterm-helper-textarea`)!
    .dispatchEvent(new ClipboardEvent("paste", { clipboardData: data, bubbles: true, cancelable: true }));
}, { pane, text });

test("a multi-line paste is bracketed only for a program that turned bracketed paste on", async ({ page }) => {
  const stack = await startLayoutStack({ panes: 1, label: "terminal-paste" });
  try {
    await openLayout(page, stack);
    const pane = stack.panes[0]!;
    await page.locator(`[data-pane-view="${pane}"] .xterm-helper-textarea`).focus();
    // `cat -v` prints what the program reads, ESC as `^[`; with the terminal's
    // echo off only its lines appear. The ready line is computed, so the
    // typed command never matches it, and it is printed once the mode is set.
    const run = async (mode: "h" | "l", marker: string): Promise<string> => {
      await page.keyboard.type(`stty -echo; printf '\\033[?2004${mode}'; echo READY_${marker}_$((40+2)); cat -v\n`);
      await expect.poll(() => paneText(page, pane), { timeout: 20_000 }).toContain(`READY_${marker}_42`); // wait-ok: the pane's shell ran the typed command and printed its ready line
      await paste(page, pane, `${marker}_ONE\n${marker}_TWO`);
      await page.keyboard.press("Enter");
      await expect.poll(() => paneText(page, pane)).toMatch(new RegExp(`${marker}_TWO`));
      const read = (await paneText(page, pane)).split(`READY_${marker}_42`).pop()!;
      await page.keyboard.press("Control+C");
      return read;
    };
    const bracketed = await run("h", "BRACKETED");
    // One paste, its two lines apart: the line break stayed a line break.
    expect(bracketed).toMatch(/\^\[\[200~BRACKETED_ONE\s*\n\s*BRACKETED_TWO\^\[\[201~/);
    const plain = await run("l", "PLAIN");
    expect(plain).toMatch(/PLAIN_ONE\s*\n\s*PLAIN_TWO/);
    expect(plain).not.toContain("^[[20");
  } finally {
    stack.stop();
  }
});
