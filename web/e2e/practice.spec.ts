// The practice window (UI_BEHAVIOR.md, Practice window) on an isolated
// engine and hided: opened from the shell's Help menu, the first-run cards
// played with Enter alone, the last card handing a real mission to the shell's
// Start panel, and the look-up cards on the list showing their frames and
// chords. The cards run the shell's own components, so a change that moves
// what a card points at stops this journey at that card and names it.

import { expect, test, type Locator, type Page } from "@playwright/test";
import { clickClosingItsPage } from "./closing-click";
import { startFixture } from "./engine-fixture";
import { startHided, type Daemon } from "./hided-fixture";

test.describe.configure({ timeout: 180_000 });

/**
 * Records each call a page makes to the system clipboard in `window.clipboardCalls`; the practice window makes none.
 * A call never reaches the real clipboard, so one the check catches has not already changed the operator's.
 */
function watchClipboard() {
  const calls: string[] = [];
  Object.assign(window, { clipboardCalls: calls });
  const board = navigator.clipboard;
  if (!board) return;
  for (const name of ["read", "readText", "write", "writeText"] as const) {
    if (!board[name]) continue;
    Object.defineProperty(board, name, { configurable: true, value: () => (calls.push(name), Promise.reject(new DOMException("The practice journey holds the clipboard", "NotAllowedError"))) });
  }
}

/**
 * Waits for the page to draw what the last change opened, two frames, before the next press: a person's next key
 * never lands in the frame a palette opened in, and a press there finds the palette's field not yet drawn.
 */
async function drawn(practice: Page): Promise<void> {
  await practice.evaluate(() => new Promise<void>((done) => requestAnimationFrame(() => requestAnimationFrame(() => done()))));
}

/** Opens the practice window from the shell's Help menu of an isolated stack, runs `journey` on it, and stops the stack. */
async function withPractice(page: Page, journey: (practice: Page, chapters: Locator) => Promise<void>): Promise<void> {
  await page.setViewportSize({ width: 1280, height: 800 });
  const engine = await startFixture();
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(engine, "practice");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await expect(page.locator("nav[data-sidebar]")).toHaveAttribute("data-sidebar", "projects", { timeout: 20_000 });

    await page.context().addInitScript(watchClipboard);
    await page.locator("[data-help-menu-trigger]").click();
    const opened = page.context().waitForEvent("page");
    await page.locator('[data-help-item="practice"]').click();
    const practice = await opened;
    await practice.setViewportSize({ width: 1280, height: 800 });
    // The window is the shell's own origin with no token: it holds no daemon connection.
    expect(new URL(practice.url()).origin).toBe(daemon.origin);
    expect(new URL(practice.url()).hash).toBe("");

    const chapters = practice.locator("[data-practice-hub] [data-practice-hub-chapter]");
    await expect(chapters.first()).toBeVisible({ timeout: 20_000 });
    await journey(practice, chapters);
    // A journey that went back to Hide closed the window, and with it what is left to read.
    if (practice.isClosed()) return;
    // The shell answered, so the line that says progress is not being kept never showed.
    await expect(practice.locator("[data-practice-unsaved]")).toHaveCount(0);
    // Every copy the chapters made went to their worlds, and no paste read the operator's clipboard.
    expect(await practice.evaluate(() => (window as unknown as { clipboardCalls: string[] }).clipboardCalls)).toEqual([]);
  } finally {
    daemon?.stop();
    engine.stop();
  }
}

/** The tags a card hangs on the parts of what it rings (`callouts`), by card. */
const CALLOUTS: Record<string, number> = { projects: 5, needs: 2, read: 1, work: 1 };

/** The first-run cards, in order: one screen each, the real part they point at on screen. */
const FIRST_RUN = ["parts", "projects", "needs", "read", "work", "agents", "explorer", "conduct", "find", "devices"];

