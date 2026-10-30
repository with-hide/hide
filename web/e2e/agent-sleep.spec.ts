// Agent sleep on an isolated pinned Herdr and hided (PRD agent-sleep B2, B3,
// B10-B12, B15, B16): Sleep agent from the pane menu ends the agent and
// leaves the pane's shell, the row keeps its place with the moon, the pane
// shows Sleeping instead of its terminal, Settings counts it, and a visit
// back to its tab starts the same provider in the same pane with the
// conversation's resume arguments.

import { expect, test } from "@playwright/test";
import path from "node:path";
import { claudeProjects, setFixtureSession, startHerdr, writeFixtureTranscript } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { countSent, screenshot } from "./wire";
import { chord } from "./chords";

test.describe.configure({ timeout: 150_000 });

const SESSION = "11111111-2222-3333-4444-555555555555";

type ProcessInfo = {
  result: { process_info: { shell_pid: number; foreground_process_group_id: number; foreground_processes: { argv?: string[] | null }[] } };
};

test("an agent slept from the pane menu keeps its row and wakes in the same pane on a visit", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const herdr = await startHerdr();
  let daemon: Daemon | null = null;
  try {
    const [, sleeper] = herdr.panes;
    // The official Claude integration's report: the conversation id, under
    // Herdr's own source.
    writeFixtureTranscript(claudeProjects(herdr), SESSION, { task: "Agent two" });
    setFixtureSession(herdr, sleeper, SESSION);
    const other = herdr.run([
      "tab", "create", "--workspace", herdr.workspace, "--cwd", path.join(herdr.root, "fixture"), "--label", "other", "--env", `PATH=${herdr.fixturePath}`, "--no-focus",
    ]) as { result: { tab: { tab_id: string } } };
    const processInfo = () => (herdr.run(["pane", "process-info", "--pane", sleeper]) as ProcessInfo).result.process_info;

    daemon = await startHided(herdr, "agent-sleep");
    const last = new Map<string, Record<string, unknown>>();
    countSent(page, last);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await page.locator("[data-checkout]").first().click();
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
    // The body says it once; nothing waits on the operator, so no turn on the head or over the terminal.
    await expect(pane.locator("[data-pane-header-band], [data-pane-head-turn]")).toHaveCount(0);
    const slept = processInfo();
    expect(slept.foreground_process_group_id).toBe(slept.shell_pid);

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
    await expect(choice).toHaveAttribute("data-agent-sleep-after", "never");
    await choice.click();
    await page.locator('[data-agent-sleep-option="24"]').click();
    await expect(choice).toHaveAttribute("data-agent-sleep-after", "24", { timeout: 10_000 });
    await screenshot(page, "agent-sleep-settings");
    await page.keyboard.press("Escape");
    await expect(page.locator('[data-settings="true"]')).toHaveCount(0);

    // B12, B16: leaving the tab and coming back wakes it in the same pane,
    // resuming the conversation, and the row is awake again.
    await page.locator(`[data-tab="${other.result.tab.tab_id}"]`).click();
    await expect(page.locator(`[data-pane-view="${sleeper}"]`)).toHaveCount(0);
    await page.locator(`[data-tab="${herdr.tab}"]`).click();
    // Herdr's schema makes argv optional and nullable; a process reported without
    // it must not end the poll before the resumed agent is reported.
    await expect.poll(() => processInfo().foreground_processes.map((process) => process.argv?.join(" ")), { timeout: 30_000 }).toContain(
      `claude --resume ${SESSION}`,
    );
    await expect(pane.locator("[data-pane-sleep]")).toHaveCount(0, { timeout: 30_000 });
    await expect(row.locator('[data-agent-status="sleeping"]')).toHaveCount(0);
    await screenshot(page, "agent-sleep-awake");
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});
