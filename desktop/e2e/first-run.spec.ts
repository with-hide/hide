// The first-run wizard in the desktop app (PRD first-run-onboarding): a HOME
// whose kit has never run opens it over the shell, its steps move only
// through their own buttons, and the notification step and Settings share the
// switch the core keeps. The development host has no permission helper, so
// macOS's permission reads as unreadable here; the packaged helper is checked
// natively. The page's test notification is caught in the main process so the
// run never puts a banner on the machine's screen.

import { expect, type ElectronApplication } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { startHerdr, type HerdrFixture } from "../../web/e2e/herdr-fixture";
import { hostLog, isolate, launch, nodeOf, screenshot, test, type Isolated } from "./fixture";

let herdr: HerdrFixture;
let run: Isolated;
let app: ElectronApplication | null = null;

test.describe.configure({ timeout: 180_000 });

test.beforeAll(async () => {
  herdr = await startHerdr();
});

test.afterAll(() => {
  herdr?.stop();
});

test.beforeEach(() => {
  run = isolate(herdr, "first-run");
  // A machine the kit has never run on (`seedKitRecord`): the first run's question waits for the wizard.
  fs.rmSync(path.join(run.env.HOME!, ".hide", "kit", "installed.json"));
});

test.afterEach(async () => {
  const info = test.info();
  if (info.status !== info.expectedStatus) console.log(hostLog(run.env).map((line) => JSON.stringify(line)).join("\n"));
  await app?.close().catch(() => undefined);
  app = null;
  run.cleanup();
});

test("the wizard walks agents, notifications and the first project, never returns, and Help reopens it as a closable walk", async () => {
  let page;
  ({ app, page } = await launch(run.env));
  await app.evaluate(({ Notification }) => {
    Notification.prototype.show = function show() {};
  });
  const wizard = page.locator("[data-first-run]");
  // B3: the agents step over the shell, with the three-part progress.
  await expect(wizard).toHaveAttribute("data-first-run", "agents", { timeout: 60_000 });
  await expect(page.locator('[data-first-run-progress="1"]')).toContainText("1 of 3");
  await screenshot(page, "first-run-agents");
  // B5: Escape does not close it.
  await page.keyboard.press("Escape");
  await expect(wizard).toHaveAttribute("data-first-run", "agents");

  // B6: Continue moves on once this Mac's kit pass for it has landed.
  await page.locator("[data-onboarding-apply]").click();
  await expect(wizard).toHaveAttribute("data-first-run", "notifications", { timeout: 60_000 });
  await expect(page.locator('[data-first-run-next="notifications"]')).toHaveText("Not now");
  await page.locator("[data-notification-turn-on]").click();
  // B17: no helper in a development host, so the setting cannot be read; the test still goes, once.
  await expect(page.locator('[data-notification-line="unreadable"]')).toBeVisible();
  await expect.poll(() => hostLog(run.env).filter((line) => line.event === "notify.shown").length).toBe(1);
  expect(hostLog(run.env).some((line) => line.event === "notify.permission_unreadable" && line.reason === "helper_missing")).toBe(true);
  await screenshot(page, "first-run-notifications");
  await page.locator('[data-first-run-next="notifications"]').click();

  await expect(wizard).toHaveAttribute("data-first-run", "project");
  // The last step asks how to start: each way is its own button, with no choice to confirm.
  await expect(page.locator("[data-first-run-way]")).toHaveCount(2);
  await screenshot(page, "first-run-start");
  await page.locator('[data-first-run-way="project"]').click();
  await expect(page.locator("[data-add-project-browse]")).toBeVisible();
  // A cancelled pick keeps Add project's views; their Back returns to the two ways, and the choice survives a reload.
  await app.evaluate(({ dialog }) => {
    dialog.showOpenDialog = (async () => ({ canceled: true, filePaths: [] })) as typeof dialog.showOpenDialog;
  });
  await page.locator("[data-add-project-browse]").click();
  await expect(page.locator("[data-add-project-browse]")).toBeVisible();
  await page.reload();
  await expect(page.locator("[data-add-project-browse]")).toBeVisible({ timeout: 30_000 });
  await screenshot(page, "first-run-project");
  await page.locator("[data-first-run-back]").click();
  await expect(page.locator('[data-first-run-way="practice"]')).toBeVisible();
  await expect(wizard).toHaveAttribute("data-first-run", "project");
  // B25: Skip for now ends it, and it does not return (B6).
  await page.locator("[data-first-run-skip]").click();
  await expect(wizard).toHaveCount(0);
  await page.reload();
  await expect(page.locator("[data-main-screen], [data-workspace-screen]").first()).toBeVisible({ timeout: 30_000 });
  await expect(wizard).toHaveCount(0);

  // The fresh wizard enables Needs You only; either setting enables the test.
  await page.locator("[data-open-settings]").click();
  await page.locator('[data-settings-tab="notifications"]').click();
  await expect(page.locator('[data-settings-group="notifications"] [data-notification-switch]')).toHaveAttribute("aria-checked", "true");
  const needsYou = page.locator("[data-notification-switch]");
  const done = page.locator("[data-notification-done-switch]");
  await expect(done).toHaveAttribute("aria-checked", "false");
  await done.click();
  await needsYou.click();
  await expect(needsYou).toHaveAttribute("aria-checked", "false");
  await expect(done).toHaveAttribute("aria-checked", "true");
  await page.locator("[data-notification-test]").click();
  await expect.poll(() => hostLog(run.env).filter((line) => line.event === "notify.shown").length).toBe(3);
  await page.reload();
  await page.locator("[data-open-settings]").click();
  await page.locator('[data-settings-tab="notifications"]').click();
  await expect(needsYou).toHaveAttribute("aria-checked", "false");
  await expect(done).toHaveAttribute("aria-checked", "true");
  await page.setViewportSize({ width: 720, height: 480 });
  expect(await page.locator("[data-settings-page]").evaluate((node) => node.scrollWidth - node.clientWidth)).toBe(0);
  await screenshot(page, "first-run-settings-notifications");
  await page.keyboard.press("Escape");

  // B45, B46: Help > Set up Hide opens the steps from the first, and Escape closes them.
  await app.evaluate(({ Menu }) => Menu.getApplicationMenu()!.getMenuItemById("setup_hide")!.click());
  await expect(wizard).toHaveAttribute("data-first-run-mode", "setup");
  await expect(wizard).toHaveAttribute("data-first-run", "agents");
  await page.keyboard.press("Escape");
  await expect(wizard).toHaveCount(0);
});

