// A row the operator read stays read and a Done row nobody read stays Done
// when hided restarts. The core's own engine numbers its agents' states
// afresh with the daemon, so the core judges a restored pane against what it
// saved, not against the engine's new numbers (docs/status-model.md).
import { expect, test, type Page } from "@playwright/test";
import { agentIn, withControl } from "./daemon-actions";
import { startFixture } from "./engine-fixture";
import { dumpOnFailure } from "./failure-dump";
import { elsewhereTab, finishUnseenTurn } from "./fixture-agents";
import { startHided, type Daemon } from "./hided-fixture";

test.describe.configure({ timeout: 150_000 });

const sidebarRow = (page: Page, pane: string) => page.locator(`nav[data-sidebar] [data-pane="${pane}"]`).first();

/**
 * Both sides of a read row that never drew: the mark, word and sleep record the
 * page drew for it, and what the core published for that pane, so a run that
 * cannot be reproduced says whether the row went unread or asleep instead
 * (with-hide/hide-private#824).
 */
async function bothSides(page: Page, daemon: Daemon | null, pane: string): Promise<unknown> {
  const drawn = await sidebarRow(page, pane).evaluate((row) => ({
    mark: row.querySelector("[data-agent-status-mark]")?.getAttribute("data-agent-status-mark") ?? null,
    attention: row.getAttribute("data-attention"),
    text: (row.textContent ?? "").trim(),
  }));
  const published = daemon
    ? await withControl(daemon, (control) => control.until((rest) => ({
      row: agentIn(rest, pane) ?? null,
      sleepAfterHours: (((rest as Record<string, unknown>).ui_state as { agent_sleep_after_hours?: number | null } | undefined)?.agent_sleep_after_hours) ?? null,
    }), `${pane}'s published row`)).catch((error: unknown) => ({ unavailable: String(error) }))
    : { unavailable: "no daemon" };
  return { pane, drawn, published };
}

/** Read: the row asks for nothing (`data-attention`), and its finished turn is quiet. */
async function expectRead(page: Page, pane: string, daemon: Daemon | null) {
  await expect(sidebarRow(page, pane)).toHaveAttribute("data-attention", "false");
  await dumpOnFailure(`${pane} read row`, () => bothSides(page, daemon, pane), () =>
    expect(sidebarRow(page, pane).locator('[data-agent-status-mark="quiet"]')).toBeVisible());
}

/** Unread: an unread finished turn (a dot; green only on a label's finished verdict). */
async function expectDone(page: Page, pane: string) {
  await expect(sidebarRow(page, pane).locator('[data-agent-status-mark="unread"], [data-agent-status-mark="done"]').first()).toBeVisible({ timeout: 30_000 });
  await expect(sidebarRow(page, pane)).toHaveAttribute("data-attention", "true");
}

test("a read row stays read and an unread Done row stays Done across a restart of hided", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const fixture = await startFixture();
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(fixture, "read-across-restart");
    const [read, unread] = fixture.panes;
    const elsewhere = await elsewhereTab(fixture, daemon);
    await finishUnseenTurn(fixture, daemon, read, elsewhere);
    await finishUnseenTurn(fixture, daemon, unread, elsewhere);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await expectDone(page, read);
    await expectDone(page, unread);
    await sidebarRow(page, read).locator("[data-agent-open]").first().click();
    await expect(page.locator(`[data-pane-view="${read}"]`)).toHaveAttribute("data-focused", "true");
    await expectRead(page, read, daemon);

    daemon = await daemon.restart();
    // Reopening is a fresh page, not a reconnect of this one.
    await page.goto("about:blank");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await expectRead(page, read, daemon);
    await expectDone(page, unread);
  } finally {
    daemon?.stop();
    fixture.stop();
  }
});