test("the first-run cards play with Enter alone, each pointing at a part of the screen, with the card clear of it", async ({ page }) => {
  await withPractice(page, async (practice) => {
    await expect(practice.locator('[data-practice-hub-chapter="overview"]')).toHaveAttribute("data-status", "new");
    await practice.locator('[data-practice-hub-chapter="overview"]').click();
    await practice.locator('[data-practice-detail="overview"] [data-practice-play]').click();
    const layer = practice.locator("[data-practice-layer]");
    await expect(layer).toHaveAttribute("data-practice-steps", String(FIRST_RUN.length));
    for (const [at, id] of FIRST_RUN.entries()) {
      await expect(layer).toHaveAttribute("data-practice-step", id);
      await drawn(practice);
      // Every card but the three zones points at real parts: each is ringed on screen (the children chip is the smallest part, so a ring is at least 16px high; a part that reaches the window's edge carries its ring's few pixels past it), and the floating card covers none of them.
      const marks = id !== "parts";
      const ringed = async () => {
        const card = await practice.locator("[data-practice-card]").boundingBox();
        const rings = await practice.locator("[data-practice-ring]").evaluateAll((spots) => spots.map((spot) => spot.getBoundingClientRect().toJSON() as { x: number; y: number; width: number; height: number }));
        if (!marks) return rings.length === 0;
        const view = practice.viewportSize()!;
        if (!card || rings.length === 0) return false;
        return rings.every((ring) => ring.width > 24 && ring.height > 16 && ring.x >= -4 && ring.y >= -4 && ring.x + ring.width <= view.width + 4 && ring.y + ring.height <= view.height + 4
          && (card.x + card.width <= ring.x || ring.x + ring.width <= card.x || card.y + card.height <= ring.y || ring.y + ring.height <= card.y));
      };
      await expect.poll(ringed, { message: `${id}'s ring is missing, off screen, or under its card`, timeout: 5_000 }).toBe(true);
      // The card sets its own scene: the list it reads, the panes of the tab, what the lane holds.
      if (id === "projects") await expect(practice.locator('[data-sidebar-lens-option="projects"]')).toHaveAttribute("data-state", "on");
      if (id === "needs") await expect(practice.locator('[data-sidebar-lens-option="tasks"]')).toHaveAttribute("data-state", "on");
      // The project view folds 내 차례 · 읽을 것 away, so its ring takes the projects alone, and each mark of a row has a tag that covers no tag or card.
      if (id === "projects") await expect(practice.locator("[data-raised-list]")).toBeHidden();
      await expect(practice.locator("[data-practice-callout]")).toHaveCount(CALLOUTS[id] ?? 0);
      // Once the card has settled (it glides to its place), no tag is off screen, under the card or touching another.
      const clear = async () => {
        const tags = await practice.locator("[data-practice-callout]").evaluateAll((all) => all.map((tag) => tag.getBoundingClientRect().toJSON() as { x: number; y: number; width: number; height: number }));
        const card = await practice.locator("[data-practice-card]").boundingBox();
        const view = practice.viewportSize()!;
        return tags.every((tag, at) => tag.x >= 0 && tag.y >= 0 && tag.x + tag.width <= view.width && tag.y + tag.height <= view.height
          && (!card || card.x + card.width <= tag.x || tag.x + tag.width <= card.x || card.y + card.height <= tag.y || tag.y + tag.height <= card.y)
          && tags.slice(at + 1).every((other) => tag.y + tag.height <= other.y || other.y + other.height <= tag.y));
      };
      await expect.poll(clear, { message: `${id}'s tags are off screen, under the card, or touching`, timeout: 5_000 }).toBe(true);
      // The agents card opens the lead's children list from the chip, and leaving the card closes it.
      if (id === "agents") await expect(practice.locator('[data-agent-tree="a1"]')).toBeVisible();
      else await expect(practice.locator("[data-agent-tree]")).toHaveCount(0);
      // The Explorer card shows what the agent changed, marked in the tree, with a result opened beside it.
      if (id === "explorer") {
        await expect(practice.locator('[data-lane-tool="explorer"]')).toHaveAttribute("aria-pressed", "true");
        await expect(practice.locator('[data-decoration="modified"]').first()).toBeVisible();
        await expect(practice.locator('[data-decoration="untracked"]')).toBeVisible();
        await expect(practice.locator('[data-pane-view="q1"]')).toBeVisible();
      }
      if (id === "conduct") await expect(practice.locator("[data-pane-mission-chip]")).toBeVisible();
      if (at === FIRST_RUN.length - 1) break;
      await practice.keyboard.press("Enter");
      await expect(layer).toHaveAttribute("data-practice-step", FIRST_RUN[at + 1]!);
    }
  });
});

test("the last card leaves the practice for Hide", async ({ page }) => {
  await withPractice(page, async (practice) => {
    await practice.locator('[data-practice-hub-chapter="overview"]').click();
    await practice.locator('[data-practice-detail="overview"] [data-practice-play]').click();
    for (let at = 0; at < FIRST_RUN.length - 1; at += 1) {
      await drawn(practice);
      await practice.keyboard.press("Enter");
    }
    await expect(practice.locator("[data-practice-layer]")).toHaveAttribute("data-practice-step", FIRST_RUN.at(-1)!);
    await drawn(practice);
    // The page closes on this press, so the button the press stands for is clicked through the helper that waits out that close.
    await clickClosingItsPage(practice.locator("[data-practice-go]"));
  });
});

