// with-hide/hide issue 2952: a paste into a pane, text or an image's path,
// reaches its program in bracketed paste exactly when the program turned that
// mode on. The page never sees the mode (the holder's screen fragments do not
// carry it), so the pane's holder wraps the paste: the browser to hided, the
// core for an image, the node, and the holder to the PTY.
import { expect, test, type Page } from "@playwright/test";
import { openLayout, startLayoutStack } from "./engine-fixture";

test.describe.configure({ timeout: 90_000 }); // wait-ok: a hided, its engine and a pane's shell started and stopped whole, with two commands typed and read

const paneText = (page: Page, pane: string) => page.evaluate((id) => window.__hideProbe?.paneText(id) ?? "", pane);

/** One pixel, a real PNG for the core to read as a clipboard image. */
const PIXEL = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4z8AAAAMBAQDJ/pLvAAAAAElFTkSuQmCC";

/** A paste on the pane's terminal, of `text` or of a PNG image, as ⌘V or the Edit menu's Paste delivers it. */
const paste = (page: Page, pane: string, text: string | { png: string }) => page.evaluate(({ pane, text }) => {
  const data = new DataTransfer();
  if (typeof text === "string") data.setData("text/plain", text);
  else data.items.add(new File([Uint8Array.from(atob(text.png), (c) => c.charCodeAt(0))], "shot.png", { type: "image/png" }));
  document.querySelector(`[data-pane-view="${pane}"] .xterm-helper-textarea`)!
    .dispatchEvent(new ClipboardEvent("paste", { clipboardData: data, bubbles: true, cancelable: true }));
}, { pane, text });

test("a pasted text or image path is bracketed only for a program that turned bracketed paste on", async ({ page }) => {
  const stack = await startLayoutStack({ panes: 1, label: "terminal-paste" });
  try {
    await openLayout(page, stack);
    const pane = stack.panes[0]!;
    await page.locator(`[data-pane-view="${pane}"] .xterm-helper-textarea`).focus();
    // `cat -v` prints what the program reads, ESC as `^[`, as it arrives;
    // with the terminal's echo off only its output appears. The ready line is
    // computed, so the typed command never matches it, and it is printed once
    // the mode is set.
    const run = async (mode: "h" | "l", marker: string): Promise<string> => {
      await page.keyboard.type(`stty -echo -icanon; printf '\\033[?2004${mode}'; echo READY_${marker}_$((40+2)); cat -v\n`);
      await expect.poll(() => paneText(page, pane), { timeout: 20_000 }).toContain(`READY_${marker}_42`); // wait-ok: the pane's shell ran the typed command and printed its ready line
      await paste(page, pane, `${marker}_ONE\n${marker}_TWO`);
      await page.keyboard.press("Enter");
      await expect.poll(() => paneText(page, pane)).toMatch(new RegExp(`${marker}_TWO`));
      // An image goes up as an attachment, and its staged path comes back to
      // the pane as a paste of its own with a space after it; a key typed
      // then lands behind that space.
      await paste(page, pane, { png: PIXEL });
      const after = () => paneText(page, pane).then((text) => text.split(`${marker}_TWO`).pop()!);
      await expect.poll(after, { timeout: 20_000 }).toMatch(/\.png"/); // wait-ok: the core read the staged image and the pane's node pasted its path
      await page.keyboard.type("Z");
      await expect.poll(after).toMatch(/Z/);
      const read = (await paneText(page, pane)).split(`READY_${marker}_42`).pop()!;
      await page.keyboard.press("Control+C");
      return read;
    };
    const bracketed = await run("h", "BRACKETED");
    // One paste, its two lines apart: the line break stayed a line break.
    expect(bracketed).toMatch(/\^\[\[200~BRACKETED_ONE\s*\n\s*BRACKETED_TWO\^\[\[201~/);
    expect(bracketed).toMatch(/\^\[\[200~"[^"]+\.png"\^\[\[201~ Z/);
    const plain = await run("l", "PLAIN");
    expect(plain).toMatch(/PLAIN_ONE\s*\n\s*PLAIN_TWO/);
    expect(plain).toMatch(/"[^"]+\.png" Z/);
    expect(plain).not.toContain("^[[20");
  } finally {
    stack.stop();
  }
});
