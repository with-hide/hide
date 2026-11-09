// Agent profiles through a real private core and node (PRD agent-profiles
// B1, B3, B14, B15): with no profile the screens are as before; a profile
// added in Settings › Agents reaches the node's profile file, the start
// panel offers it, and Project settings saves it as the project's choice.
// Captures land in HIDE_E2E_SCREENSHOT_DIR, in Dark and Light, when it is named.
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { expect, type ElectronApplication, type Page } from "@playwright/test";
import { openProject, withControl } from "../../web/e2e/daemon-actions";
import { startEngine } from "../../web/e2e/engine-fixture";
import { projectOpener, showProjectRow } from "../../web/e2e/wire";
import { animationsFinished, compositorPresents } from "../../web/e2e/wait";
import { connectHome, fitWindow, isolate, launchShell, test } from "./fixture";

test.describe.configure({ timeout: 300_000 });

async function setTheme(page: Page, theme: "dark" | "light"): Promise<void> {
  await page.locator("[data-open-settings]").click();
  await page.locator('[data-settings-tab="general"]').click();
  await page.locator(`[data-theme-option="${theme}"]`).click();
  await expect(page.locator("html")).toHaveClass(new RegExp(`\\b${theme}\\b`));
  await page.keyboard.press("Escape");
  await expect(page.locator('[data-settings="true"]')).toHaveCount(0);
}

/** The candidate's window as macOS draws it. */
async function capture(app: ElectronApplication, page: Page, name: string): Promise<void> {
  const dir = process.env.HIDE_E2E_SCREENSHOT_DIR;
  if (!dir || process.platform !== "darwin") return;
  fs.mkdirSync(dir, { recursive: true });
  await animationsFinished(page);
  await compositorPresents(page);
  const source = await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0]!.getMediaSourceId());
  execFileSync("/usr/sbin/screencapture", ["-x", "-o", "-l", source.split(":")[1]!, path.join(dir, `${name}.png`)]);
}

async function openAgents(page: Page): Promise<void> {
  await page.locator("[data-open-settings]").click();
  await page.locator('[data-settings-tab="agents"]').click();
  await expect(page.locator('[data-agent-row^="local"], [data-agent-row*=":claude-code:"]').first()).toBeVisible({ timeout: 60_000 });
}

async function openStartPanel(page: Page): Promise<void> {
  await page.keyboard.press("Meta+KeyK");
  await expect(page.locator('[data-palette="Search"] [data-palette-input]')).toBeFocused();
  await page.keyboard.type("Start an agent");
  await page.locator('[data-palette-row="command:start-agent"]').click();
  await expect(page.locator("[data-start-panel]")).toBeVisible();
}

async function openWorkingStyle(page: Page): Promise<void> {
  const gear = page.locator("[data-project]", { hasText: "sample" }).locator("[data-row-settings]");
  await projectOpener(page.locator("[data-project]", { hasText: "sample" })).hover({ position: { x: 12, y: 8 } });
  await (await showProjectRow(page, gear)).click();
  const sheet = page.locator("[data-project-settings]");
  await expect(sheet).toBeVisible();
  await sheet.getByRole("tab", { name: "Working style" }).click();
}

async function closeAll(page: Page): Promise<void> {
  for (let i = 0; i < 3 && (await page.locator('[data-settings="true"], [data-project-settings], [data-start-panel], [role="dialog"]').count()) > 0; i++) await page.keyboard.press("Escape");
}

