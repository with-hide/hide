// The practice window (UI_BEHAVIOR.md, Practice window) on an isolated
// engine and hided: opened from the shell's Help menu, every chapter played to
// its end with Enter alone, and again by the guide's button, without a call to
// the system clipboard, and the chapter list then reads every chapter as seen
// from the progress the core kept. The chapters run the shell's own
// components, so a change that moves what a step points at, presses or waits
// for stops this journey at that step and names it. A chapter started over
// from any step plays to its end the same way: nothing the shell's screen held
// from the run before carries into the next.

import fs from "node:fs";
import path from "node:path";
import { expect, test, type Locator, type Page } from "@playwright/test";
import { clickClosingItsPage } from "./closing-click";
import { startFixture } from "./engine-fixture";
import { startHided, type Daemon } from "./hided-fixture";

test.describe.configure({ timeout: 180_000 });

/**
 * More presses than any chapter needs: its steps, the press that starts it, one that brings in a step's scene, one
 * past each waiting step once its beat came, and, where the clock is run ahead, one per beat; a practice that needs
 * more has stopped moving.
 */
const PRESSES_PER_CHAPTER = 40;

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
    // A card pressed on the list opens what its chapter has the learner do, and its start plays it.
    const detail = document.querySelector("[data-practice-detail]")?.getAttribute("data-practice-detail");
    if (!root) return document.querySelector("[data-practice-hub]") ? (detail ? `hub~${detail}` : "hub") : "none";
    const layer = document.querySelector("[data-practice-layer]");
    // A step that opens on its scene reads as its own state until the guide comes in, which a press does.
    const step = layer?.hasAttribute("data-practice-ended") ? "ended" : `${layer?.getAttribute("data-practice-step")}${layer?.hasAttribute("data-practice-scene") ? "~scene" : ""}`;
    return `${root.getAttribute("data-practice-chapter")}:${step}:${root.getAttribute("data-practice-moves")}`;
  });
}

/**
 * Ends the moment a step ⏎ performed stays while its result is shown, the way the learner's ⏎ there does, so the
 * journey does not wait out the learner's pace; the press is made in the page only while the moment lasts, so it can
 * never land on the next step. Where the bubble shows no button (a palette is up), the moment is waited out.
 */
async function settled(practice: Page): Promise<void> {
  await practice.evaluate(() => document.querySelector<HTMLButtonElement>("[data-practice-layer][data-practice-settling] [data-practice-go]")?.click());
  await expect(practice.locator("[data-practice-settling]")).toHaveCount(0);
}

/**
 * Waits for the page to draw what the last change opened, two frames, before the next press: a person's next key
 * never lands in the frame a palette opened in, and a press there finds the palette's field not yet drawn.
 */
async function drawn(practice: Page): Promise<void> {
  await practice.evaluate(() => new Promise<void>((done) => requestAnimationFrame(() => requestAnimationFrame(() => done()))));
}

/** One press of the guide: ⏎, or its bubble's button where the bubble shows one. */
type GuidePress = (practice: Page) => Promise<void>;
const pressEnter: GuidePress = (practice) => practice.keyboard.press("Enter");
/**
 * The bubble's button, as a mouse presses it, on every step that shows one: all but the chapter list, where ⏎ opens
 * the chosen chapter's card and then starts it, and an open palette, which hides the bubble's button and keeps ⏎ for itself.
 */
const pressButton: GuidePress = async (practice) => {
  const guiding = await practice.locator("[data-practice-layer]").count();
  // Before a step's scene is over there is no bubble yet, and ⏎ brings it in.
  if (!guiding || (await practice.locator("[data-palette], [data-practice-scene]").count())) return practice.keyboard.press("Enter");
  await practice.locator("[data-practice-layer] [data-practice-go]").click();
};

