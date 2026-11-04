// B1-B7/B14: real cold restoration and one Hide-triggered terminal split.
import { expect, test, type Page } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { toPage } from "../../desktop/src/main/wirePath";
import { paneCommandLine, startHerdr, type HerdrFixture } from "./herdr-fixture";
import { waitForTerminal } from "./group-start-helper";
import { tabLayout, tabPanes, tabsOf, withControl } from "./daemon-actions";
import { startFixture } from "./engine-fixture";
import { fixtureProgram } from "./platform-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { chord } from "./chords";
import { countSent, enterWorkspace, screenshot } from "./wire";

type Snapshot = {
  panes: { pane_id: string; tab_id: string; terminal_id: string; restore_error?: string }[];
  layouts: { tab_id: string; splits: { direction: string; ratio: number }[] }[];
};
const snapshot = (herdr: HerdrFixture) => (herdr.run(["api", "snapshot"]) as { result: { snapshot: Snapshot } }).result.snapshot;
const listsAgent = (herdr: HerdrFixture, pane: string) =>
  (herdr.run(["agent", "list"]) as { result: { agents: { pane_id: string }[] } }).result.agents.some((agent) => agent.pane_id === pane);
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
    await expect(page.locator("[data-pane-view]")).toHaveCount(2);
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

// Herdr only: it restarts Herdr's server with the folder gone and reads Herdr's cold restore, its `restore_error` and the panes it keeps.
test("a missing-folder cold restore shows failure without an agent and closes only after Herdr @herdr", async ({ page }) => {
  const herdr = await startHerdr({ agents: false });
  let daemon: Daemon | null = null;
  try {
    const folder = path.join(herdr.root, "fixture");
    await herdr.restart(() => fs.renameSync(folder, `${folder}-moved`));
    await expect.poll(() => snapshot(herdr).panes.filter((pane) => pane.restore_error).length).toBe(2);
    daemon = await startHided(herdr, "coexistence-restore");
    await page.goto("about:blank");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page);
    const failed = page.locator("[data-pane-restore-failed]");
    await expect(failed).toHaveCount(2);
    await expect(failed.first()).toContainText(toPage(folder));
    await expect(page.locator("[data-agent-wake]")).toHaveCount(0);
    await screenshot(page, "coexistence-restoration-failed");
    const pane = await failed.first().getAttribute("data-pane-restore-failed");
    await failed.first().getByRole("button", { name: "Close pane", exact: true }).click();
    await expect.poll(() => snapshot(herdr).panes.some((item) => item.pane_id === pane)).toBe(false);
    await expect(failed).toHaveCount(1);
    await screenshot(page, "coexistence-restoration-peer-preserved");
  } finally { daemon?.stop(); herdr.stop(); }
});

// Herdr only: it restarts Herdr's server over a moved folder and reads Herdr's own agent list around the restore.
test("cold restoration failure retains a sleeping row and visiting it does not wake the agent @herdr", async ({ page }) => {
  const herdr = await startHerdr();
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(herdr, "coexistence-sleep");
    const sent = countSent(page);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page);
    const sleeper = herdr.panes[1];
    const row = page.locator(`nav[data-sidebar] [data-pane="${sleeper}"]`);
    await expect(row).toContainText("Agent two");
    await page.locator(`[data-pane-menu="${sleeper}"]`).click();
    await page.locator('[data-menu-item="sleep_agent"]').click();
    await expect(page.locator('[data-pane-sleep="sleeping"]')).toBeVisible();
    // Hide draws the pane asleep once its shell holds the terminal again, and
    // Herdr drops the agent from its own list only on its next look at the
    // pane: a restart before that saves the agent into the session the cold
    // restore lists again, which is not the state this test restores from.
    await expect.poll(() => listsAgent(herdr, sleeper)).toBe(false);
    const other = herdr.run(["tab", "create", "--workspace", herdr.workspace, "--cwd", path.join(herdr.root, "fixture"), "--label", "other", "--no-focus"]) as { result: { tab: { tab_id: string } } };
    await page.locator(`[data-tab="${other.result.tab.tab_id}"]`).click();
    const folder = path.join(herdr.root, "fixture");
    daemon = await daemon.restart(async () => {
      await herdr.restart(() => fs.renameSync(folder, `${folder}-moved`));
    });
    await page.goto("about:blank");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page);
    await page.locator(`[data-tab="${herdr.tab}"]`).click();
    const failed = page.locator(`[data-pane-restore-failed="${sleeper}"]`);
    await expect(failed).toBeVisible();
    await expect(row).toContainText("Agent two");
    await expect(row.locator('[data-agent-status="sleeping"]')).toBeVisible();
    await expect(page.locator("[data-agent-wake]")).toHaveCount(0);
    expect(sent.get("agent_wake") ?? 0).toBe(0);
    expect(listsAgent(herdr, sleeper)).toBe(false);
    await failed.getByRole("button", { name: "Close pane", exact: true }).click();
    await expect.poll(() => snapshot(herdr).panes.some((pane) => pane.pane_id === sleeper)).toBe(false);
    await expect(row).toHaveCount(0);
  } finally { daemon?.stop(); herdr.stop(); }
});
