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

/** What each first-run card rings, for the cards whose part is small or sits among others: the one ring is the box that fits it, a few pixels off each edge. */
const FITTED: Record<string, string> = {
  session: '[data-sidebar-box] [data-pane="a2"]',
  pr: '[data-sidebar-box] [data-pane="a2"] [data-pr-mark]',
  mission: '[data-sidebar-box] [data-pane="a2"] [data-mission-chip]',
  pane: '[data-pane-view="a2"]',
  line1: '[data-pane-view="a2"] [data-pane-move-row]',
  line2: '[data-pane-view="a2"] [data-pane-header-band]',
  tree: '[data-pane-tree="a2"]',
  missionChip: '[data-pane-view="a2"] [data-pane-mission-chip]',
  explorerButton: '[data-lane-tool="explorer"]',
};

/** The first-run cards, in order: one ring and one card a screen. A card that asks for a press (`tree`, `missionChip`, `explorerButton`) goes on when its target is pressed, and ⏎ presses it. */
const FIRST_RUN = ["left", "middle", "right", "projects", "session", "pr", "mission", "needs", "read", "work", "pane", "line1", "line2", "tree", "kids", "missionChip", "conduct", "explorerButton", "explorer", "find", "devices"];

/** Plays the first run with Enter alone up to the card `id`, waiting for each card to come up before the next press. */
async function toStep(practice: Page, id: string): Promise<void> {
  const layer = practice.locator("[data-practice-layer]");
  const target = FIRST_RUN.indexOf(id);
  for (;;) {
    const now = Number(await layer.getAttribute("data-practice-index"));
    if (now >= target) return;
    await drawn(practice);
    await practice.keyboard.press("Enter");
    await expect(layer).toHaveAttribute("data-practice-index", String(now + 1));
  }
}

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
      // Every card points at a real part: it is ringed on screen (the children chip is the smallest part, so a ring is at least 16px high; a part that reaches the window's edge carries its ring's few pixels past it), and the floating card covers none of them.
      const ringed = async () => {
        const card = await practice.locator("[data-practice-card]").boundingBox();
        const rings = await practice.locator("[data-practice-ring]").evaluateAll((spots) => spots.map((spot) => spot.getBoundingClientRect().toJSON() as { x: number; y: number; width: number; height: number }));
        // One ring a card, never a ring inside a ring.
        if (rings.length !== 1) return false;
        const view = practice.viewportSize()!;
        if (!card || rings.length === 0) return false;
        // After a card that changes the screen the ring and the card wait for it to settle, then come back whole.
        const shown = await practice.evaluate(() => [".pc-spot", ".pc-card"].every((selector) => Number(getComputedStyle(document.querySelector(selector)!).opacity) > 0.99));
        if (!shown) return false;
        return rings.every((ring) => ring.width > 24 && ring.height > 16 && ring.x >= -4 && ring.y >= -4 && ring.x + ring.width <= view.width + 4 && ring.y + ring.height <= view.height + 4
          && (card.x + card.width <= ring.x || ring.x + ring.width <= card.x || card.y + card.height <= ring.y || ring.y + ring.height <= card.y));
      };
      await expect.poll(ringed, { message: `${id}'s ring is missing, off screen, or under its card`, timeout: 5_000 }).toBe(true);
      // The card sets its own scene: the list it reads, the panes of the tab, what the lane holds.
      if (id === "projects") await expect(practice.locator('[data-sidebar-lens-option="projects"]')).toHaveAttribute("data-state", "on");
      if (id === "needs") await expect(practice.locator('[data-sidebar-lens-option="tasks"]')).toHaveAttribute("data-state", "on");
      // The project cards fold 내 차례 · 읽을 것 away, so the list shows the projects alone.
      if (id === "projects" || id === "session" || id === "pr" || id === "mission") await expect.poll(() => practice.locator("[data-raised-group]").evaluateAll((all) => all.length > 0 && all.every((one) => getComputedStyle(one).display === "none")), { message: "the raised sections are folded away" }).toBe(true);
      // A card says it all in its own words: no tag is drawn, and the one ring fits what it names.
      await expect(practice.locator("[data-practice-callout], [data-practice-mark]")).toHaveCount(0);
      const fitted = FITTED[id];
      if (fitted) {
        const fits = async () => {
          const ring = await practice.locator("[data-practice-ring]").boundingBox();
          const part = await practice.locator(fitted).first().boundingBox();
          return !!ring && !!part && [ring.x - part.x, ring.y - part.y, part.x + part.width - (ring.x + ring.width), part.y + part.height - (ring.y + ring.height)].every((gap) => gap <= 0 && gap >= -8);
        };
        await expect.poll(fits, { message: `${id}'s ring does not fit its part`, timeout: 5_000 }).toBe(true);
      }
      // The children list is open on its own card only; leaving a card closes what it opened.
      if (id === "kids") await expect(practice.locator('[data-agent-tree="a2"]')).toBeVisible();
      else await expect(practice.locator("[data-agent-tree]")).toHaveCount(0);
      // 내 차례 holds what waits for the learner (a question, an approval) and no 볼 것; its ring takes the switch with it, 태스크 on.
      if (id === "needs") await expect(practice.locator('[data-task-group="needs_you"] [data-mark="look"]')).toHaveCount(0);
      // The Explorer card shows what the agent changed, marked in the tree, with a result opened beside it.
      if (id === "explorer") {
        await expect(practice.locator('[data-lane-tool="explorer"]')).toHaveAttribute("aria-pressed", "true");
        await expect(practice.locator('[data-decoration="modified"]').first()).toBeVisible();
        await expect(practice.locator('[data-decoration="untracked"]')).toBeVisible();
        // The lane keeps about its default width: the agent's window stays wide enough to read beside it.
        await expect(practice.locator('[data-pane-view="q1"]')).toBeVisible();
        expect((await practice.locator('[data-pane-view="q1"]').boundingBox())!.width).toBeGreaterThan(240);
      }
      // The mission chip opened the map beside the Conductor's window.
      if (id === "conduct") {
        await expect(practice.locator('[data-pane-view="a2"] [data-pane-mission-chip]')).toBeVisible();
        await expect(practice.locator("[data-mission-map]").first()).toBeVisible();
      }
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
    await toStep(practice, FIRST_RUN.at(-1)!);
    await expect(practice.locator("[data-practice-layer]")).toHaveAttribute("data-practice-step", FIRST_RUN.at(-1)!);
    await drawn(practice);
    // The page closes on this press, so the button the press stands for is clicked through the helper that waits out that close.
    await clickClosingItsPage(practice.locator("[data-practice-go]"));
  });
});

