// Real core, private state and pinned Herdr. Memory UI has no platform-specific behavior.
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { expect, type ElectronApplication } from "@playwright/test";
import { startHerdr } from "../../web/e2e/herdr-fixture";
import { countSent, setLanePinned, showProjectMemory, showProjectRow } from "../../web/e2e/wire";
import { isolate, launch, fitWindow, screenshot, test } from "./fixture";
import { seedReconciliation } from "./memory-reconcile-fixture";
import { progressGithub } from "./memory-progress-fixture";

test("the Hide AI tool's Memory tab compares sources and reads at its minimum width in all four languages", async () => {
  const herdr = await startHerdr({ agents: false });
  const run = isolate(herdr, "memory-docs");
  progressGithub(herdr);
  let app: ElectronApplication | null = null;
  try {
    const repo = path.join(run.env.HOME!, "projects", "sample");
    fs.mkdirSync(repo);
    execFileSync("git", ["init", "-q", "-b", "main"], { cwd: repo });
    fs.writeFileSync(path.join(repo, "AGENTS.md"), "Use one unpooled connection per write.\n");
    execFileSync("git", ["add", "AGENTS.md"], { cwd: repo });
    execFileSync("git", ["-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "-qm", "Initial guide"], { cwd: repo });
    execFileSync("git", ["remote", "add", "origin", "https://github.com/acme/memory-fixture.git"], { cwd: repo });
    herdr.run(["workspace", "create", "--cwd", repo, "--label", "sample", "--env", `PATH=${herdr.fixturePath}`, "--no-focus"]);
    const launched = await launch(run.env);
    app = launched.app;
    const page = launched.page;
    countSent(page);
    await (await showProjectRow(page, page.locator("[data-project]", { hasText: "sample" }).locator("[data-checkout]").first())).click();
    await showProjectMemory(page);
    await page.locator('[data-lane="memory"] [data-hide-ai-tool-tab="memory"]').click();
    await expect(page.locator('[data-memory="off"]')).toBeVisible();
    await page.locator('[data-memory-switch="off"]').click();
    await expect(page.locator('[data-memory="on"]')).toBeVisible({ timeout: 30000 });
    seedReconciliation(run, "acme/memory-fixture");
    await page.reload();
    await expect(page.locator('[data-workspace-screen]')).toBeVisible();
    await expect(page.locator('[data-connection]')).toHaveCount(0);
    await showProjectMemory(page);
    const tab = page.locator('[data-lane="memory"]');
    // The tab shown is the page's own, so a reload starts on 제안.
    await tab.locator('[data-hide-ai-tool-tab="memory"]').click();
    await expect(tab.locator('[data-memory="on"]')).toBeVisible();
    // A promotion is a row of the 제안 tab now (PRD hide-ai-suggestions D-02); the Memory tab holds no PR group.
    await expect(tab.locator("[data-memory-pr-group]")).toHaveCount(0);
    await expect(tab).toContainText("Personal");
    await expect(tab).toContainText("Outdated");
    const facts = { home: run.env.HOME, state: run.env.HIDE_STATE_DIR, socket: herdr.socket, fixture: "invented cards and scripted GitHub facts; no network", daemonPid: run.daemonPid(), capture: "candidate-window renderer via its own CDP; no OS capture" };
    const candidate = await app.evaluate(({ BrowserWindow }) => ({ pid: process.pid, executable: process.execPath, windows: BrowserWindow.getAllWindows().length, visible: BrowserWindow.getAllWindows()[0]?.isVisible() }));
    expect(candidate.windows).toBe(1);
    if (process.env.HIDE_E2E_SCREENSHOT_DIR) fs.writeFileSync(path.join(process.env.HIDE_E2E_SCREENSHOT_DIR, "candidate.json"), JSON.stringify({ candidate, ...facts }, null, 2));
    await screenshot(page, "memory-compact-dark");
    await tab.locator('[data-memory-cards] [data-memory-card="5"]').scrollIntoViewIfNeeded();
    await screenshot(page, "memory-personal-outdated-dark");
    await tab.locator('[data-memory-compare="2"]').click();
    await expect(tab.locator("[data-memory-comparison]")).toContainText("Use a pooled connection for all writes.");
    await screenshot(page, "memory-compare-dark");
    await tab.locator('[data-memory-compare="2"]').click();
    // Return to this Memory, then the light/narrow state.
    await (await showProjectRow(page, page.locator("[data-project]", { hasText: "sample" }).locator("[data-checkout]").first())).click();
    await showProjectMemory(page);
    await page.evaluate(() => { document.documentElement.classList.remove("dark"); document.documentElement.classList.add("light"); });
    await fitWindow(app, { width: 900, height: 760 });
    await showProjectMemory(page);
    await screenshot(page, "memory-compact-light-narrow");
    // Pinned, the lane has a divider that proves its minimum width (PRD
    // right-lane B6).
    await fitWindow(app, { width: 1440, height: 900 });
    await showProjectMemory(page);
    await setLanePinned(page, true);
    const divider = page.locator('[data-lane-divider="lane"]');
    for (let step = 0; step < 6; step++) await divider.press("ArrowRight");
    await expect.poll(() => page.locator("[data-lane-frame]").evaluate((element) => Math.round(element.getBoundingClientRect().width))).toBe(360);
    for (const language of ["en", "ko", "zh-CN", "ja"]) {
      await page.locator("[data-open-settings]").click();
      await page.locator("[data-interface-language]").click();
      await page.locator(`[data-language-option="${language}"]`).click();
      await expect(page.locator("html")).toHaveAttribute("lang", language);
      await page.keyboard.press("Escape");
      await showProjectMemory(page);
      await expect.poll(() => tab.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(true);
      await tab.locator('[data-memory-cards] [data-memory-card="4"]').scrollIntoViewIfNeeded();
      await expect(tab.locator('[data-memory-outdated="4"]')).toContainText("repository-ownership-and-session-verification-guide.md");
      await expect(tab.locator('[data-memory-outdated="4"] [data-memory-document]')).toHaveAttribute("title", "docs/architecture/repository-ownership-and-session-verification-guide.md");
      await expect.poll(() => tab.locator('[data-memory-outdated="4"]').evaluate((element) => element.getBoundingClientRect().height <= Number.parseFloat(getComputedStyle(element).lineHeight))).toBe(true);
      await screenshot(page, `memory-minimum-${language}`);
      await tab.locator('[data-memory-compare="2"]').click();
      await expect(tab.locator("[data-memory-comparison]")).toBeVisible();
      await expect.poll(() => tab.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(true);
      await screenshot(page, `memory-minimum-compare-${language}`);
      await tab.locator('[data-memory-compare="2"]').click();
    }
    // Optional bounded interactive inspection of this same private daemon through Aside.
    const inspection = process.env.HIDE_MEMORY_INSPECTION_FILE;
    if (inspection) {
      fs.writeFileSync(inspection, JSON.stringify({ url: page.url(), root: run.root, pid: app.process().pid }));
      await expect.poll(() => fs.existsSync(`${inspection}.done`), { timeout: 180_000 }).toBe(true);
    }
    await expect.poll(() => tab.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(true);
  } finally {
    if (app) await app.close();
    run.cleanup();
    herdr.stop();
  }
});