/**
 * Presses the guide from the chapter list, each press waiting for the practice to move, and follows each end card on to the
 * flow it leads to. The flows lead round in a circle (the mission's end card leads back to the first), so a press that
 * arrives on a flow already played goes back to the list, which starts a flow not yet played, until every one has been.
 */
async function playEveryChapter(practice: Page, chapters: number, press: GuidePress = pressEnter): Promise<void> {
  const played = new Set<string>();
  let last = "";
  let now = await practiceState(practice);
  let presses = 0;
  do {
    expect(presses, `the practice stopped moving at ${now}`).toBeLessThan(chapters * PRESSES_PER_CHAPTER);
    const before = now;
    await drawn(practice);
    // A step that waits on the world's own beat takes no press and moves on by itself, in up to the beats' sum.
    if ((await practice.locator("[data-practice-layer][data-practice-waiting]").count()) > 0) {
      try {
        await expect.poll(() => practiceState(practice), { message: `the wait at ${before} never ended`, timeout: 30_000 }).not.toBe(before);
      } catch (error) {
        await keepStall(practice);
        throw error;
      }
    } else {
      await press(practice);
      presses += 1;
      await expect.poll(() => practiceState(practice), { message: `a press at ${before} moved nothing`, timeout: 10_000 }).not.toBe(before);
    }
    await settled(practice);
    now = await practiceState(practice);
    if (!now.startsWith("hub")) {
      const id = now.split(":")[0];
      if (played.has(id) && id !== last) {
        await practice.locator("[data-practice-hub-open]").click();
        now = await practiceState(practice);
      } else played.add(id);
      last = id;
    }
    const next = CHAPTER_IDS.find((id) => !played.has(id));
    if (now === "hub" && next) {
      last = "";
      await playFromList(practice, next);
      now = await practiceState(practice);
      played.add(next);
      last = next;
    }
  } while (now !== "hub");
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
    engine.stop();
  }
}

test("the practice window plays every chapter with Enter alone and the core keeps them seen", async ({ page }) => {
  // The waiting steps run in real time, so the whole list takes minutes.
  test.setTimeout(600_000);
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
  // The waiting steps run in real time, so the whole list takes minutes.
  test.setTimeout(600_000);
  await withPractice(page, async (practice, chapters) => {
    const count = await chapters.count();
    await playEveryChapter(practice, count, pressButton);
    await expect(chapters.and(practice.locator('[data-status="done"]'))).toHaveCount(count);
  });
});

/**
 * Presses Enter in chapter `id` until its step `at` shows, each press waiting for the practice to move; null presses on until the chapter is left.
 * A step that waits on the world's own beat takes no Enter; `hurry`, given by a journey that faked the page's clock, runs it ahead there so the beat comes at once.
 */
async function pressTo(practice: Page, id: string, at: number | null, hurry: (() => Promise<void>) | null = null): Promise<void> {
  const layer = practice.locator(`[data-practice-chapter="${id}"] [data-practice-layer]`);
  for (let presses = 0; ; presses += 1) {
    const now = await practiceState(practice);
    if (!now.startsWith(`${id}:`)) return expect(at, `chapter ${id} was left before step ${at}`).toBeNull();
    if (at !== null && Number(await layer.getAttribute("data-practice-index")) >= at) return;
    if (presses >= PRESSES_PER_CHAPTER) await keepStall(practice);
    expect(presses, `the practice stopped moving at ${now}`).toBeLessThan(PRESSES_PER_CHAPTER);
    const waiting = (await layer.getAttribute("data-practice-waiting")) !== null;
    await drawn(practice);
    if (waiting && hurry) await hurry();
    else if (!waiting) await practice.keyboard.press("Enter");
    try {
      await expect.poll(() => practiceState(practice), { message: `Enter at ${now} moved nothing`, timeout: 10_000 }).not.toBe(now);
    } catch (error) {
      await keepStall(practice);
      throw error;
    }
    await settled(practice);
  }
}