test("a card that asks for a press goes on when its target is pressed, and the card's own button presses it", async ({ page }) => {
  await withPractice(page, async (practice) => {
    await practice.locator('[data-practice-hub-chapter="overview"]').click();
    await practice.locator('[data-practice-detail="overview"] [data-practice-play]').click();
    const layer = practice.locator("[data-practice-layer]");
    // The children chip: pressed by the learner, the list opens and the card after it rings that list.
    await toStep(practice, "tree");
    await expect(practice.locator('[data-practice-card][data-practice-press="true"]')).toBeVisible();
    await expect(practice.locator("[data-agent-tree]")).toHaveCount(0);
    await practice.locator('[data-pane-tree="a2"]').click();
    await expect(layer).toHaveAttribute("data-practice-step", "kids");
    await expect(practice.locator('[data-agent-tree="a2"]')).toBeVisible();
    // The mission chip, pressed the same way: the mission's map opens on the right.
    await toStep(practice, "missionChip");
    await expect(practice.locator("[data-mission-map]")).toHaveCount(0);
    await practice.locator('[data-pane-view="a2"] [data-pane-mission-chip]').click();
    await expect(layer).toHaveAttribute("data-practice-step", "conduct");
    await expect(practice.locator("[data-mission-map]").first()).toBeVisible();
    // The Explorer's rail button opens the Explorer beside a result.
    await toStep(practice, "explorerButton");
    await expect(practice.locator('[data-lane-tool="explorer"]')).toHaveAttribute("aria-pressed", "false");
    await practice.locator('[data-lane-tool="explorer"]').click();
    await expect(layer).toHaveAttribute("data-practice-step", "explorer");
    await expect(practice.locator('[data-lane-tool="explorer"]')).toHaveAttribute("aria-pressed", "true");
    // Going back to a press card closes what the press opened: the card asks again.
    await practice.keyboard.press("ArrowLeft");
    await expect(layer).toHaveAttribute("data-practice-step", "explorerButton");
    await expect(practice.locator('[data-lane-tool="explorer"]')).toHaveAttribute("aria-pressed", "false");
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
    await toStep(practice, FIRST_RUN.at(-1)!);
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
