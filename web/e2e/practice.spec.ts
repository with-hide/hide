// The practice window (UI_BEHAVIOR.md, Practice window) on an isolated pinned
// Herdr and hided: opened from the shell's Help menu, every chapter played to
// its end with Enter alone, and again by the guide's button, without a call to
// the system clipboard, and the chapter list then reads every chapter as seen
// from the progress the core kept. The chapters run the shell's own
// components, so a change that moves what a step points at, presses or waits
// for stops this journey at that step and names it. A chapter started over
// from any step plays to its end the same way: nothing the shell's screen held
// from the run before carries into the next.

import { expect, test, type Locator, type Page } from "@playwright/test";
import { clickClosingItsPage } from "./closing-click";
import { startHerdr } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";

test.describe.configure({ timeout: 180_000 });

/** More presses than any chapter's steps and the press that starts it; a practice that needs more has stopped moving. */
const PRESSES_PER_CHAPTER = 25;

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

/** Where the practice window stands: the chapter list, or a chapter's step (or its end card) and how many events its world has answered. */
function practiceState(page: Page): Promise<string> {
  return page.evaluate(() => {
    const root = document.querySelector("[data-practice-chapter]");
    if (!root) return document.querySelector("[data-practice-hub]") ? "hub" : "none";
    const layer = document.querySelector("[data-practice-layer]");
    const step = layer?.hasAttribute("data-practice-ended") ? "ended" : layer?.getAttribute("data-practice-step");
    return `${root.getAttribute("data-practice-chapter")}:${step}:${root.getAttribute("data-practice-moves")}`;
  });
}

/**
 * Chapter 4's mention step by hand, the way the step says also works: the
 * signup agent's Copy name and ID, then the lead pane's Paste, which takes
 * the copy the chapter made, never the system clipboard; the learner's own
 * words then go with Enter as they are.
 */
async function pasteByHand(practice: Page): Promise<void> {
  await practice.locator('[data-pane-view="w1:p3"] header').click({ button: "right" });
  await practice.locator('[role="menu"] [data-menu-item="copy_reference"]').click();
  const lead = practice.locator('[data-pane-view="w1:p1"] .xterm');
  // The guide's bubble sits inside the pane; its lower left corner is the terminal's own.
  const box = await lead.boundingBox();
  if (!box) throw new Error("the lead pane's terminal is not on screen");
  const corner = { x: 24, y: box.height - 24 };
  await lead.click({ button: "right", position: corner });
  await practice.locator('[role="menu"] [data-menu-item="paste"]').click();
  // The pasted reference finishes the mention step as @ would.
  await expect.poll(() => practiceState(practice), { message: "the pasted reference did not reach the lead's prompt" }).toMatch(/^delegate:send:/);
  await lead.click({ position: corner });
  await practice.keyboard.type(" update the signup copy");
  await practice.keyboard.press("Enter");
  // The world took the request only if it named the copied ID.
  await expect.poll(() => practiceState(practice), { message: "the pasted request did not reach the signup agent" }).toMatch(/^delegate:end:/);
}

/** Waits out a step ⏎ performed while its result is shown, so the next Enter is the next step's and not the one that skips the wait. */
async function settled(practice: Page): Promise<void> {
  await expect(practice.locator("[data-practice-settling]")).toHaveCount(0);
}

/** One press of the guide: ⏎, or its bubble's button where the bubble shows one. */
type GuidePress = (practice: Page) => Promise<void>;
const pressEnter: GuidePress = (practice) => practice.keyboard.press("Enter");
/**
 * The bubble's button, as a mouse presses it, on every step that shows one: all but the chapter list, where ⏎ plays
 * the chosen chapter, and an open palette, which hides the bubble's button and keeps ⏎ for itself.
 */
const pressButton: GuidePress = async (practice) => {
  const guiding = await practice.locator("[data-practice-layer]").count();
  if (!guiding || (await practice.locator("[data-palette]").count())) return practice.keyboard.press("Enter");
  await practice.locator("[data-practice-layer] [data-practice-go]").click();
};

/** Presses the guide from the chapter list until it comes back to it, each press waiting for the practice to move. */
async function playEveryChapter(practice: Page, chapters: number, press: GuidePress = pressEnter): Promise<void> {
  let now = await practiceState(practice);
  let presses = 0;
  do {
    expect(presses, `the practice stopped moving at ${now}`).toBeLessThan(chapters * PRESSES_PER_CHAPTER);
    const before = now;
    if (now.startsWith("delegate:mention:")) await pasteByHand(practice);
    else await press(practice);
    presses += 1;
    await expect.poll(() => practiceState(practice), { message: `a press at ${before} moved nothing`, timeout: 10_000 }).not.toBe(before);
    await settled(practice);
    now = await practiceState(practice);
  } while (now !== "hub");
}

