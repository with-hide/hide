// Agent sleep on an isolated engine and hided (PRD agent-sleep B2, B3,
// B10-B12, B15, B16): Sleep agent from the pane menu ends the agent and
// leaves the pane's shell, the row keeps its place with the moon, the pane
// shows Sleeping instead of its terminal, Settings counts it, and a visit
// back to its tab starts the same provider in the same pane with the
// conversation's resume arguments.

import { expect, test } from "@playwright/test";
import { startFixture } from "./engine-fixture";
import { agentIn, focusTab, newTab, withControl } from "./daemon-actions";
import { CLAUDE_FULL_ACCESS } from "./pane-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { countSent, enterWorkspace, screenshot } from "./wire";
import { chord } from "./chords";

test.describe.configure({ timeout: 150_000 });

// The session the fixture's second agent runs (`engine-fixture.ts`), whose transcript names it Agent two.
const SESSION = "fixture-two";

test("an agent slept from the pane menu keeps its row and wakes in the same pane on a visit", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const engine = await startFixture();
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(engine, "agent-sleep");
    const [, sleeper] = engine.panes;
    // Each start leaves its line on the pane's screen; whitespace is dropped, since the line wraps at the pane's width.
    const flat = (text: string) => text.replace(/\s+/g, "");
    const resume = flat(`claude --resume ${SESSION} ${CLAUDE_FULL_ACCESS[0]} ${CLAUDE_FULL_ACCESS[1]}`);
    const typedResumes = () => page.evaluate((id) => window.__hideProbe?.paneText(id) ?? "", sleeper).then((text) => flat(text).split(resume).length - 1);
    // A new tab is the one shown; the agents' tab is shown again, with this one beside it.
    const other = await withControl(daemon, async (control) => {
      const made = await newTab(control, engine, "other");
      await focusTab(control, engine, engine.tab);
      return made;
    });
    const last = new Map<string, Record<string, unknown>>();
    countSent(page, last);
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    const pane = page.locator(`[data-pane-view="${sleeper}"]`);
    await expect(pane).toBeVisible({ timeout: 20_000 });
    // Sleep must preserve a name the automatic label worker has produced.
    await expect(page.locator(`nav[data-sidebar] [data-pane="${sleeper}"]`)).toContainText("Agent two");

    // B15: Sleep agent from the pane menu.
    await page.locator(`[data-pane-menu="${sleeper}"]`).click();
    const item = page.locator('[data-menu-item="sleep_agent"]');
    await expect(item).toBeEnabled({ timeout: 20_000 });
    await screenshot(page, "agent-sleep-menu");
    await item.click();
    await expect.poll(() => last.get("agent_sleep")?.pane_id).toBe(sleeper);

    // B7, B11: the agent ended and its shell holds the pane; the pane says
    // Sleeping with Wake agent instead of showing that shell.
    await expect(pane.locator('[data-pane-sleep="sleeping"]')).toBeVisible({ timeout: 30_000 });
    await expect(pane.locator(`[data-agent-wake="${sleeper}"]`)).toBeVisible();
    // The body says it once; nothing waits on the operator, so no second row.
    await expect(pane.locator("[data-pane-header-band]")).toHaveCount(0);
    // The shell holds the terminal again: its prompt is the last thing on the pane's screen, below the line that started the agent.
    await expect.poll(() => page.evaluate((id) => (window.__hideProbe?.paneText(id) ?? "").trimEnd().split("\n").pop() ?? "", sleeper), { timeout: 20_000 }).toMatch(/fixture %$/);

    // B10: the row keeps its name with the moon.
    const row = page.locator(`nav[data-sidebar] [data-pane="${sleeper}"]`);
    await expect(row.locator('[data-agent-status="sleeping"]')).toBeVisible({ timeout: 20_000 });
    await expect(row).toContainText("Agent two");
    // The row names its agent by product and draws no provider logo (agent-mark-quiet B1, B7).
    await expect(row.locator("[data-agent-mark]")).toHaveCount(0);
    await expect(row.locator("[data-agent-open]")).toHaveAttribute("aria-label", /Claude Code/);
    await screenshot(page, "agent-sleep-sleeping");

    // B2, B3: Settings > Performance counts it and keeps the choice.
    await page.keyboard.press(chord("settings"));
    await page.locator('[data-settings-tab="agents"]').click();
    await expect(page.locator('[data-settings-group="idle-agents"]')).toContainText("1 sleeping now");
    const choice = page.locator("[data-agent-sleep-after]");
    // 24 hours until the operator chooses (the default), and 48 once chosen.
    await expect(choice).toHaveAttribute("data-agent-sleep-after", "24");
    await choice.click();
    await expect(page.locator('[data-agent-sleep-option="48"]')).toBeVisible();
    await screenshot(page, "agent-sleep-choices");
    await page.locator('[data-agent-sleep-option="48"]').click();
    await expect(choice).toHaveAttribute("data-agent-sleep-after", "48", { timeout: 10_000 });
    await screenshot(page, "agent-sleep-settings");
    await page.keyboard.press("Escape");
    await expect(page.locator('[data-settings="true"]')).toHaveCount(0);

    // B12, B16: leaving the tab and coming back wakes it in the same pane,
    // resuming the conversation, and the row is awake again.
    const typedBefore = await typedResumes();
    await page.locator(`[data-tab="${other.tab}"]`).click();
    await expect(page.locator(`[data-pane-view="${sleeper}"]`)).toHaveCount(0);
    await page.locator(`[data-tab="${engine.tab}"]`).click();
    // The wake types one more resume line at the pane's shell, and the agent is listed in the pane again.
    await expect.poll(typedResumes, { timeout: 30_000 }).toBeGreaterThan(typedBefore);
    await withControl(daemon, (control) => control.until((rest) => (agentIn(rest, sleeper)?.session_id === SESSION ? true : undefined), `the resumed agent in ${sleeper}`, 30_000));
    await expect(pane.locator("[data-pane-sleep]")).toHaveCount(0, { timeout: 30_000 });
    await expect(row.locator('[data-agent-status="sleeping"]')).toHaveCount(0);
    await screenshot(page, "agent-sleep-awake");
  } finally {
    daemon?.stop();
    engine.stop();
  }
});
