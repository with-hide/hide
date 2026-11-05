// B1-B7/B14: real cold restoration and one Hide-triggered terminal split.
import { expect, test, type Page } from "@playwright/test";
import path from "node:path";
import { paneCommandLine } from "./pane-fixture";
import { waitForTerminal } from "./group-start-helper";
import { corePaneFocus, coreTerminalPane, shownTab, tabLayout, tabPanes, tabsOf, withControl } from "./daemon-actions";
import { dumpOnFailure } from "./failure-dump";
import { startFixture } from "./engine-fixture";
import { fixtureProgram } from "./platform-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { chord } from "./chords";
import { countSent, enterWorkspace, screenshot } from "./wire";

test.describe.configure({ timeout: 120_000 });

/** Proves typed input executes in the pane's normal shell: the page types a fixture program's line and the pane shows what it printed. */
async function terminalRuns(page: Page, fixture: { root: string; shell: Parameters<typeof paneCommandLine>[0] }, pane: string): Promise<void> {
  const marker = "HIDE_TERMINAL_EXECUTED";
  const executable = fixtureProgram(path.join(fixture.root, "bin"), "hide-terminal-check", `console.log(${JSON.stringify(marker)});`);
  const view = page.locator(`[data-pane-view="${pane}"]`);
  await expect(view).toHaveAttribute("data-transport", "controlling");
  await view.locator("textarea").focus();
  await page.keyboard.type(paneCommandLine(fixture.shell, [executable]));
  await page.keyboard.press("Enter");
  await expect.poll(() => page.evaluate((id) => window.__hideProbe?.paneText(id), pane), { timeout: 5_000 }).toContain(marker);
}

test("Opening a new terminal and splitting it keeps one tab and both terminal identities", async ({ page }) => {
  const fixture = await startFixture({ agents: false });
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(fixture, "coexistence-split");
    const live = daemon;
    const read = <T,>(pick: (rest: Parameters<typeof tabsOf>[0]) => T | undefined, what: string) => withControl(live, (control) => control.until(pick, what, 20_000));
    const sent = countSent(page);
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
    await enterWorkspace(page);
    const tabsBefore = await read((rest) => tabsOf(rest, fixture.workspace), "the tabs of the checkout");
    await page.locator("[data-new-agent-tab]").first().click();
    const tab = await read((rest) => tabsOf(rest, fixture.workspace)?.find((id) => !tabsBefore.includes(id)), "a new tab in the checkout");
    await waitForTerminal(page, tab);
    await expect(page.locator("[data-pane-view]")).toHaveCount(1);
    const first = await page.locator("[data-pane-view]").getAttribute("data-pane-view");
    await terminalRuns(page, fixture, first!);
    await page.keyboard.press(chord("split_right"));
    // A split the page could not place sends nothing and says why only in
    // its own notes; they and the core's focus go with the failure
    // (with-hide/hide-private#476).
    await dumpOnFailure("split", async () => ({
      page: await page.evaluate(() => window.__hideProbe?.diagnostics().slice(-5)),
      core: await read((rest) => ({ terminal: coreTerminalPane(rest), focused: corePaneFocus(rest), shown: shownTab(rest) }), "the core's focus"),
    }), () => expect(page.locator("[data-pane-view]")).toHaveCount(2));
    await expect.poll(() => sent.get("create_pane")).toBe(1);
    await expect(page.locator("[data-canvas]")).toHaveAttribute("data-canvas", tab);
    const ids = await page.locator("[data-pane-view]").evaluateAll((nodes) => nodes.map((node) => node.getAttribute("data-pane-view")!).sort());
    expect(ids).toContain(first);
    // The core lists the same two panes in the tab, split to the right at half.
    const listed = await read((rest) => { const panes = tabPanes(rest, tab); return panes.length === 2 ? [...panes].sort() : undefined; }, "two panes in the new tab");
    expect(listed).toEqual(ids);
    const split = await read((rest) => { const root = tabLayout(rest, tab)?.root as { type: string; direction?: string; ratio?: number } | undefined; return root?.type === "split" ? root : undefined; }, "the tab's split");
    expect(split).toEqual(expect.objectContaining({ direction: "right", ratio: 0.5 }));
    const pane = ids.find((id) => id !== first)!;
    await expect(page.locator(`[data-pane-view="${pane}"]`)).toHaveAttribute("data-focused", "true");
    await terminalRuns(page, fixture, pane);
    await screenshot(page, "coexistence-new-tab-two-panes");
  } finally { daemon?.stop(); fixture.stop(); }
});
