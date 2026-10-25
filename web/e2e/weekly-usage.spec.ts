// The weekly usage chips and popover on an isolated pinned Herdr and hided
// (#153). The daemon's PATH carries a `claude` that answers `/usage` with a
// fixed reading once the test releases it, so its row is first loading and
// then available with a Fable bucket; this HOME has no Codex login, so that
// row is unavailable. Along the way the page tells the core when the popover
// opens and closes and when the page is hidden.

import { expect, test, type Page } from "@playwright/test";
import { startHerdr } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { usageShim } from "./usage-fixture";
import { screenshot } from "./wire";

test.describe.configure({ timeout: 120_000 });

type Hints = { usage_window_visible?: boolean; usage_popover_open?: boolean };

/** Every usage hint the page sent, in order, read off its `ui_state_update` events. */
function sentHints(page: Page): Hints[] {
  const hints: Hints[] = [];
  page.on("websocket", (ws) =>
    ws.on("framesent", (frame) => {
      try {
        const event = JSON.parse(String(frame.payload)) as { kind?: string; payload?: Hints };
        if (event.kind !== "ui_state_update" || !event.payload) return;
        const { usage_window_visible, usage_popover_open } = event.payload;
        if (usage_window_visible !== undefined || usage_popover_open !== undefined) hints.push({ usage_window_visible, usage_popover_open });
      } catch {
        /* the handshake is not an event */
      }
    }),
  );
  return hints;
}

// @platform: The usage child gets a restricted environment that differs per OS (USERPROFILE and SystemRoot on Windows), and a Node-based `claude` only starts with it.
test("weekly usage: a loading, an available and an unavailable row, and the hints the page sends", { tag: "@platform" }, async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const herdr = await startHerdr({ readiness: "registered" });
  const usage = usageShim();
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(herdr, "usage", undefined, { PATH: usage.path, CODEX_HOME: undefined });
    const hints = sentHints(page);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);

    // Claude's read is held open, so its row is still loading; Codex has no login here.
    const trigger = page.locator('[data-usage-trigger][data-usage-chip="claude"]');
    const claudeChip = page.locator('[data-usage-chip="claude"]');
    const codexChip = page.locator('[data-usage-chip="codex"]');
    await expect(codexChip).toHaveAttribute("data-usage-state", "unavailable", { timeout: 20_000 });
    await expect(claudeChip).toHaveAttribute("data-usage-state", "loading");
    await expect(claudeChip).toHaveText("");
    await expect(codexChip).toHaveText("");
    await expect(trigger).toHaveAccessibleName("Claude Code, …");
    await expect.poll(() => hints.some((hint) => hint.usage_window_visible === true)).toBe(true);

    await trigger.click();
    const popover = page.locator("[data-usage-popover]");
    await expect(popover.getByText("Usage", { exact: true })).toBeVisible();
    const claudeRow = popover.locator('[data-usage-row="claude"]:not([data-usage-bucket])');
    const codexRow = popover.locator('[data-usage-row="codex"]');
    await expect(claudeRow.locator("[data-usage-value]")).toHaveText("…");
    await expect(codexRow.locator("[data-usage-value]")).toHaveText("Unavailable");
    await expect(codexRow).toContainText("Signed out · run codex to sign in");
    await expect.poll(() => hints.at(-1)).toEqual({ usage_popover_open: true });
    await screenshot(page, "usage-loading-unavailable");

    // Claude answers: the open popover fills in the row, its reset, and the Fable bucket under it.
    usage.release();
    const week = claudeRow.locator('[data-usage-window-row="weekly"]:not([data-usage-bucket])');
    await expect(week.locator("[data-usage-value]")).toHaveText("62%", { timeout: 20_000 });
    await expect(week.locator("[data-usage-reset]")).toHaveText(/^· in 5d [45]h$/);
    const fable = claudeRow.locator('[data-usage-bucket="Fable"]');
    await expect(fable.locator("[data-usage-value]")).toHaveText("6%");
    await expect(fable.locator("[data-usage-reset]")).toHaveText(/^· in 5d [45]h$/);
    await screenshot(page, "usage-popover-available");

    await page.keyboard.press("Escape");
    await expect(popover).toHaveCount(0);
    await expect.poll(() => hints.at(-1)).toEqual({ usage_popover_open: false });
    await expect(claudeChip).toHaveText("62%");
    await expect(trigger).toHaveAccessibleName("Claude Code, 7 days, 62%");
    await screenshot(page, "usage-chips");

    await trigger.click();
    await popover.getByRole("button", { name: "Choose sidebar items" }).click();
    const toggle = page.getByRole("switch", { name: "Claude Code 5 hours" });
    await toggle.click();
    const second = await page.context().newPage();
    await second.goto(`${daemon.origin}/#token=${daemon.token}`);
    await expect(second.locator('[data-usage-chip="claude"]')).toHaveCount(2);
    await expect(second.locator('[data-usage-chip="claude"][data-usage-window="five_hour"]')).toContainText("4%");
    await page.getByRole("button", { name: "Hide Claude Code" }).click();
    await expect(second.locator('[data-usage-chip="claude"]')).toHaveCount(0);
    await page.getByRole("button", { name: "Show Claude Code" }).click();
    await expect(second.locator('[data-usage-chip="claude"]')).toHaveCount(2);
    await second.close();

    // A hidden page tells the core it is no longer looking.
    await page.evaluate(() => {
      Object.defineProperty(document, "visibilityState", { configurable: true, get: () => "hidden" });
      document.dispatchEvent(new Event("visibilitychange"));
    });
    await expect.poll(() => hints.at(-1)).toEqual({ usage_window_visible: false });
  } finally {
    daemon?.stop();
    herdr.stop();
    usage.cleanup();
  }
});