test("a profile added in the Agents settings page is offered by the start panel and kept as a project's choice", async () => {
  const engine = await startEngine();
  const run = isolate(engine, "agent-profiles");
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

    // B1: no profile, so nothing but an add button joins the screens.
    for (const theme of ["dark", "light"] as const) {
      await setTheme(page, theme);
      await openAgents(page);
      await expect(page.locator('[data-inline-profiles="claude-code"] [data-profile-add]')).toBeVisible({ timeout: 60_000 });
      await expect(page.locator("[data-profile-row]")).toHaveCount(0);
      await capture(app, page, `01-settings-agents-none-${theme}`);
      await closeAll(page);
      await openStartPanel(page);
      await expect(page.locator("[data-profile-pick]")).toHaveCount(0);
      await capture(app, page, `02-start-panel-none-${theme}`);
      await closeAll(page);
      await openWorkingStyle(page);
      await expect(page.locator("[data-project-agent-accounts]")).toHaveCount(0);
      await capture(app, page, `03-project-working-style-none-${theme}`);
      await closeAll(page);
    }

    // B3: add one through the dialog; the node keeps it and installs its hooks.
    const account = path.join(run.env.HOME!, "accounts", "claude-work");
    await openAgents(page);
    await page.locator('[data-inline-profiles="claude-code"] [data-profile-add]').click();
    await page.locator("#profile-add-name").fill("work");
    await page.locator("#profile-add-account").fill(account);
    await capture(app, page, "04-add-profile-dialog-light");
    await page.locator("[data-profile-submit]").click();
    const dialog = page.locator("[data-profile-dialog]");
    await expect.poll(async () => ((await dialog.count()) === 0 ? "closed" : (await dialog.locator("[data-profile-error], [data-tone=error], [aria-invalid=true]").count()) > 0 ? `refused: ${await dialog.innerText()}` : "open"), { timeout: 60_000 }).toBe("closed");
    const row = page.locator('[data-inline-profiles="claude-code"] [data-profile-row]').filter({ hasText: "work" });
    await expect(row).toBeVisible();
    const file = JSON.parse(fs.readFileSync(path.join(run.env.HIDE_STATE_DIR!, "agent-profiles.json"), "utf8")) as { profiles: { id: string; name: string; account?: unknown }[] };
    const work = file.profiles.find((profile) => profile.name === "work")!;
    expect(work).toBeDefined();
    expect(fs.statSync(account).isDirectory()).toBe(true);
    await closeAll(page);

    // B15: the project starts Claude Code with it.
    await openWorkingStyle(page);
    const choice = page.locator('[data-project-agent-account="claude-code"] [role="combobox"]');
    await choice.click();
    await page.getByRole("option", { name: "work", exact: true }).click();
    await expect(page.locator("[data-project-settings]")).toContainText("Saved");
    const state = JSON.parse(fs.readFileSync(path.join(run.env.HIDE_STATE_DIR!, "core-state.json"), "utf8")) as { project_settings: Record<string, { root: string; agent_profiles?: Record<string, string> }> };
    expect(Object.values(state.project_settings).find((record) => record.root === repo)?.agent_profiles).toEqual({ "claude-code": work.id });
    await closeAll(page);

    // B14: the start panel opens on the project's choice and lists every profile.
    await openStartPanel(page);
    await expect(page.locator("[data-profile-pick]")).toHaveAttribute("data-profile-pick", work.id);
    await closeAll(page);

    for (const theme of ["dark", "light"] as const) {
      await setTheme(page, theme);
      await openAgents(page);
      await expect(row).toBeVisible();
      await capture(app, page, `05-settings-agents-profile-${theme}`);
      await row.locator("button").first().click();
      await expect(page.locator('[data-profile-dialog="edit"] [data-profile-env]')).toHaveAttribute("data-profile-env", "true");
      await capture(app, page, `06-edit-profile-dialog-${theme}`);
      await closeAll(page);
      await openStartPanel(page);
      await page.locator("[data-profile-pick]").click();
      await expect(page.locator(`[data-profile-pick-option="${work.id}"]`)).toBeVisible();
      await capture(app, page, `07-start-panel-profile-menu-${theme}`);
      await closeAll(page);
      await openWorkingStyle(page);
      await expect(page.locator('[data-project-agent-account-value]')).toHaveAttribute("data-project-agent-account-value", work.id);
      await capture(app, page, `08-project-agent-accounts-${theme}`);
      await closeAll(page);
    }

    // B4, B27 in Korean: the machine's refusal is worded by the screen, and
    // the delete names the project that goes back to the agent's default.
    await page.locator("[data-open-settings]").click();
    await page.locator('[data-settings-tab="general"]').click();
    await page.locator("[data-interface-language]").click();
    await page.locator('[data-language-option="ko"]').click();
    await expect(page.locator("[data-interface-language]")).toHaveAttribute("data-interface-language", "ko");
    await page.locator('[data-settings-tab="agents"]').click();
    await page.locator('[data-inline-profiles="claude-code"] [data-profile-add]').click();
    await page.locator("#profile-add-name").fill("work");
    await page.locator("#profile-add-account").fill(path.join(run.env.HOME!, "accounts", "claude-other"));
    await page.locator("[data-profile-submit]").click();
    await expect(page.locator("[data-profile-dialog]")).toContainText("이 에이전트에 같은 이름의 프로필이 이미 있어요.", { timeout: 60_000 });
    await capture(app, page, "09-add-refused-ko-light");
    await page.keyboard.press("Escape");
    await expect(page.locator("[data-profile-dialog]")).toHaveCount(0);
    await row.locator("button").first().click();
    await expect(page.locator('[data-profile-dialog="edit"] [data-profile-env]')).toHaveAttribute("data-profile-env", "true");
    await page.locator("[data-profile-delete]").click();
    await expect(page.locator("[data-profile-delete-projects]")).toContainText("sample", { timeout: 60_000 });
    await capture(app, page, "10-delete-check-ko-light");
    await closeAll(page);
  } finally {
    if (app) await app.close();
    run.cleanup();
  }
});
