// This app's own update as the shell draws it (PRD auto-update D-03, D-20,
// D-21, B15, B24 to B28): the ↑ button beside Help and the card it opens once
// per ready version, the dot on `?` and the update at the head of Help, and
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

type Calls = { check: number; restart: number; prerelease: boolean[]; dismiss: string[]; links: string[] };

declare global {
  interface Window {
    updateTest?: { show: (phase: Phase, installOnQuit?: boolean, required?: boolean) => void; calls: Calls };
  }
}

/** The desktop host's bridge with its updater replaced by the test, installed before the page loads. */
async function desktopBridge(page: Page): Promise<void> {
  await page.addInitScript((platform) => {
    const calls: Calls = { check: 0, restart: 0, prerelease: [], dismiss: [], links: [] };
    let listener: ((status: unknown) => void) | null = null;
    let prerelease = false;
    // The host keeps the closed version beside the status, as `AppUpdates.dismiss` does.
    let dismissed: string | null = null;
    let phase: Phase = { kind: "idle", checkedAt: null };
    let installOnQuit = true;
    // Whether a release names a minimum version above this app, as `AppUpdates` reads it from the release list.
    let required = false;
    const send = () => listener?.({ phase, prerelease, installOnQuit, dismissed, required });
    window.updateTest = {
      calls,
      show: (next, onQuit = true, needed = false) => {
        phase = next;
        installOnQuit = onQuit;
        required = needed;
        send();
      },
    };
    // Release notes open in the default browser through the page's opener.
    window.open = ((url?: string | URL) => { calls.links.push(String(url)); return null; }) as typeof window.open;
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
          send();
          return () => undefined;
        },
        check: () => { calls.check += 1; },
        restart: () => { calls.restart += 1; },
        setPrerelease: (on: boolean) => {
          calls.prerelease.push(on);
          prerelease = on;
        },
        dismiss: (version: string) => {
          calls.dismiss.push(version);
          if (phase.kind === "ready" && phase.version === version) {
            dismissed = version;
            send();
          }
        },
      },
    } as never;
  }, process.platform);
}

const show = (page: Page, phase: Phase, installOnQuit = true, required = false) => page.evaluate(([next, onQuit, needed]) => window.updateTest!.show(next as Phase, onQuit as boolean, needed as boolean), [phase, installOnQuit, required] as const);
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

test("the desktop app shows every update state in About and nothing beside Help until one is ready; a browser tab shows none", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  const engine = await startFixture();
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(engine, "app-update");
    const trigger = page.locator("[data-help-menu-trigger]");
    const dot = trigger.locator("[data-help-update-dot]");
    const row = page.locator("[data-settings-update]");

    // B24: a browser tab has no dot, no update line and no About row.
    await load(page, daemon);
    await expect(trigger).toHaveAccessibleName("Help");
    await expect(dot).toHaveCount(0);
    await expect(page.locator("[data-update-button]")).toHaveCount(0);
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

    await expect(page.locator("[data-update-button]")).toHaveCount(0);

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
    engine.stop();
  }
});

