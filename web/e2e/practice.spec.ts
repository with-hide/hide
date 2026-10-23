// The practice window (UI_BEHAVIOR.md, Practice window) on an isolated pinned
// Herdr and hided: opened from the shell's Help menu, every chapter played to
// its end with Enter alone without a call to the system clipboard, and the
// chapter list then reads every chapter as seen from the progress the core kept. The chapters run the shell's own
// components, so a change that moves what a step points at, presses or waits
// for stops this journey at that step and names it.

import { expect, test, type Page } from "@playwright/test";
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

test("the practice window plays every chapter with Enter alone and the core keeps them seen", async ({ page }) => {
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
    const count = await chapters.count();
    expect(count).toBeGreaterThan(1);
    await expect(chapters.and(practice.locator('[data-status="new"]'))).toHaveCount(count);

    await playEveryChapter(practice, count);

    await expect(chapters.and(practice.locator('[data-status="done"]'))).toHaveCount(count);
    // The shell answered, so the line that says progress is not being kept never showed.
    await expect(practice.locator("[data-practice-unsaved]")).toHaveCount(0);
    // Every copy the chapters made went to their worlds, and no paste read the operator's clipboard.
    expect(await practice.evaluate(() => (window as unknown as { clipboardCalls: string[] }).clipboardCalls)).toEqual([]);
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});
