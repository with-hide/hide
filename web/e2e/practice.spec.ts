// The practice window (UI_BEHAVIOR.md, Practice window) on an isolated pinned
// Herdr and hided: opened from the shell's Help menu, every chapter played to
// its end with Enter alone without a call to the system clipboard, and the
// chapter list then reads every chapter as seen from the progress the core kept. The chapters run the shell's own
// components, so a change that moves what a step points at, presses or waits
// for stops this journey at that step and names it. A chapter started over
// from any step plays to its end the same way: nothing the shell's screen held
// from the run before carries into the next.

import { expect, test, type Locator, type Page } from "@playwright/test";
import { startHerdr } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";

test.describe.configure({ timeout: 180_000 });

/** More presses than any chapter's steps and the press that starts it; a practice that needs more has stopped moving. */
const PRESSES_PER_CHAPTER = 25;

/** Records each call a page makes to the system clipboard in `window.clipboardCalls`; the practice window makes none. */
function watchClipboard() {
  const calls: string[] = [];
  Object.assign(window, { clipboardCalls: calls });
  const board = navigator.clipboard;
  if (!board) return;
  for (const name of ["read", "readText", "write", "writeText"] as const) {
    const real = board[name] as ((...args: unknown[]) => unknown) | undefined;
    if (!real) continue;
    Object.defineProperty(board, name, { configurable: true, value: (...args: unknown[]) => (calls.push(name), real.apply(board, args)) });
  }
}

/** Where the practice window stands: the chapter list, or a chapter's step and how many events its world has answered. */
function practiceState(page: Page): Promise<string> {
  return page.evaluate(() => {
    const root = document.querySelector("[data-practice-chapter]");
    if (!root) return document.querySelector("[data-practice-hub]") ? "hub" : "none";
    const step = document.querySelector("[data-practice-layer]")?.getAttribute("data-practice-step");
    return `${root.getAttribute("data-practice-chapter")}:${step}:${root.getAttribute("data-practice-moves")}`;
  });
}

/** Presses Enter from the chapter list until it comes back to it, each press waiting for the practice to move. */
async function playEveryChapter(practice: Page, chapters: number): Promise<void> {
  let now = await practiceState(practice);
  let presses = 0;
  do {
    expect(presses, `the practice stopped moving at ${now}`).toBeLessThan(chapters * PRESSES_PER_CHAPTER);
    const before = now;
    await practice.keyboard.press("Enter");
    presses += 1;
    await expect.poll(() => practiceState(practice), { message: `Enter at ${before} moved nothing`, timeout: 10_000 }).not.toBe(before);
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
