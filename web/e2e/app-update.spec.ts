// This app's own update as the shell draws it (PRD auto-update D-03, D-20,
// D-21, B15, B24 to B28): the dot on the footer's `?`, Help's last line, and
// Settings > About's update row and preview switch. The desktop host's
// updater is stood in for by its bridge (`window.hideHost.updates`), driven
// from the test; the updater itself is unit tested in the desktop package and
// a real update runs in the package lane (desktop/e2e/packaged-update.spec.ts).

import { expect, test, type Page } from "@playwright/test";
import { startFixture } from "./engine-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { screenshot } from "./wire";

test.describe.configure({ timeout: 120_000 });

type Phase =
  | { kind: "idle"; checkedAt: number | null }
  | { kind: "checking" }
  | { kind: "downloading"; version: string; percent: number }
  | { kind: "ready"; version: string }
  | { kind: "failed"; stage: "check" | "download" }
  | { kind: "off"; reason: "development" | "location" };

type Calls = { check: number; restart: number; prerelease: boolean[] };

declare global {
  interface Window {
    updateTest?: { show: (phase: Phase, installOnQuit?: boolean) => void; calls: Calls };
  }
}

/** The desktop host's bridge with its updater replaced by the test, installed before the page loads. */
async function desktopBridge(page: Page): Promise<void> {
  await page.addInitScript((platform) => {
    const calls: Calls = { check: 0, restart: 0, prerelease: [] };
    let listener: ((status: unknown) => void) | null = null;
    let prerelease = false;
    let current: unknown = { phase: { kind: "idle", checkedAt: null }, prerelease, installOnQuit: true };
    window.updateTest = {
      calls,
      show: (phase, installOnQuit = true) => {
        current = { phase, prerelease, installOnQuit };
        listener?.(current);
      },
    };
    window.hideHost = {
      kind: "electron",
      platform,
      onCommand: () => () => undefined,
      reportBindings: () => undefined,
      reportLanguage: () => undefined,
      revealPath: () => undefined,
      pickFolder: async () => null,
      probePaths: async () => [],
      openPath: () => undefined,
      notify: () => undefined,
      onNotificationOpen: () => () => undefined,
      withdrawNotifications: () => undefined,
      notificationPermission: async () => "unsupported",
      openNotificationSettings: () => undefined,
      reportBadge: () => undefined,
      onWindowChrome: () => () => undefined,
      editorAvailable: async () => false,
      openInEditor: () => undefined,
      browser: { sync: () => undefined, endCycle: () => undefined, capture: async () => null, command: () => undefined, onEvent: () => () => undefined },
      updates: {
        onStatus: (next: (status: unknown) => void) => {
          listener = next;
          next(current);
          return () => undefined;
        },
        check: () => { calls.check += 1; },
        restart: () => { calls.restart += 1; },
        setPrerelease: (on: boolean) => {
          calls.prerelease.push(on);
          prerelease = on;
        },
      },
    } as never;
  }, process.platform);
}

const show = (page: Page, phase: Phase, installOnQuit = true) => page.evaluate(([next, onQuit]) => window.updateTest!.show(next as Phase, onQuit as boolean), [phase, installOnQuit] as const);
const calls = (page: Page) => page.evaluate(() => window.updateTest!.calls);

async function openAbout(page: Page): Promise<void> {
  await page.locator("[data-open-settings]").click();
  await page.locator('[data-settings-tab="about"]').click();
}

async function closeSettings(page: Page): Promise<void> {
  await page.keyboard.press("Escape");
  await expect(page.locator('[data-settings-tab="about"]')).toHaveCount(0);
}

async function load(page: Page, daemon: Daemon): Promise<void> {
  await page.goto(`${daemon.origin}/#token=${daemon.token}`);
  await expect(page.locator("nav[data-sidebar]")).toBeVisible({ timeout: 20_000 });
}

