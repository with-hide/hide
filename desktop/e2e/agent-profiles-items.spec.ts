// The start panel's agent items on a real private core and node (PRD
// agent-profiles-phase2 B2-B4): a profile added in Settings › Agents
// is one item beside its agent's built-in one, with no profile menu, and the
// trigger names the item chosen. Its own fixture, apart from the Settings
// journey in agent-profiles.spec.ts. Captures land in HIDE_E2E_SCREENSHOT_DIR.
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { expect, type ElectronApplication, type Page } from "@playwright/test";
import { chord } from "../../web/e2e/chords";
import { openProject, withControl } from "../../web/e2e/daemon-actions";
import { startEngine } from "../../web/e2e/engine-fixture";
import { animationsFinished, compositorPresents } from "../../web/e2e/wait";
import { connectHome, fitWindow, isolate, launchShell, test } from "./fixture";

test.describe.configure({ timeout: 300_000 });

async function capture(app: ElectronApplication, page: Page, name: string): Promise<void> {
  const dir = process.env.HIDE_E2E_SCREENSHOT_DIR;
  if (!dir || process.platform !== "darwin") return;
  fs.mkdirSync(dir, { recursive: true });
  await animationsFinished(page);
  await compositorPresents(page);
  const source = await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0]!.getMediaSourceId());
  execFileSync("/usr/sbin/screencapture", ["-x", "-o", "-l", source.split(":")[1]!, path.join(dir, `${name}.png`)]);
}

/** Adds a profile of Claude Code with its own account folder through Settings › Agents, as an operator does, and returns to the shell. */
async function addProfile(page: Page, name: string, account: string): Promise<void> {
  await page.locator("[data-open-settings]").click();
  await page.locator('[data-settings-tab="agents"]').click();
  await page.locator('[data-inline-profiles="claude-code"] [data-profile-add]').click({ timeout: 60_000 });
  await page.locator("#profile-add-name").fill(name);
  await page.locator("#profile-add-account").fill(account);
  await page.locator("[data-profile-submit]").click();
  await expect(page.locator("[data-profile-dialog]")).toHaveCount(0, { timeout: 60_000 });
  await expect(page.locator('[data-inline-profiles="claude-code"] [data-profile-row]').filter({ hasText: name })).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.locator('[data-settings="true"]')).toHaveCount(0);
}

// @platform: the account folder is a native path the node checks and makes private by its system's rule.
test("the start panel lists a registered profile as an item and names the chosen one", { tag: "@platform" }, async () => {
  const engine = await startEngine();
  const run = isolate(engine, "agent-profiles-items");
  let app: ElectronApplication | null = null;
  try {
    const repo = path.join(run.env.HOME!, "projects", "sample");
    fs.mkdirSync(repo);
    execFileSync("git", ["init", "-q", "-b", "main"], { cwd: repo });
    await withControl(await connectHome(run), (control) => openProject(control, repo, "sample"));
    const launched = await launchShell(run.env);
    app = launched.app;
    const page = launched.page;
    await fitWindow(app, { width: 1280, height: 860 });
    await addProfile(page, "work", path.join(run.env.HOME!, "accounts", "claude-work"));
    await addProfile(page, "side", path.join(run.env.HOME!, "accounts", "claude-side"));
    await page.keyboard.press(chord("search", "electron"));
    await page.keyboard.type("Start an agent");
    await page.locator('[data-palette-row="command:start-agent"]').click();
    await expect(page.locator("[data-start-panel]")).toBeVisible();
    await expect(page.locator("[data-profile-pick]")).toHaveCount(0);
    await page.locator("[data-agent-item]").click();
    const items = page.locator("[data-agent-item-option^='claude/']");
    await expect(items).toHaveCount(3);
    await expect(items.nth(0)).toHaveText(/^Claude Code(?! ·)/);
    await expect(items.nth(1)).toContainText("Claude Code · work");
    await expect(items.nth(2)).toContainText("Claude Code · side");
    await capture(app, page, "items-open");
    await page.locator('[data-agent-item-option="claude/default"]').click();
    await expect(page.locator("[data-agent-item]")).toHaveText("Claude Code");
    await capture(app, page, "default-chosen");
  } finally {
    if (app) await app.close();
    run.cleanup();
  }
});