/** Keeps what the practice window showed where it stopped moving: its screen, and whether the step's target was there to press. */
async function keepStall(practice: Page): Promise<void> {
  const info = test.info();
  // Written under the test's own output folder, which CI keeps with the run's logs.
  await practice.screenshot({ path: info.outputPath("practice-stall.png") });
  const facts = await practice.evaluate(() => {
    const layer = document.querySelector("[data-practice-layer]");
    return {
      step: layer?.getAttribute("data-practice-step"),
      settling: layer?.hasAttribute("data-practice-settling"),
      waiting: layer?.hasAttribute("data-practice-waiting"),
      children: [...document.querySelectorAll("[data-agent-child]")].map((row) => row.getAttribute("data-agent-child")),
      trees: [...document.querySelectorAll("[data-pane-tree]")].map((tree) => `${tree.getAttribute("data-pane-tree")}:${tree.getAttribute("aria-expanded")}`),
      focused: document.querySelector("[data-pane-view][data-focused='true']")?.getAttribute("data-pane-view") ?? null,
    };
  });
  fs.writeFileSync(info.outputPath("practice-stall.json"), JSON.stringify(facts, null, 2));
}

/** Every chapter's id, one per file in `src/practice/chapters/`, as the practice lists them; each gets a journey of its own below. */
const CHAPTER_IDS = fs.readdirSync(path.resolve("src/practice/chapters")).filter((file) => file.endsWith(".ts")).map((file) => file.slice(0, -".ts".length));

/** Opens chapter `id`'s card on the list and presses its start, or its From the start where the card offers one and `fromStart` asks for it. */
async function playFromList(practice: Page, id: string, fromStart = false): Promise<void> {
  await practice.locator(`[data-practice-hub-chapter="${id}"]`).click();
  const detail = practice.locator(`[data-practice-detail="${id}"]`);
  await expect(detail).toBeVisible();
  const again = detail.locator("[data-practice-from-start]");
  await (fromStart && (await again.count()) ? again : detail.locator("[data-practice-play]")).click();
}

// One test per chapter, so a chapter whose steps grow takes its own time budget and shard rather than every chapter's.
for (const id of CHAPTER_IDS) {
  test(`chapter ${id} started over from each of its steps plays to its end with Enter alone`, async ({ page }) => {
    test.setTimeout(600_000);
    await withPractice(page, async (practice) => {
      // Chapter 0 waits seconds at a time for its Conductor; replayed from each of its steps that would be minutes, so its clock runs ahead on those steps.
      await practice.clock.install();
      const hurry = () => practice.clock.fastForward(5_000);
      const row = practice.locator(`[data-practice-hub-chapter="${id}"]`);
      const layer = practice.locator(`[data-practice-chapter="${id}"] [data-practice-layer]`);
      await playFromList(practice, id);
      const steps = Number(await layer.getAttribute("data-practice-steps"));
      // Each run goes one step further than the one before, and starts over from the list where the learner would.
      for (let reached = 1; reached < steps; reached += 1) {
        await pressTo(practice, id, reached, hurry);
        await practice.locator("[data-practice-hub-open]").click();
        // A run that reached the last step has seen the chapter, and the list then offers it again from the start.
        await expect(row).toHaveAttribute("data-status", /^(paused|done)$/);
        await playFromList(practice, id, true);
        await expect(layer).toHaveAttribute("data-practice-index", "0");
      }
      await pressTo(practice, id, null, hurry);
      if ((await practiceState(practice)) !== "hub") await practice.locator("[data-practice-hub-open]").click();
      await expect(row).toHaveAttribute("data-status", "done");
    });
  });
}