/** Replaces the native folder picker with one answer per call. */
async function stubPicker(app: ElectronApplication, folders: string[]): Promise<void> {
  await app.evaluate(({ dialog }, queued) => {
    const answers = [...queued];
    dialog.showOpenDialog = (async () => {
      const next = answers.shift();
      if (!next) throw new Error("no queued pick");
      return { canceled: false, filePaths: [next] };
    }) as typeof dialog.showOpenDialog;
  }, folders);
}

test("the first project closes the wizard into the start panel on it, Start runs there, and a later project opens no panel", async () => {
  const home = run.env.HOME!;
  for (const name of ["alpha", "beta"]) fs.mkdirSync(path.join(home, "projects", name), { recursive: true });
  const [alpha, beta] = ["alpha", "beta"].map((name) => fs.realpathSync(path.join(home, "projects", name)));
  let page;
  ({ app, page } = await launch(run.env));
  const wizard = page.locator("[data-first-run]");
  await expect(wizard).toHaveAttribute("data-first-run", "agents", { timeout: 60_000 });
  await page.locator("[data-onboarding-apply]").click();
  await expect(wizard).toHaveAttribute("data-first-run", "notifications", { timeout: 60_000 });
  await page.locator('[data-first-run-next="notifications"]').click();
  await expect(wizard).toHaveAttribute("data-first-run", "project");
  await page.locator('[data-first-run-way="project"]').click();
  // The wizard keeps its own steps: its Host list has no Add device.
  await page.locator("[data-add-project-host]").click();
  await expect(page.locator("[data-host-option-add-device]")).toHaveCount(0);
  await page.keyboard.press("Escape");
  await expect(wizard).toHaveAttribute("data-first-run", "project");

  // B23: the project added closes the wizard and opens the start panel on it, the keyboard in its text.
  await stubPicker(app, [alpha!]);
  await page.locator("[data-add-project-browse]").click();
  await expect(wizard).toHaveCount(0, { timeout: 20_000 });
  const panel = page.locator("[data-start-panel]");
  await expect(panel).toBeVisible();
  await expect(panel.locator("[data-start-target]")).toHaveAttribute("data-start-target", new RegExp(`^checkout:${nodeOf(run.env)}:.*/alpha$`));
  await expect(page.locator("[data-start-text]")).toBeFocused();
  await screenshot(page, "first-run-start-panel");

  // B24: Start runs the agent in that project, in a new tab.
  await page.locator("[data-start-text]").fill("first task");
  await page.locator("[data-start-submit]").click();
  const failure = panel.locator("[data-start-failure]");
  await expect
    .poll(async () => ((await failure.count()) > 0 ? `refused: ${await failure.innerText()}` : (await panel.count()) === 0 ? "closed" : "open"), { timeout: 90_000 })
    .toBe("closed");
  await expect(page.locator("[data-workspace-screen]")).toBeVisible({ timeout: 30_000 });

  // B24: a project added later opens no start panel.
  await page.locator("[data-sidebar-new-workspace]").click();
  await stubPicker(app, [beta!]);
  await page.locator("[data-add-project-browse]").click();
  await expect(page.locator("[data-add-project]")).toHaveCount(0, { timeout: 20_000 });
  await expect(page.locator("[data-project-list]")).toContainText("beta", { timeout: 20_000 });
  await expect(panel).toHaveCount(0);
});

