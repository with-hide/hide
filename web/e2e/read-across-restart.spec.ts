// A row the operator read stays read and a Done row nobody read stays Done
// when hided restarts, on either engine. A Herdr server that restarts or
// hands off numbers its panes' states afresh and keeps none of the tokens
// Hide wrote on them, so the core judges a restored pane against what it
// saved, not against Herdr's new numbers (docs/status-model.md, Across a
// Herdr restart); the two tests that restart that server are Herdr's own.
import { expect, test, type Page } from "@playwright/test";
import { startFixture } from "./engine-fixture";
import { elsewhereTab as elsewhereHerdrTab, finishFixtureTurn, liveHandoff, startHerdr, type HerdrFixture } from "./herdr-fixture";
import { elsewhereTab, finishUnseenTurn } from "./fixture-agents";
import { startHided, type Daemon } from "./hided-fixture";

test.describe.configure({ timeout: 150_000 });

const sidebarRow = (page: Page, pane: string) => page.locator(`nav[data-sidebar] [data-pane="${pane}"]`).first();

type PaneRow = { result: { pane: { tokens?: Record<string, string> } } };
/** The mark naming the server the core read `pane` on, as Herdr holds it (`wire::SERVER_MARK_TOKEN`). */
const serverMark = (herdr: HerdrFixture, pane: string) => (herdr.run(["pane", "get", pane]) as PaneRow).result.pane.tokens?.hide_server;

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

/** Two turns finished unseen; the operator reads the first, and the core marks its pane with the server it read it on. */
async function readOneOfTwo(page: Page, label: string, shortRoot: boolean): Promise<{ herdr: HerdrFixture; daemon: Daemon; read: string; unread: string; mark: string }> {
  await page.setViewportSize({ width: 1440, height: 900 });
  const herdr = await startHerdr({ shortRoot });
  const [read, unread] = herdr.panes;
  await finishFixtureTurn(herdr, read, elsewhereHerdrTab(herdr));
  await finishFixtureTurn(herdr, unread, elsewhereHerdrTab(herdr));
  const daemon = await startHided(herdr, label);
  await page.goto(`${daemon.origin}/#token=${daemon.token}`);
  await expectDone(page, read);
  await expectDone(page, unread);
  await sidebarRow(page, read).locator("[data-agent-open]").first().click();
  await expect(page.locator(`[data-pane-view="${read}"]`)).toHaveAttribute("data-focused", "true");
  await expectRead(page, read);
  await expect.poll(() => serverMark(herdr, read), { message: "the core marks the pane it read", timeout: 30_000 }).toBeTruthy();
  return { herdr, daemon, read, unread, mark: serverMark(herdr, read)! };
}

/**
 * The restored server holds no mark, and the core writes its new connection's
 * mark only once it has judged the pane settled there, so a new mark on the
 * pane is the signal that the rows below show the core's verdict.
 */
async function expectJudgedOnTheRestoredServer(page: Page, herdr: HerdrFixture, read: string, unread: string, before: string) {
  await expect
    .poll(() => {
      const now = serverMark(herdr, read);
      return now !== undefined && now !== before;
    }, { message: "the core marks the pane on the restored server", timeout: 60_000 })
    .toBe(true);
  await expectRead(page, read);
  await expectDone(page, unread);
}

// Herdr only: the subject is the Herdr server restarting and the pane tokens it keeps.
test("a read row stays read and an unread Done row stays Done across a Herdr cold restart @herdr", async ({ page }) => {
  const { herdr, daemon, read, unread, mark } = await readOneOfTwo(page, "read-across-restart", false);
  try {
    await herdr.restart();
    await expectJudgedOnTheRestoredServer(page, herdr, read, unread, mark);
  } finally {
    daemon.stop();
    herdr.stop();
  }
});

// Herdr only: the subject is Herdr's live handoff, which an app update runs and which starts the new server with no pane tokens. Windows has no handoff.
test("a read row stays read and an unread Done row stays Done across a Herdr live handoff @herdr", async ({ page }) => {
  test.skip(process.platform === "win32", "Herdr's live handoff is a Unix server feature");
  const { herdr, daemon, read, unread, mark } = await readOneOfTwo(page, "read-across-handoff", true);
  try {
    liveHandoff(herdr);
    await expectJudgedOnTheRestoredServer(page, herdr, read, unread, mark);
  } finally {
    daemon.stop();
    herdr.stop();
  }
});