test("Back over a step that changed the screen plays the chapter again up to that step as it began", async ({ page }) => {
  await withPractice(page, async (practice) => {
    await playFromList(practice, "agents");
    const layer = practice.locator('[data-practice-chapter="agents"] [data-practice-layer]');
    // The split is the step that changed the screen: a second terminal stands beside the agent.
    await pressTo(practice, "agents", 1);
    await expect(practice.locator("[data-pane-view]")).toHaveCount(2);
    // Back to the split: one pane, before anything was split.
    await layer.locator("[data-practice-back]").click();
    await expect(layer).toHaveAttribute("data-practice-index", "0");
    await expect(practice.locator("[data-pane-view]")).toHaveCount(1);
    await expect(layer.locator("[data-practice-back]")).toHaveCount(0);
  });
});

test("the bubble's Exit closes the practice from any step, keeping where the chapter stopped", async ({ page }) => {
  await withPractice(page, async (practice) => {
    await playFromList(practice, "overview");
    await pressTo(practice, "overview", 1);
    await clickClosingItsPage(practice.locator('[data-practice-chapter="overview"] [data-practice-exit]'));
    // The step reached went to the shell before the window went, so the list opened again offers to continue there.
    await page.locator("[data-help-menu-trigger]").click();
    const reopened = page.context().waitForEvent("page");
    await page.locator('[data-help-item="practice"]').click();
    const again = await reopened;
    await expect(again.locator('[data-practice-hub-chapter="overview"]')).toHaveAttribute("data-status", "paused", { timeout: 20_000 });
  });
});

test("the first chapter ends by asking what next, and Use Hide now closes the practice", async ({ page }) => {
  await withPractice(page, async (practice) => {
    await playFromList(practice, "mission");
    const layer = practice.locator('[data-practice-chapter="mission"] [data-practice-layer]');
    await pressTo(practice, "mission", Number(await layer.getAttribute("data-practice-steps")) - 1);
    await practice.keyboard.press("Enter");
    const choice = practice.locator('[data-practice-chapter="mission"] [data-practice-choice]');
    await expect(choice.locator("[data-practice-choice-way]")).toHaveCount(3);
    await clickClosingItsPage(choice.locator('[data-practice-choice-way="leave"]'));
  });
});

test("the chapter list leads back to Hide, and a finished chapter stops on its end card", async ({ page }) => {
  await withPractice(page, async (practice) => {
    await playFromList(practice, "overview");
    await practice.locator('[data-practice-chapter="overview"] [data-practice-hub-open]').click();
    await expect(practice.locator("[data-practice-hub]")).toBeVisible();
    await expect(practice.locator("[data-practice-hub] [data-practice-leave]")).toBeVisible();
    await expect(practice).toHaveTitle(/Practice Hide|Hide 연습/);
    // The last step's button finishes the chapter on its end card, which lists the tasks and leads back to the list.
    await playFromList(practice, "overview");
    await pressTo(practice, "overview", 8);
    await practice.keyboard.press("Enter");
    const end = practice.locator('[data-practice-chapter="overview"] [data-practice-end]');
    await expect(end).toBeVisible();
    await expect(end.locator("li")).toHaveCount(2);
    await end.locator("[data-practice-end-hub]").click();
    await expect(practice.locator('[data-practice-hub-chapter="overview"]')).toHaveAttribute("data-status", "done");
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

test("the sidebar's invitation brings the open practice window forward on the mission flow and goes once a chapter is opened", async ({ page }) => {
  await withShell(page, async () => {
    const invite = page.locator("[data-practice-invite]");
    await expect(invite).toBeVisible();
    // A practice window already open on its chapter list: the invitation plays the mission flow there, in the same window.
    await page.locator("[data-help-menu-trigger]").click();
    const opened = page.context().waitForEvent("page");
    await page.locator('[data-help-item="practice"]').click();
    const practice = await opened;
    await expect(practice.locator("[data-practice-hub]")).toBeVisible({ timeout: 20_000 });
    // Only opening the window is not practicing: the invitation stays until a chapter opens.
    await expect(invite).toBeVisible();
    await page.bringToFront();
    await page.locator("[data-practice-invite-open]").click();
    await expect(practice.locator('[data-practice-chapter="mission"]')).toBeVisible({ timeout: 20_000 });
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
