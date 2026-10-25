// A turn that ended blocked or unfinished draws its mark on a real daemon
// (PRD agent-blocked-state B1-B4, status-marks B5-B8): the turn badge in Needs
// You, dimmed once read, and a plain dot to read for work left, each with its
// word leading the sentence.
import { expect, test, type Page } from "@playwright/test";
import { herdrGate } from "./herdr-gate";
import { elsewhereTab, finishFixtureTurn, herdrHasFocus, labelAgent, setFixtureLifecycle, startHerdr, type HerdrFixture } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";

test.describe.configure({ timeout: 150_000 });

const sidebarRow = (page: Page, pane: string) => page.locator(`nav[data-sidebar] [data-pane="${pane}"]`).first();

/** A block and an unfinished turn, each finished unseen in one tab, as `startHerdr`'s two panes. */
async function finishTwoTurns(herdr: HerdrFixture, [blocked, unfinished]: [string, string]) {
  labelAgent(herdr, blocked, { task: "디스크 정리 작업", progress: "디스크가 가득 차 멈췄어요", end: "blocked" });
  await finishFixtureTurn(herdr, blocked, elsewhereTab(herdr));
  labelAgent(herdr, unfinished, { task: "설정 화면 스위치 추가", progress: "테스트 3개 남음", end: "unfinished" });
  await finishFixtureTurn(herdr, unfinished, elsewhereTab(herdr));
}

/**
 * Herdr reports its changes to the core in the order it made them, so once
 * `pane` shows working here, every state Herdr held before that change (the
 * seen it gave a tab) has reached the core and been projected.
 */
async function afterNextHerdrChange(herdr: HerdrFixture, page: Page, pane: string) {
  await setFixtureLifecycle(herdr, pane, "working");
  await expect(sidebarRow(page, pane).locator('[data-agent-status="working"]')).toBeVisible({ timeout: 30_000 });
}

/** The row is read: its mark dimmed and it is not among the Needs You rows. */
async function expectRead(page: Page, pane: string, mark: string) {
  await expect(sidebarRow(page, pane).locator(`[data-agent-status="${mark}"]`)).toHaveClass(/opacity-\(--opacity-read-status\)/);
  await expect(page.locator(`[data-raised-group="needs_you"] [data-pane="${pane}"]`)).toHaveCount(0);
}

test("a blocked turn draws the turn mark in Needs You that dims once read, and an unfinished turn a plain dot to read outside it", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const herdr = await startHerdr({ readiness: "registered" });
  let daemon: Daemon | null = null;
  try {
    const [blocked, unfinished] = herdr.panes;
    await finishTwoTurns(herdr, herdr.panes);
    daemon = await startHided(herdr, "blocked-state");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    const row = (pane: string) => sidebarRow(page, pane);
    await expect(row(blocked).locator('[data-agent-status="error"]')).toBeVisible({ timeout: 30_000 });
    await expect(page.locator(`[data-raised-group="needs_you"] [data-pane="${blocked}"]`)).toBeVisible();
    // Work left is a plain dot to read with its own word, never the green finish (status-marks D-03, D-04).
    await expect(row(unfinished).locator('[data-agent-status="unfinished"]')).toHaveAttribute("data-agent-status-mark", "unread");
    await expect(page.locator(`[data-raised-group="needs_you"] [data-pane="${unfinished}"]`)).toHaveCount(0);
    await expect(row(unfinished)).toContainText("Unfinished");
    await row(blocked).locator("[data-agent-open]").first().click();
    await expect(page.locator(`[data-pane-view="${blocked}"]`)).toHaveAttribute("data-focused", "true");
    // Read (B2): the block leaves Needs You for Seen, its turn mark dims and its cause stays.
    const mark = row(blocked).locator('[data-agent-status="error"]');
    await expect(mark).toHaveClass(/opacity-\(--opacity-read-status\)/);
    await expect(page.locator(`[data-raised-group="needs_you"] [data-pane="${blocked}"]`)).toHaveCount(0);
    await expect(row(blocked)).toContainText("디스크가 가득 차 멈췄어요");
    // Still read after Herdr's next change reaches the core (#901): the click's
    // assertions above hold on the frame the click drew, and a revert would
    // show on the projection that change brings.
    await afterNextHerdrChange(herdr, page, unfinished);
    await expectRead(page, blocked, "error");
    await expect(row(blocked)).toContainText("디스크가 가득 차 멈췄어요");
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});

test("a read block stays read when Herdr marks its tab seen after the operator moved on", async ({ page }) => {
  // Herdr gives a tab's panes the seen mark when it applies a focus, which
  // takes a finished pane from done to idle at the same sequence (#901). Held
  // at the socket, that focus lands after the operator has gone on to the
  // neighbouring row, so the core holds the read it made while the pane was
  // still done and Herdr's own bookkeeping must not make it news.
  await page.setViewportSize({ width: 1440, height: 900 });
  const herdr = await startHerdr({ readiness: "registered" });
  let gate: Awaited<ReturnType<typeof herdrGate>> | null = null;
  let daemon: Daemon | null = null;
  try {
    const [blocked, unfinished] = herdr.panes;
    await finishTwoTurns(herdr, herdr.panes);
    gate = await herdrGate(herdr);
    daemon = await startHided({ ...herdr, socket: gate.socket }, "blocked-state-seen-late");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await expect(sidebarRow(page, blocked).locator('[data-agent-status="error"]')).toBeVisible({ timeout: 30_000 });
    await expect(page.locator(`[data-raised-group="needs_you"] [data-pane="${blocked}"]`)).toBeVisible();

    const held = gate.arm("pane.focus");
    await sidebarRow(page, blocked).locator("[data-agent-open]").first().click();
    await held;
    await expect(page.locator(`[data-pane-view="${blocked}"]`)).toHaveAttribute("data-focused", "true");
    await expectRead(page, blocked, "error");
    await sidebarRow(page, unfinished).locator("[data-agent-open]").first().click();
    await expect(page.locator(`[data-pane-view="${unfinished}"]`)).toHaveAttribute("data-focused", "true");
    expect(gate.params("pane.focus").map((params) => params.pane_id)).toEqual([blocked]);

    await gate.release();
    // The gate saw Herdr's first answer, not the core's queued focus finish.
    // A lifecycle event can overtake it; wait for Herdr's actual focus before
    // asking for the change that fences its seen bookkeeping.
    await expect.poll(() => herdrHasFocus(herdr, unfinished)).toBe(true);
    expect(gate.params("pane.focus").map((params) => params.pane_id)).toContain(unfinished);
    await afterNextHerdrChange(herdr, page, unfinished);
    await expectRead(page, blocked, "error");
  } finally {
    daemon?.stop();
    await gate?.stop();
    herdr.stop();
  }
});
