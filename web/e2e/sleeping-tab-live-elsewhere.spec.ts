// A sleeping agent's tab whose conversation already runs in another pane
// (with-hide/hide#2590): the visit does not resume a second copy of it. The
// screen goes to the pane that runs it, and the sleeping pane is left to its
// shell, as an agent that ended there leaves it.

import fs from "node:fs";
import path from "node:path";
import { expect, test } from "@playwright/test";
import { toPage } from "../../desktop/src/main/wirePath";
import { startFixture } from "./engine-fixture";
import { agentIn, corePaneFocus, shownTab, startAgentInCheckout, tabOfPane, withControl, type AgentRow } from "./daemon-actions";
import { CLAUDE_FULL_ACCESS } from "./pane-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { enterWorkspace, screenshot } from "./wire";

test.describe.configure({ timeout: 150_000 });

// The session the fixture's second agent runs (`engine-fixture.ts`).
const SESSION = "fixture-two";

test("a visit to a sleeping tab whose conversation runs in another pane goes to that pane", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const engine = await startFixture();
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(engine, "sleeping-tab-live-elsewhere");
    const [, sleeper] = engine.panes;
    const flat = (text: string) => text.replace(/\s+/g, "");
    const resume = flat(`claude --resume ${SESSION} ${CLAUDE_FULL_ACCESS[0]} ${CLAUDE_FULL_ACCESS[1]}`);
    const typedResumes = () => page.evaluate((id) => window.__hideProbe?.paneText(id) ?? "", sleeper).then((text) => flat(text).split(resume).length - 1);
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    const pane = page.locator(`[data-pane-view="${sleeper}"]`);
    await expect(pane).toBeVisible({ timeout: 20_000 });

    await page.locator(`[data-pane-menu="${sleeper}"]`).click();
    const item = page.locator('[data-menu-item="sleep_agent"]');
    await expect(item).toBeEnabled({ timeout: 20_000 });
    await item.click();
    await expect(pane.locator('[data-pane-sleep="sleeping"]')).toBeVisible({ timeout: 30_000 });
    const typedBefore = await typedResumes();

    // The same conversation resumed in a tab of its own, as Take over or a
    // `claude --resume` typed in another tab does.
    const folder = toPage(fs.realpathSync.native(path.join(engine.root, "fixture")));
    const live = await withControl(daemon, async (control) => {
      const started = await startAgentInCheckout(control, folder, { resume_session_id: SESSION }, "the same conversation in another tab");
      await control.until((rest) => (agentIn(rest, started)?.session_id === SESSION && shownTab(rest) === tabOfPane(rest, started)) || undefined, `${started} on ${SESSION}, shown`, 30_000);
      return started;
    });
    await screenshot(page, "sleeping-tab-live-elsewhere-before");

    // The operator clicks the sleeping agent's tab.
    await page.locator(`[data-tab="${engine.tab}"]`).click();

    // The screen goes to the pane that runs the conversation, and the
    // sleeping pane holds no agent and no sleeping body any more.
    await withControl(daemon, (control) => control.until((rest) => (corePaneFocus(rest) === live && shownTab(rest) === tabOfPane(rest, live)) || undefined, `the focus on ${live}`, 30_000));
    await expect(page.locator(`[data-pane-view="${live}"]`)).toBeVisible({ timeout: 20_000 });
    await expect(page.locator(`[data-pane-view="${sleeper}"]`)).toHaveCount(0);
    await withControl(daemon, (control) => control.until((rest) => {
      const rows = ((rest.navigator as { agents?: AgentRow[] } | undefined)?.agents ?? []).filter((row) => row.session_id === SESSION);
      return rows.length === 1 && rows[0].pane_id === live && !agentIn(rest, sleeper) ? true : undefined;
    }, `one agent on ${SESSION}, in ${live}`, 30_000));
    await screenshot(page, "sleeping-tab-live-elsewhere-after");

    // A second visit finds the pane's shell, with nothing left to wake.
    await page.locator(`[data-tab="${engine.tab}"]`).click();
    await expect(pane).toBeVisible({ timeout: 20_000 });
    await expect(pane.locator("[data-pane-sleep]")).toHaveCount(0);
    expect(await typedResumes()).toBe(typedBefore);
  } finally {
    daemon?.stop();
    engine.stop();
  }
});