test("with every agent off, the first project closes the wizard and opens no start panel", async () => {
  const home = run.env.HOME!;
  fs.mkdirSync(path.join(home, "projects", "alpha"), { recursive: true });
  const alpha = fs.realpathSync(path.join(home, "projects", "alpha"));
  let page;
  ({ app, page } = await launch(run.env));
  const wizard = page.locator("[data-first-run]");
  await expect(wizard).toHaveAttribute("data-first-run", "agents", { timeout: 60_000 });
  const onTiles = page.locator('[data-onboarding-tile$=":on"]');
  // Each click turns a tile off, so the first tile still on is the next one.
  for (let left = await onTiles.count(); left > 0; left -= 1) {
    await onTiles.first().click();
    await expect(onTiles).toHaveCount(left - 1);
  }
  await page.locator("[data-onboarding-apply]").click();
  await expect(wizard).toHaveAttribute("data-first-run", "notifications", { timeout: 60_000 });
  await page.locator('[data-first-run-next="notifications"]').click();
  await expect(wizard).toHaveAttribute("data-first-run", "project");
  await page.locator('[data-first-run-way="project"]').click();

  // B50: no agent on, so no panel opens on the project. (The fixture's shared Herdr runs agents, so the
  // Home here lists its tasks; the empty Home with no agent is B28's own check.)
  await stubPicker(app, [alpha]);
  await page.locator("[data-add-project-browse]").click();
  await expect(wizard).toHaveCount(0, { timeout: 20_000 });
  await expect(page.locator("[data-project-list]")).toContainText("alpha", { timeout: 20_000 });
  await expect(page.locator("[data-start-panel]")).toHaveCount(0);
});

test("practice first ends the wizard over the empty Home and opens the practice window on chapter 0", async () => {
  let page;
  ({ app, page } = await launch(run.env));
  const wizard = page.locator("[data-first-run]");
  await expect(wizard).toHaveAttribute("data-first-run", "agents", { timeout: 60_000 });
  await page.locator("[data-onboarding-apply]").click();
  await expect(wizard).toHaveAttribute("data-first-run", "notifications", { timeout: 60_000 });
  await page.locator('[data-first-run-next="notifications"]').click();
  await expect(wizard).toHaveAttribute("data-first-run", "project");
  const opened = app.waitForEvent("window");
  await page.locator('[data-first-run-way="practice"]').click();
  const practice = await opened;
  await expect(practice.locator('[data-practice-chapter="overview"]')).toBeVisible({ timeout: 30_000 });
  expect(new URL(practice.url()).searchParams.get("chapter")).toBe("overview");
  await expect(wizard).toHaveCount(0);
  // Opening the chapter is practicing: the sidebar's invitation is gone with it.
  await expect(page.locator("[data-practice-invite]")).toHaveCount(0);
  // The practice window is named apart and smaller, against the main window's lower right, so the main window shows beside it.
  const windows = await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows().map((window) => ({ practice: window.webContents.getURL().includes("practice=1"), title: window.getTitle(), bounds: window.getBounds() })));
  const main = windows.find((window) => !window.practice)!.bounds;
  const beside = windows.find((window) => window.practice)!;
  expect(beside.title).toMatch(/^(Practice Hide|Hide 연습)$/);
  expect(beside.bounds.width).toBeLessThan(main.width);
  expect(beside.bounds.x + beside.bounds.width).toBe(main.x + main.width);
  expect(beside.bounds.y + beside.bounds.height).toBe(main.y + main.height);
  // Back to Hide closes it and leaves the main window.
  const closed = practice.waitForEvent("close");
  await practice.locator("[data-practice-leave]").click();
  await closed;
  expect(await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows().length)).toBe(1);
  await page.reload();
  await expect(page.locator("[data-main-screen], [data-workspace-screen]").first()).toBeVisible({ timeout: 30_000 });
  await expect(wizard).toHaveCount(0);
});
