// A row the operator read stays read and a Done row nobody read stays Done
// when hided restarts, on either engine. A Herdr server that restarts or
// hands off numbers its panes' states afresh and keeps none of the tokens
// Hide wrote on them, so the core judges a restored pane against what it
// saved, not against Herdr's new numbers (docs/status-model.md, Across a
// Herdr restart); the two tests that restart that server are Herdr's own.
import { expect, test, type Page } from "@playwright/test";
import { startFixture } from "./engine-fixture";
import { elsewhereTab, finishUnseenTurn } from "./fixture-agents";
import { startHided, type Daemon } from "./hided-fixture";

test.describe.configure({ timeout: 150_000 });

const sidebarRow = (page: Page, pane: string) => page.locator(`nav[data-sidebar] [data-pane="${pane}"]`).first();

/** Read: the row asks for nothing (`data-attention`), and its finished turn is quiet. */
async function expectRead(page: Page, pane: string) {
  await expect(sidebarRow(page, pane)).toHaveAttribute("data-attention", "false");
  await expect(sidebarRow(page, pane).locator('[data-agent-status-mark="quiet"]')).toBeVisible();
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
    await expectRead(page, read);

    daemon = await daemon.restart();
    // Reopening is a fresh page, not a reconnect of this one.
    await page.goto("about:blank");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await expectRead(page, read);
    await expectDone(page, unread);
  } finally {
    daemon?.stop();
    fixture.stop();
  }
});
