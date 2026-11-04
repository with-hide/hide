// A private native transcript reaches the actual ledger, snapshot and both
// usage surfaces. Cache reads are deliberately much larger than new tokens.
import fs from "node:fs";
import path from "node:path";
import { expect, type ElectronApplication } from "@playwright/test";
import { startFixture } from "../../web/e2e/engine-fixture";
import { usageShim } from "../../web/e2e/usage-fixture";
import { connectHome, fitWindow, isolate, launchShell, screenshot, test } from "./fixture";

test("native usage history preserves totals, hover geometry and the Overview filter", async () => {
  // Two shells, not agents: the usage shim stands in for `claude` on the daemon's PATH, where an agent
  // start would find it too. The daemon starts after that PATH and the ledger's files are set, so this
  // composes `startStack`'s steps.
  const fixture = await startFixture({ agents: false });
  const run = isolate(fixture, "usage-history");
  const usage = usageShim();
  run.env.PATH = usage.path;
  usage.release();
  // General terminal fixtures link their transcripts; the ledger deliberately
  // refuses linked trees, so this accounting fixture owns a real directory.
  const projects = path.join(run.env.HOME!, ".claude", "projects");
  if (fs.lstatSync(projects).isSymbolicLink()) fs.unlinkSync(projects);
  const root = path.join(projects, "fixture");
  fs.mkdirSync(root, { recursive: true });
  const now = Date.now();
  fs.writeFileSync(path.join(root, "usage.jsonl"), Array.from({ length: 6 }, (_, i) => JSON.stringify({
    type: "assistant", sessionId: "fixture", requestId: `request-${i}`,
    timestamp: new Date(now - i * 86_400_000).toISOString(),
    message: { id: `message-${i}`, model: `fixture-unpriced-model-${"x".repeat(100)}`, content: [], usage: { input_tokens: 10, output_tokens: 20, cache_creation_input_tokens: 30, cache_read_input_tokens: 900 } },
  })).join("\n") + "\n");
  let app: ElectronApplication | undefined;
  const failures: unknown[] = [];
  try {
    await fixture.opened!(await connectHome(run));
    const opened = await launchShell(run.env);
    app = opened.app;
    const page = opened.page;
    await fitWindow(app, { width: 720, height: 480 });
    const chip = page.locator('[data-usage-chip="claude"]');
    await expect(chip).toBeVisible({ timeout: 30_000 });
    const popover = page.locator('[data-usage-popover]');
    const detail = popover.locator('[data-usage-detail]');
    const capture = async (name: string) => {
      const candidate = await app!.evaluate(({ BrowserWindow }) => ({ pid: process.pid, executable: process.execPath, windows: BrowserWindow.getAllWindows().length, switches: process.argv, bounds: BrowserWindow.getAllWindows()[0]!.getBounds() }));
      expect(candidate.windows).toBe(1);
      const directory = process.env.HIDE_E2E_SCREENSHOT_DIR;
      if (directory) fs.writeFileSync(path.join(directory, `${name}.json`), JSON.stringify({ candidate, home: run.env.HOME, socket: fixture.socket, fixture: "six native calls; new tokens 360, cache read 5400; unknown model", collector: "isolated Electron renderer screenshot" }, null, 2));
      await screenshot(page, name);
    };
    for (const language of ["en", "ko"] as const) {
      for (const large of [false, true]) {
        await page.locator('[data-open-settings]').click();
        await page.locator('[data-settings-tab="general"]').click();
        await page.locator('[data-interface-language]').click();
        await page.locator(`[data-language-option="${language}"]`).click();
        await page.locator(`[data-theme-option="${language === "en" ? "light" : "dark"}"]`).click();
        const slider = page.locator('[data-font-size] [role="slider"]');
        await slider.press(large ? "End" : "Home");
        if (!large) { await slider.press("ArrowRight"); await slider.press("ArrowRight"); }
        await page.keyboard.press("Escape");
        await chip.click();
        // The approved compact popover leads with today, not the entire period.
        await expect(popover.locator('[data-usage-total]')).toHaveText("60", { timeout: 30_000 });
        await expect(popover.locator('[data-usage-day]')).toHaveCount(30);
        await expect(detail.locator('[data-usage-cost]')).not.toContainText("$0.00");
        const before = await detail.boundingBox();
        await popover.locator('[data-usage-day][data-today="true"]').hover();
        await expect(detail.locator('[data-usage-cost]')).toHaveAttribute("aria-label", /fixture-unpriced-model/);
        // A day of other content, here the oldest with no records against the
        // fixture's unpriced cost, must keep the summary's height: a value that
        // wraps moves the grass under the pointer, which then hovers another day.
        await popover.locator('[data-usage-day]').first().hover();
        await expect(detail.locator('[data-usage-total]')).toHaveText(language === "en" ? "No data" : "기록 없음");
        const after = await detail.boundingBox();
        expect(after!.height).toBe(before!.height);
        expect(await detail.locator('.usage-summary-value').evaluateAll(values => values.every(value => value.scrollWidth <= value.clientWidth))).toBe(true);
        await popover.locator('[data-usage-day][data-today="true"]').hover();
        expect(await popover.evaluate(element => element.scrollWidth <= element.clientWidth)).toBe(true);
        await detail.locator('[data-usage-cost]').focus();
        const costHint = page.locator('[data-slot="tooltip-content"]');
        await expect(costHint).toBeVisible();
        expect(await costHint.evaluate(element => element.scrollWidth <= element.clientWidth)).toBe(true);
        await capture(`usage-history-${language}-${large ? "largest" : "default"}`);
        await popover.locator('[data-usage-history-link]').click();
        const overview = page.locator('[data-usage-overview]');
        await expect(overview).toBeVisible();
        await expect(overview.locator('[data-usage-total]')).toHaveText("360");
        const filter = overview.locator('[data-usage-agent-select]');
        await filter.click();
        await page.getByRole("option", { name: "Claude Code", exact: true }).click();
        await expect(filter).toHaveText("Claude Code");
        await expect(overview.locator('[data-usage-agent-row="claude"]')).toHaveCount(1);
        await expect(overview.locator('[data-usage-agent-row="claude"]')).toContainText("960");
        await filter.click();
        await page.getByRole("option", { name: language === "en" ? "All agents" : "모든 에이전트", exact: true }).click();
        await expect(overview.locator('[data-usage-date-row]')).toHaveCount(6);
        expect(await overview.evaluate(element => element.scrollWidth <= element.clientWidth)).toBe(true);
        await capture(`usage-overview-${language}-${large ? "largest" : "default"}`);
      }
    }
  } catch (error) { failures.push(error); }
  finally {
    try { await app?.close(); } catch (error) { failures.push(error); }
    try { run.cleanup(); } catch (error) { failures.push(error); }
    try { fixture.stop(); } catch (error) { failures.push(error); }
    try { usage.cleanup(); } catch (error) { failures.push(error); }
  }
  if (failures.length) throw new AggregateError(failures, "native history or teardown failed");
});
