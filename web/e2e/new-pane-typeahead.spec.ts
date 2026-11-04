// Keys typed into a split before its shell's line editor reads (PRD
// instant-pane-topology D-11): the core holds them until the new pane's shell
// reads keys itself, so the kernel's line mode never echoes them or erases a
// multi-byte character one byte at a time. The new shell is held at a gate in
// its `.zshrc` until the keys are typed, so the order is the test's, not the
// scheduler's (docs/TESTING.md, The test decides the order).

import { expect, test, type Page } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { chord } from "./chords";
import { startFixture } from "./engine-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { enterWorkspace } from "./wire";

/** What `pane` shows now, as the page draws it. */
function visible(page: Page, pane: string): Promise<string> {
  return page.evaluate((id) => window.__hideProbe!.paneText(id), pane);
}

test("keys typed into a split before its shell reads arrive whole", async ({ page }) => {
  const herdr = await startFixture({ agents: false });
  let daemon: Daemon | undefined;
  try {
    daemon = await startHided(herdr, "new-pane-typeahead");
    // A shell started from now on waits in its startup file until `release`
    // exists, with its terminal still in the kernel's line mode. The
    // fixture's own two shells have read their startup file by now.
    const home = path.join(herdr.root, "home");
    const release = path.join(home, "release");
    const rc = path.join(home, ".zshrc");
    fs.writeFileSync(rc, `while [[ ! -e ${release} ]]; do sleep 0.05; done\n${fs.readFileSync(rc, "utf8")}`);

    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    const [first] = herdr.panes;
    const origin = page.locator(`[data-pane-view="${first}"]`);
    await expect(origin).toHaveAttribute("data-transport", "controlling");
    await origin.locator(".xterm-helper-textarea").focus();
    const paneIds = () => page.locator("[data-pane-view]").evaluateAll((views) => views.map((view) => view.getAttribute("data-pane-view")!));
    const before = await paneIds();

    // Split, then type at once: a syllable typed with the Korean input
    // source on, erased, and the command meant all along.
    await page.keyboard.press(chord("split_right"));
    await page.keyboard.insertText("ㅊ");
    await page.keyboard.press("Backspace");
    await page.keyboard.press("Backspace");
    await page.keyboard.type("cla");
    await expect(page.locator("[data-pane-view]")).toHaveCount(before.length + 1);
    const created = (await paneIds()).filter((id) => !before.includes(id));
    expect(created).toHaveLength(1);
    const split = created[0]!;
    // The shell is released once the keys' fate is decided while it is still
    // held: the core lists their request as waiting for the new pane's shell,
    // or the keys already reached its terminal in the kernel's line mode,
    // which echoes them. Both are seen soon after the engine names the pane, well
    // inside the wait's two seconds, and neither depends on when the pane's
    // terminal session opens.
    const waitsForShell = () =>
      page.evaluate((pane) => window.__hideProbe!.inputRequests().some((request) => request.state === "pending" && request.held_for === pane), split);
    await expect.poll(async () => (await waitsForShell()) || (await visible(page, split)).includes("cla"), { timeout: 20_000 }).toBe(true);
    fs.writeFileSync(release, "");

    // The line editor took the keys: the prompt holds the command, with no
    // broken character and no echo of it on a line of its own above.
    await expect.poll(() => visible(page, split), { timeout: 20_000 }).toMatch(/^fixture % cla\s*$/m);
    const screen = await visible(page, split);
    expect(screen).not.toContain("<ffffffff>");
    expect(screen.split("\n").filter((line) => line.includes("cla"))).toEqual([expect.stringMatching(/^fixture % cla\s*$/)]);
  } finally {
    try {
      daemon?.stop();
    } finally {
      herdr.stop();
    }
  }
});