test("the desktop app shows a ready update as the Help dot and line, and every state in About; a browser tab shows none", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  const herdr = await startFixture();
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(herdr, "app-update");
    const trigger = page.locator("[data-help-menu-trigger]");
    const dot = trigger.locator("[data-help-update-dot]");
    const row = page.locator("[data-settings-update]");

    // B24: a browser tab has no dot, no update line and no About row.
    await load(page, daemon);
    await expect(trigger).toHaveAccessibleName("Help");
    await expect(dot).toHaveCount(0);
    await openAbout(page);
    await expect(page.locator('[data-settings-group="about"]')).toBeVisible();
    await expect(row).toHaveCount(0);
    await expect(page.locator("[data-update-prerelease]")).toHaveCount(0);
    await closeSettings(page);

    await desktopBridge(page);
    await page.reload();
    await expect(page.locator("nav[data-sidebar]")).toBeVisible({ timeout: 20_000 });

    // D-21: looking, downloading and a failure stay in About; the footer and Help are unchanged.
    for (const phase of [{ kind: "checking" }, { kind: "downloading", version: "0.5.0", percent: 40 }, { kind: "failed", stage: "download" }] as Phase[]) {
      await show(page, phase);
      await expect(dot).toHaveCount(0);
      await expect(trigger).toHaveAccessibleName("Help");
    }
    await trigger.click();
    await expect(page.locator("[data-help-version]")).toBeVisible();
    await expect(page.locator("[data-help-update]")).toHaveCount(0);
    await page.keyboard.press("Escape");

    // B15: ready puts a named dot on `?` and turns Help's last line into Restart, reached by keyboard (B30).
    await show(page, { kind: "ready", version: "0.5.0" });
    await expect(dot).toBeVisible();
    await expect(trigger).toHaveAccessibleName("Help · Update ready");
    await trigger.focus();
    await page.keyboard.press("Enter");
    const line = page.locator('[data-help-update="0.5.0"]');
    await expect(line).toHaveText(/^hide 0\.5\.0 ready\s*Restart$/);
    await expect(page.locator("[data-help-version]")).toHaveCount(0);
    await screenshot(page, "app-update-help-ready");
    await line.focus();
    await page.keyboard.press("Enter");
    expect((await calls(page)).restart).toBe(1);

    // B25 to B28: About's row in each state, with its one action.
    await openAbout(page);
    const action = page.locator("[data-update-action]");
    const cases: [Phase, boolean, RegExp, string | null][] = [
      [{ kind: "idle", checkedAt: null }, true, /Not checked yet/, "check"],
      [{ kind: "idle", checkedAt: Date.now() - 5 * 60_000 }, true, /Up to date · checked 5 min\. ago/, "check"],
      [{ kind: "checking" }, true, /Checking…/, null],
      [{ kind: "downloading", version: "0.5.0", percent: 42 }, true, /Downloading 0\.5\.0 · 42%/, null],
      [{ kind: "ready", version: "0.5.0" }, true, /0\.5\.0 ready · installs when you quit/, "restart"],
      [{ kind: "ready", version: "0.5.0" }, false, /0\.5\.0 ready · restart to install/, "restart"],
      [{ kind: "failed", stage: "check" }, true, /Couldn't check for updates/, "retry"],
      [{ kind: "failed", stage: "download" }, true, /Couldn't update/, "retry"],
      [{ kind: "off", reason: "development" }, true, /Updates are off in development builds/, null],
      [{ kind: "off", reason: "location" }, true, /Move hide to Applications to get updates/, null],
    ];
    for (const [phase, installOnQuit, text, expected] of cases) {
      await show(page, phase, installOnQuit);
      await expect(row).toHaveAttribute("data-settings-update", phase.kind);
      await expect(row).toContainText(text);
      if (expected) await expect(action).toHaveAttribute("data-update-action", expected);
      else await expect(action).toHaveCount(0);
    }
    const preview = page.locator("[data-update-prerelease]");
    await expect(preview).toBeDisabled();

    await show(page, { kind: "failed", stage: "check" });
    await action.click();
    expect((await calls(page)).check).toBe(1);
    await show(page, { kind: "idle", checkedAt: Date.now() });
    await expect(preview).toBeEnabled();
    await expect(preview).toHaveAttribute("aria-checked", "false");
    await preview.focus();
    await page.keyboard.press("Space");
    expect((await calls(page)).prerelease).toEqual([true]);
    await show(page, { kind: "ready", version: "0.5.0" });
    await screenshot(page, "app-update-about-ready");
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});
