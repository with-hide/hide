// Real core, private state and pinned Herdr. Memory UI has no platform-specific behavior.
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { expect, type ElectronApplication } from "@playwright/test";
import { startHerdr } from "../../web/e2e/herdr-fixture";
import { countSent, showProjectRow, showTool } from "../../web/e2e/wire";
import { isolate, launch, fitWindow, screenshot, test } from "./fixture";
import { seedReconciliation } from "./memory-reconcile-fixture";
import { progressGithub } from "./memory-progress-fixture";

test("Memory compares sources, limits a batch and groups a confirmed PR", async () => {
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
    const sent = new Map<string, Record<string, unknown>>();
    countSent(page, sent);
    await (await showProjectRow(page, page.locator("[data-project]", { hasText: "sample" }).locator("[data-checkout]").first())).click();
    await showTool(page, "memory");
    await expect(page.locator('[data-memory="off"]')).toBeVisible();
    await page.locator('[data-memory-switch="off"]').click();
    await expect(page.locator('[data-memory="on"]')).toBeVisible({ timeout: 30000 });
    seedReconciliation(run, "acme/memory-fixture");
    await page.reload();
    await expect(page.locator('[data-workspace-screen]')).toBeVisible();
    await expect(page.locator('[data-connection]')).toHaveCount(0);
    await showTool(page, "memory");
    const tab = page.locator('[data-lane="memory"]');
    await expect(tab.locator('[data-memory="on"]')).toBeVisible();
    await expect(tab.locator('[data-memory-pr-group="72"]')).toContainText("m9 · m10");
    await expect(tab).toContainText("Personal");
    await expect(tab).toContainText("Outdated");
    await expect(tab.locator('[data-memory-select="3"]')).toBeDisabled();
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
    await tab.locator('[data-memory-select="1"]').click();
    await tab.locator('[data-memory-select="2"]').click();
    await expect(tab.locator('[data-memory-selection="2"]')).toBeVisible();
    await expect(tab.locator('[data-memory-promote="1"]')).toHaveCount(0);
    await expect(tab.locator('[data-memory-promote="2"]')).toHaveCount(0);
    await expect(tab.locator('[data-memory-promote="3"]')).toBeVisible();
    await screenshot(page, "memory-two-selected-dark");
    await tab.locator('[data-memory-promote="3"]').click();
    await expect(tab.locator('[data-memory-select="3"]')).toBeEnabled();
    for (const number of [6, 7, 8]) await tab.locator(`[data-memory-select="${number}"]`).click();
    await expect(tab.locator('[data-memory-selection="5"]')).toBeVisible();
    await expect(tab.locator('[data-memory-select="3"]')).toBeDisabled();
    await tab.locator('[data-memory-pr-group="72"] [data-memory-row-toggle="9"]').click();
    await expect(tab.locator('[data-memory-progress-detail="9"] [data-memory-preview]')).toHaveCount(2);
    await expect(tab.locator('[data-memory-pr-group="72"]')).toContainText("Awaiting review");
    await tab.locator('[data-memory-pr-group="72"] [data-memory-open-pr="72"]').click();
    // The PR button asks the core for the PR's card (PRD work-tab-pr-lens B20): this
    // fixture lists PR 72, so its card opens in the lane's 보기 slot (PRD
    // sidebar-task-lens B14), and no Overview opens.
    await expect.poll(() => sent.get("pr_open")).toMatchObject({ pr_number: 72 });
    await expect(page.locator('aside[data-lane="detail"][data-detail="pull_request"] [data-pr-card="72"]')).toBeVisible();
    await expect(page.locator("[data-overview-page]")).toHaveCount(0);
    // Return to this Memory, then the light/narrow state.
    await (await showProjectRow(page, page.locator("[data-project]", { hasText: "sample" }).locator("[data-checkout]").first())).click();
    await showTool(page, "memory");
    await page.evaluate(() => { document.documentElement.classList.remove("dark"); document.documentElement.classList.add("light"); });
    await fitWindow(app, { width: 900, height: 760 });
    await showTool(page, "memory");
    await screenshot(page, "memory-compact-light-narrow");
    // Pinned, the lane has a divider that proves its minimum width (PRD
    // right-lane B6).
    await fitWindow(app, { width: 1440, height: 900 });
    await showTool(page, "memory");
    await page.locator('[data-lane-pin="off"]').click();
    const divider = page.locator('[data-lane-divider="lane"]');
    for (let step = 0; step < 6; step++) await divider.press("ArrowRight");
    await expect.poll(() => page.locator("[data-lane-frame]").evaluate((element) => Math.round(element.getBoundingClientRect().width))).toBe(360);
    for (const language of ["en", "ko", "zh-CN", "ja"]) {
      await page.locator("[data-open-settings]").click();
      await page.locator("[data-interface-language]").click();
      await page.locator(`[data-language-option="${language}"]`).click();
      await expect(page.locator("html")).toHaveAttribute("lang", language);
      await page.keyboard.press("Escape");
      await showTool(page, "memory");
      await expect.poll(() => tab.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(true);
      await expect(tab.locator('[data-memory-pr-group="72"]')).toContainText("72");
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