/** Opens the practice window from the shell's Help menu of an isolated stack, runs `journey` on it, and stops the stack. */
async function withPractice(page: Page, journey: (practice: Page, chapters: Locator) => Promise<void>): Promise<void> {
  await page.setViewportSize({ width: 1280, height: 800 });
  const herdr = await startHerdr();
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(herdr, "practice");
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
    expect(await chapters.count()).toBeGreaterThan(1);
    await journey(practice, chapters);
    // A journey that went back to Hide closed the window, and with it what is left to read.
    if (practice.isClosed()) return;
    // The shell answered, so the line that says progress is not being kept never showed.
    await expect(practice.locator("[data-practice-unsaved]")).toHaveCount(0);
    // Every copy the chapters made went to their worlds, and no paste read the operator's clipboard.
    expect(await practice.evaluate(() => (window as unknown as { clipboardCalls: string[] }).clipboardCalls)).toEqual([]);
  } finally {
    daemon?.stop();
    herdr.stop();
  }
}

test("the practice window plays every chapter with Enter alone and the core keeps them seen", async ({ page }) => {
  await withPractice(page, async (practice, chapters) => {
    const count = await chapters.count();
    await expect(chapters.and(practice.locator('[data-status="new"]'))).toHaveCount(count);
    await playEveryChapter(practice, count);
    await expect(chapters.and(practice.locator('[data-status="done"]'))).toHaveCount(count);
  });
});

// The guide is not part of the screen it teaches: a press on its button leaves the keyboard in the pane that held it,
// so a step whose chord acts where the keyboard is (⌘W closes that pane, ⌃⇥ starts from it) plays as ⏎ plays it.
test("the practice window plays every chapter by the guide's button as it does with Enter", async ({ page }) => {
  await withPractice(page, async (practice, chapters) => {
    const count = await chapters.count();
    await playEveryChapter(practice, count, pressButton);
    await expect(chapters.and(practice.locator('[data-status="done"]'))).toHaveCount(count);
  });
});

/** Presses Enter in chapter `id` until its step `at` shows, each press waiting for the practice to move; null presses on until the chapter is left. */
async function pressTo(practice: Page, id: string, at: number | null): Promise<void> {
  const layer = practice.locator(`[data-practice-chapter="${id}"] [data-practice-layer]`);
  for (let presses = 0; ; presses += 1) {
    const now = await practiceState(practice);
    if (!now.startsWith(`${id}:`)) return expect(at, `chapter ${id} was left before step ${at}`).toBeNull();
    if (at !== null && Number(await layer.getAttribute("data-practice-index")) >= at) return;
    expect(presses, `the practice stopped moving at ${now}`).toBeLessThan(PRESSES_PER_CHAPTER);
    await practice.keyboard.press("Enter");
    await expect.poll(() => practiceState(practice), { message: `Enter at ${now} moved nothing`, timeout: 10_000 }).not.toBe(now);
    await settled(practice);
  }
}

test("every chapter started over from each of its steps plays to its end with Enter alone", async ({ page }) => {
  test.setTimeout(600_000);
  await withPractice(page, async (practice, chapters) => {
    const ids = await chapters.evaluateAll((rows) => rows.map((row) => row.getAttribute("data-practice-hub-chapter")!));
    for (const id of ids) {
      const row = practice.locator(`[data-practice-hub-chapter="${id}"]`);
      const layer = practice.locator(`[data-practice-chapter="${id}"] [data-practice-layer]`);
      await row.locator("[data-practice-play]").click();
      const steps = Number(await layer.getAttribute("data-practice-steps"));
      // Each run goes one step further than the one before, and starts over from the list where the learner would.
      for (let reached = 1; reached < steps; reached += 1) {
        await pressTo(practice, id, reached);
        await practice.locator("[data-practice-hub-open]").click();
        // A run that reached the last step has seen the chapter, and the list then offers it again from the start.
        await expect(row).toHaveAttribute("data-status", /^(paused|done)$/);
        const fromStart = row.locator("[data-practice-from-start]");
        await ((await fromStart.count()) ? fromStart : row.locator("[data-practice-play]")).click();
        await expect(layer).toHaveAttribute("data-practice-index", "0");
      }
      await pressTo(practice, id, null);
      if ((await practiceState(practice)) !== "hub") await practice.locator("[data-practice-hub-open]").click();
      await expect(row).toHaveAttribute("data-status", "done");
    }
  });
});