test("a look-up card shows its frames, one line and the chords it uses", async ({ page }) => {
  await withPractice(page, async (practice) => {
    await practice.locator('[data-practice-hub-chapter="left"]').click();
    const detail = practice.locator('[data-practice-detail="left"]');
    await expect(detail).toBeVisible();
    expect(await detail.locator("[data-practice-frames] li").count()).toBeGreaterThanOrEqual(3);
    await expect(detail.locator("[data-practice-chords] li").first()).toBeVisible();
    // The frames take turns: one is lit, and it changes.
    const lit = () => detail.locator('[data-frame-active="true"]').evaluate((li) => [...li.parentElement!.children].indexOf(li));
    const first = await lit();
    await expect.poll(lit, { timeout: 10_000 }).not.toBe(first);
  });
});

test("the chapter list leads back to Hide, and a finished first run reads as seen", async ({ page }) => {
  await withPractice(page, async (practice) => {
    await practice.locator('[data-practice-hub-chapter="overview"]').click();
    await practice.locator('[data-practice-detail="overview"] [data-practice-play]').click();
    await expect(practice.locator("[data-practice-layer]")).toBeVisible();
    for (let at = 0; at < FIRST_RUN.length - 1; at += 1) {
      await drawn(practice);
      await practice.keyboard.press("Enter");
    }
    await practice.locator("[data-practice-hub-open]").click();
    await expect(practice.locator("[data-practice-hub]")).toBeVisible();
    await expect(practice.locator('[data-practice-hub-chapter="overview"]')).toHaveAttribute("data-status", "done");
    await expect(practice).toHaveTitle(/Practice Hide|Hide 연습/);
    await clickClosingItsPage(practice.locator("[data-practice-hub] [data-practice-leave]"));
  });
});

/** An isolated stack's shell, signed in, for `journey`; the stack stops after it. */
async function withShell(page: Page, journey: (origin: string) => Promise<void>): Promise<void> {
  await page.setViewportSize({ width: 1280, height: 800 });
  const engine = await startFixture();
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(engine, "practice-invite");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await expect(page.locator("nav[data-sidebar]")).toHaveAttribute("data-sidebar", "projects", { timeout: 20_000 });
    await journey(daemon.origin);
  } finally {
    daemon?.stop();
    engine.stop();
  }
}

test("the sidebar's invitation brings the open practice window forward on the first run and goes once a chapter is opened", async ({ page }) => {
  await withShell(page, async () => {
    const invite = page.locator("[data-practice-invite]");
    await expect(invite).toBeVisible();
    // A practice window already open on its chapter list: the invitation plays the first run there, in the same window.
    await page.locator("[data-help-menu-trigger]").click();
    const opened = page.context().waitForEvent("page");
    await page.locator('[data-help-item="practice"]').click();
    const practice = await opened;
    await expect(practice.locator("[data-practice-hub]")).toBeVisible({ timeout: 20_000 });
    // Only opening the window is not practicing: the invitation stays until a chapter opens.
    await expect(invite).toBeVisible();
    await page.bringToFront();
    await page.locator("[data-practice-invite-open]").click();
    await expect(practice.locator('[data-practice-chapter="overview"]')).toBeVisible({ timeout: 20_000 });
    expect(page.context().pages()).toHaveLength(2);
    await expect(invite).toHaveCount(0);
    await page.reload();
    await expect(page.locator("nav[data-sidebar]")).toBeVisible({ timeout: 20_000 });
    await expect(invite).toHaveCount(0);
  });
});

test("the sidebar's invitation closed with its × stays closed after a reload", async ({ page }) => {
  await withShell(page, async () => {
    const invite = page.locator("[data-practice-invite]");
    await expect(invite).toBeVisible();
    await page.locator("[data-practice-invite-close]").click();
    await expect(invite).toHaveCount(0);
    await page.reload();
    await expect(page.locator("nav[data-sidebar]")).toBeVisible({ timeout: 20_000 });
    await expect(invite).toHaveCount(0);
    // The ways in that were there stay: the Help menu still opens the practice window.
    await page.locator("[data-help-menu-trigger]").click();
    await expect(page.locator('[data-help-item="practice"]')).toBeVisible();
  });
});