test("a ready update shows the ↑ button, opens its card once per version, and leads Help", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  const engine = await startFixture();
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(engine, "app-update-ready");
    const trigger = page.locator("[data-help-menu-trigger]");
    const dot = trigger.locator("[data-help-update-dot]");
    await desktopBridge(page);
    await load(page, daemon);

    // Ready: the ↑ button heads View, Help and Settings, and its card opens by itself once,
    // taking nothing from where the keyboard was.
    // The keyboard stays where it was when the card opens by itself.
    await trigger.focus();
    await show(page, { kind: "ready", version: "0.5.0" });
    const button = page.locator('[data-update-button="0.5.0"]');
    const card = page.locator('[data-update-card="0.5.0"]');
    await expect(button).toBeVisible();
    await expect(button).toHaveAccessibleName("hide 0.5.0 ready");
    await expect(card).toBeVisible();
    await expect(page.locator('[data-update-card-open="auto"]')).toBeVisible();
    await expect(card).toContainText("Update ready");
    await expect(card).toContainText("Downloaded hide 0.5.0.");
    await expect(card).toContainText("Restarting keeps your agents and panes running.");
    await expect(trigger).toBeFocused();
    await expect(dot).toBeVisible();
    await screenshot(page, "app-update-card");

    // Opened by itself it stays through Escape and a click elsewhere; What's new opens the release page.
    await page.keyboard.press("Escape");
    await page.locator("main").click({ position: { x: 200, y: 400 } });
    await expect(card).toBeVisible();
    await card.locator("[data-update-card-notes]").click();
    expect((await calls(page)).links).toEqual(["https://github.com/with-hide/hide/releases/tag/v0.5.0"]);

    // Later folds it into the button; the host keeps the close, so the same version does not open it again.
    await card.locator("[data-update-card-later]").click();
    await expect(card).toHaveCount(0);
    expect((await calls(page)).dismiss).toEqual(["0.5.0"]);
    await show(page, { kind: "ready", version: "0.5.0" });
    await expect(card).toHaveCount(0);
    await expect(button).toBeVisible();
    await expect(dot).toBeVisible();
    await screenshot(page, "app-update-button");

    // The button opens the card on request, as an ordinary popover that Escape closes.
    await button.click();
    await expect(page.locator('[data-update-card-open="asked"]')).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(card).toHaveCount(0);
    await button.click();
    await card.locator("[data-update-card-restart]").click();
    expect((await calls(page)).restart).toBe(1);
    expect((await calls(page)).dismiss).toEqual(["0.5.0"]);

    // B15: Help keeps its named dot and opens with the update, Restart now reached by keyboard (B30);
    // the version line at its foot stays this app's own.
    await page.keyboard.press("Escape");
    await expect(trigger).toHaveAccessibleName("Help · Update ready");
    await trigger.focus();
    await page.keyboard.press("Enter");
    const block = page.locator('[data-help-update-block="0.5.0"]');
    await expect(block).toContainText("hide 0.5.0 is ready");
    await expect(block).toContainText("Agents and panes keep running");
    await expect(page.locator("[data-help-version]")).toBeVisible();
    await screenshot(page, "app-update-help-ready");
    const line = page.locator('[data-help-update="0.5.0"]');
    await expect(line).toHaveText("Restart now");
    await line.focus();
    await page.keyboard.press("Enter");
    expect((await calls(page)).restart).toBe(2);

    // A newer ready version opens the card once more.
    await show(page, { kind: "downloading", version: "0.5.1", percent: 10 });
    await expect(button).toHaveCount(0);
    await show(page, { kind: "ready", version: "0.5.1" });
    await expect(page.locator('[data-update-card="0.5.1"]')).toBeVisible();

    // A required update opens its card even after its close, with no close or Later, says the work keeps
    // running, and stays through Escape and a click elsewhere, still taking no key meant for a pane.
    await page.locator('[data-update-card="0.5.1"] [data-update-card-later]').click();
    await expect(page.locator('[data-update-card="0.5.1"]')).toHaveCount(0);
    // The closed card hands the keyboard back to its ↑ button from a timer; wait for it, or it lands after the next focus.
    await expect(page.locator('[data-update-button="0.5.1"]')).toBeFocused();
    await trigger.focus();
    await show(page, { kind: "ready", version: "0.5.1" }, true, true);
    const required = page.locator('[data-update-card="0.5.1"][data-update-card-required]');
    await expect(required).toBeVisible();
    await expect(page.locator('[data-update-button="0.5.1"]')).toHaveAccessibleName("Required update · hide 0.5.1");
    await expect(required).toContainText("Required update");
    await expect(required.locator("[data-update-card-keeps]")).toHaveText("Your work won't stop: agents and panes keep running while hide restarts.");
    await expect(required.locator("[data-update-card-close], [data-update-card-later]")).toHaveCount(0);
    await expect(trigger).toBeFocused();
    await page.keyboard.press("Escape");
    await page.locator("main").click({ position: { x: 200, y: 400 } });
    await expect(required).toBeVisible();
    await screenshot(page, "app-update-card-required");
    await required.locator("[data-update-card-restart]").click();
    expect((await calls(page)).restart).toBe(3);
  } finally {
    daemon?.stop();
    engine.stop();
  }
});
