// The native host's smallest window and largest text must contain the footer's
// chips, the rail's View, Help and Settings, and the Usage sheet. The provider alone is a fixed external shim.
import fs from "node:fs";
import path from "node:path";
import { expect, type ElectronApplication } from "@playwright/test";
import { startHerdr } from "../../web/e2e/herdr-fixture";
import { usageShim } from "../../web/e2e/usage-fixture";
import { fitWindow, isolate, launchShell, screenshot, test } from "./fixture";

test("three usage chips, View, Help and Settings fit the minimum native window in English and Korean", async () => {
  const herdr = await startHerdr();
  const run = isolate(herdr, "usage");
  const usage = usageShim();
  run.env.PATH = usage.path;
  usage.release();
  let app: ElectronApplication | undefined;
  const failures: unknown[] = [];
  try {
    const opened = await launchShell(run.env);
    app = opened.app;
    const page = opened.page;
    await fitWindow(app, { width: 720, height: 480 });
    await expect(page.locator('[data-usage-chip="claude"]')).toContainText("62%", { timeout: 30_000 });
    await page.locator('[data-usage-chip="claude"]').click();
    await page.getByRole("button", { name: "Choose sidebar items" }).click();
    await page.getByRole("switch", { name: "Claude Code 5 hours" }).click();
    await page.locator('[data-settings-tab="general"]').click();
    await page.locator('[data-font-size] [role="slider"]').press("End");
    const facts = { home: run.env.HOME, state: run.env.HIDE_STATE_DIR, socket: herdr.socket, provider: "external Claude CLI shim; Codex signed out", window: "720x480, maximum Interface text" };
    const capture = async (name: string) => {
      const candidate = await app!.evaluate(({ BrowserWindow }) => ({
        pid: process.pid, executable: process.execPath, windows: BrowserWindow.getAllWindows().length,
        switches: process.argv, bounds: BrowserWindow.getAllWindows()[0]!.getBounds(),
      }));
      expect(candidate.windows).toBe(1);
      const directory = process.env.HIDE_E2E_SCREENSHOT_DIR;
      if (directory) fs.writeFileSync(path.join(directory, `${name}.json`), JSON.stringify({ ...facts, candidate, collector: "Playwright page.screenshot on isolated Electron renderer; excludes native traffic lights" }, null, 2));
      await screenshot(page, name);
    };
    for (const language of ["en", "ko"] as const) {
      await page.locator('[data-interface-language]').click();
      await page.locator(`[data-language-option="${language}"]`).click();
      await page.locator(`[data-theme-option="${language === "en" ? "light" : "dark"}"]`).click();
      await page.locator('[data-settings-tab="usage"]').click();
      await expect(page.locator('[data-usage-settings]')).toBeVisible();
      await expect(page.locator('[data-settings-tab="about"]')).toBeInViewport({ ratio: 1 });
      const alignment = await page.locator('[data-usage-pin-row]').evaluateAll(rows => rows.map(row => {
        const control = row.querySelector('[role="switch"]')!.getBoundingClientRect();
        const label = row.querySelector('[data-usage-pin-label]')!.getBoundingClientRect();
        return { right: control.right, centered: Math.abs((control.top + control.bottom - label.top - label.bottom) / 2) < 4 };
      }));
      // Claude Code's and Codex CLI's 5-hour and weekly windows, and agy's weekly window.
      expect(alignment).toHaveLength(5);
      expect(alignment.every(row => row.centered && Math.abs(row.right - alignment[0]!.right) < 1)).toBe(true);
      await capture(`usage-settings-${language}-electron`);
      const settings = page.locator('[data-usage-settings]');
      await settings.getByText("Grok", { exact: true }).scrollIntoViewIfNeeded();
      await capture(`usage-settings-unsupported-${language}-electron`);
      // Retired agents have no settings row; omp is the last current row.
      await expect(settings.getByText("Cursor", { exact: true })).toHaveCount(0);
      for (const label of ["Grok", "OpenCode", "Pi", "omp"]) {
        await expect(settings.locator("[data-usage-unsupported]").getByText(label, { exact: true })).toHaveCount(1);
      }
      await settings.getByText("omp", { exact: true }).scrollIntoViewIfNeeded();
      expect(await settings.evaluate(element => element.scrollWidth <= element.clientWidth)).toBe(true);
      await capture(`usage-settings-bottom-${language}-electron`);
      await page.keyboard.press("Escape");
      const chips = page.locator('[data-usage-chip]');
      await expect(chips).toHaveCount(3);
      const footer = page.locator('[data-sidebar-footer]');
      const boxes = await footer.evaluate((element) => {
        const bounds = element.getBoundingClientRect();
        return [...element.querySelectorAll("button")].map(button => {
          const box = button.getBoundingClientRect();
          return box.left >= bounds.left && box.right <= bounds.right && box.top >= bounds.top && box.bottom <= bounds.bottom;
        });
      });
      // The three chips in the footer; View, Help and Settings stand at the device rail's foot.
      expect(boxes).toHaveLength(3);
      expect(boxes.every(Boolean)).toBe(true);
      const menus = page.locator("[data-device-rail] [data-rail-menus] button");
      await expect(menus).toHaveCount(3);
      for (const menu of await menus.all()) await expect(menu).toBeInViewport({ ratio: 1 });
      await capture(`usage-footer-${language}-electron`);
      await chips.first().click();
      const popover = page.locator('[data-usage-popover]');
      await expect(popover.locator('[data-usage-window-row="five_hour"] [data-usage-value]')).toHaveText("4%");
      expect(await popover.evaluate(element => element.scrollWidth <= element.clientWidth)).toBe(true);
      await capture(`usage-popover-${language}-electron`);
      await popover.getByRole("button", { name: language === "en" ? "Choose sidebar items" : "사이드바에 표시할 항목 고르기", exact: true }).click();
      await page.locator('[data-settings-tab="general"]').click();
    }
  } catch (error) { failures.push(error); }
  finally {
    try { await app?.close(); } catch (error) { failures.push(error); }
    try { run.cleanup(); } catch (error) { failures.push(error); }
    try { herdr.stop(); } catch (error) { failures.push(error); }
    try { usage.cleanup(); } catch (error) { failures.push(error); }
  }
  if (failures.length) throw new AggregateError(failures, "native Usage or teardown failed");
});