test("Back over a step that changed the screen plays the chapter again up to that step as it began", async ({ page }) => {
  await withPractice(page, async (practice) => {
    await practice.locator('[data-practice-hub-chapter="screen"] [data-practice-play]').click();
    const layer = practice.locator('[data-practice-chapter="screen"] [data-practice-layer]');
    const focused = practice.locator('[data-pane-view][data-focused="true"]');
    // Split, then jump back to the agent: two steps that each changed the screen.
    await pressTo(practice, "screen", 2);
    await expect(focused).toHaveAttribute("data-pane-view", "lead");
    // Back to the jump: the split is there and the keyboard is in the new pane again, as the jump step began.
    await layer.locator("[data-practice-back]").click();
    await expect(layer).toHaveAttribute("data-practice-index", "1");
    await expect(practice.locator("[data-pane-view]")).toHaveCount(2);
    await expect(focused).not.toHaveAttribute("data-pane-view", "lead");
    // Back to the split: one pane, before anything was split.
    await layer.locator("[data-practice-back]").click();
    await expect(layer).toHaveAttribute("data-practice-index", "0");
    await expect(practice.locator("[data-pane-view]")).toHaveCount(1);
    await expect(layer.locator("[data-practice-back]")).toHaveCount(0);
  });
});

test("the bubble's Exit closes the practice from any step, keeping where the chapter stopped", async ({ page }) => {
  await withPractice(page, async (practice) => {
    await practice.locator('[data-practice-hub-chapter="switch"] [data-practice-play]').click();
    await pressTo(practice, "switch", 1);
    await clickClosingItsPage(practice.locator('[data-practice-chapter="switch"] [data-practice-exit]'));
    // The step reached went to the shell before the window went, so the list opened again offers to continue there.
    await page.locator("[data-help-menu-trigger]").click();
    const reopened = page.context().waitForEvent("page");
    await page.locator('[data-help-item="practice"]').click();
    const again = await reopened;
    await expect(again.locator('[data-practice-hub-chapter="switch"]')).toHaveAttribute("data-status", "paused", { timeout: 20_000 });
  });
});

test("the first chapter ends by asking what next, and Use Hide now closes the practice", async ({ page }) => {
  await withPractice(page, async (practice) => {
    await practice.locator('[data-practice-hub-chapter="overview"] [data-practice-play]').click();
    const layer = practice.locator('[data-practice-chapter="overview"] [data-practice-layer]');
    await pressTo(practice, "overview", Number(await layer.getAttribute("data-practice-steps")) - 1);
    await practice.keyboard.press("Enter");
    const choice = practice.locator('[data-practice-chapter="overview"] [data-practice-choice]');
    await expect(choice.locator("[data-practice-choice-way]")).toHaveCount(3);
    await clickClosingItsPage(choice.locator('[data-practice-choice-way="leave"]'));
  });
});

test("the chapter list leads back to Hide, and a finished chapter stops on its end card", async ({ page }) => {
  await withPractice(page, async (practice) => {
    await practice.locator('[data-practice-hub-chapter="switch"] [data-practice-play]').click();
    await practice.locator('[data-practice-chapter="switch"] [data-practice-hub-open]').click();
    await expect(practice.locator("[data-practice-hub]")).toBeVisible();
    await expect(practice.locator("[data-practice-hub] [data-practice-leave]")).toBeVisible();
    await expect(practice).toHaveTitle(/Practice Hide|Hide 연습/);
    // The last step's button finishes the chapter on its end card, which lists the tasks and leads back to the list.
    await practice.locator('[data-practice-hub-chapter="switch"] [data-practice-play]').click();
    await pressTo(practice, "switch", 4);
    await practice.keyboard.press("Enter");
    const end = practice.locator('[data-practice-chapter="switch"] [data-practice-end]');
    await expect(end).toBeVisible();
    await expect(end.locator("li")).toHaveCount(3);
    await end.locator("[data-practice-end-hub]").click();
    await expect(practice.locator('[data-practice-hub-chapter="switch"]')).toHaveAttribute("data-status", "done");
  });
});

/** An isolated stack's shell, signed in, for `journey`; the stack stops after it. */
async function withShell(page: Page, journey: (origin: string) => Promise<void>): Promise<void> {
  await page.setViewportSize({ width: 1280, height: 800 });
  const herdr = await startHerdr();
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(herdr, "practice-invite");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await expect(page.locator("nav[data-sidebar]")).toHaveAttribute("data-sidebar", "projects", { timeout: 20_000 });
    await journey(daemon.origin);
  } finally {
    daemon?.stop();
    herdr.stop();
  }
}

test("the sidebar's invitation brings the open practice window forward on chapter 0 and goes once a chapter is opened", async ({ page }) => {
  await withShell(page, async () => {
    const invite = page.locator("[data-practice-invite]");
    await expect(invite).toBeVisible();
    // A practice window already open on its chapter list: the invitation plays chapter 0 there, in the same window.
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
