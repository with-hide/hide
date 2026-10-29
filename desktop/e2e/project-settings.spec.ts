// Scoped saves, failure recovery and restart use a real private core and
// filesystem. Provider discovery is the fixture's PATH, never the account's.
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { expect, type ElectronApplication } from "@playwright/test";
import { initGithubRepository } from "../../web/e2e/github-remote";
import { startHerdr } from "../../web/e2e/herdr-fixture";
import { fixtureProgram } from "../../web/e2e/platform-fixture";
import { showProjectRow } from "../../web/e2e/wire";
import { projectRow } from "../../web/e2e/overview-entry";
import { animationsFinished, compositorPresents } from "../../web/e2e/wait";
import { captureNativeWindow } from "./native-window";
import { fitWindow, isolate, launch, screenshot, test } from "./fixture";

test("Project settings keep confirmed values through a failed save and restart", async () => {
  const herdr = await startHerdr({ agents: false });
  const run = isolate(herdr, "project-settings");
  fs.mkdirSync(run.env.XDG_STATE_HOME!, { recursive: true }); run.env.HIDE_STATE_DIR = path.join(run.env.XDG_STATE_HOME!, "hide"); // The account's own folder: only a core there writes the guardrails policy.
  let app: ElectronApplication | null = null;
  try {
    const repo = path.join(run.env.HOME!, "projects", "sample");
    fs.mkdirSync(repo);
    initGithubRepository(repo);
    // gh as a logged-out account answers it: exit 4 and its sign-in hint.
    fixtureProgram(path.join(herdr.root, "bin"), "gh", `console.error("To get started with GitHub CLI, please run:  gh auth login"); process.exit(4);`);
    fs.writeFileSync(path.join(repo, "README.md"), "Fixture\n");
    execFileSync("git", ["add", "README.md"], { cwd: repo });
    execFileSync("git", ["-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "-qm", "Initial"], { cwd: repo });
    herdr.run(["workspace", "create", "--cwd", repo, "--label", "sample", "--env", `PATH=${herdr.fixturePath}`, "--no-focus"]);
    let launched = await launch(run.env);
    app = launched.app;
    let page = launched.page;
    const gear = () => page.locator('[data-project]', { hasText: "sample" }).locator('[data-row-settings]');
    await (await showProjectRow(page, gear())).click();
    let sheet = page.locator('[data-project-settings]');
    await expect(sheet).toBeVisible();
    await expect(sheet.getByRole("tab")).toHaveText(["Connections", "Working style", "Memory"]);
    // A GitHub repository whose read fails stays GitHub under Auto and says
    // so, rather than moving its issues to Local.
    await expect(sheet).toContainText("Automatic (GitHub)");
    await expect(sheet).toContainText("GitHub read failed");
    await captureNativeWindow(app, "project-settings-connections-native", { state: run.env.HIDE_STATE_DIR, home: run.env.HOME, socket: herdr.socket, daemonPid: run.daemonPid(), build: "worktree debug", provider: "fixture gh logged out, GitHub remote" });
    await sheet.getByRole("combobox", { name: "Issue source", exact: true }).click();
    await page.getByRole("option", { name: "Local", exact: true }).click();
    await expect(sheet).toContainText("Saved");
    const stateFile = path.join(run.env.HIDE_STATE_DIR!, "core-state.json");
    const stored = () => JSON.parse(fs.readFileSync(stateFile, "utf8")) as { project_settings: Record<string, { device_id: string; root: string; issue_source?: string; new_work?: string }> };
    await expect.poll(() => Object.values(stored().project_settings).find((record) => record.root === repo)?.issue_source).toBe("local");
    const connection = sheet.getByRole("combobox", { name: "GitHub repository", exact: true });
    await connection.click();
    await page.getByRole("option", { name: "Choose repository", exact: true }).click();
    await sheet.getByRole("textbox", { name: "owner/repository" }).fill("example/private-fixture");
    await sheet.getByRole("button", { name: "Apply", exact: true }).click();
    await expect(connection).toContainText("Choose repository");
    await expect(sheet).toContainText("Saved");
    const repositoryBackup = `${stateFile}.before-repository-failure`;
    fs.renameSync(stateFile, repositoryBackup);
    fs.mkdirSync(stateFile);
    try {
      await connection.click();
      await page.getByRole("option", { name: "Detect automatically", exact: true }).click();
      await expect(sheet).toContainText("Couldn't save");
      await expect(connection).toContainText("Choose repository");
      await expect(sheet).toContainText("example/private-fixture");
      await screenshot(page, "project-settings-repository-save-failed");
    } finally { fs.rmdirSync(stateFile); fs.renameSync(repositoryBackup, stateFile); }
    await sheet.getByRole("button", { name: "Try again" }).click();
    await expect(connection).toContainText("Detect automatically");
    await sheet.getByRole("tab", { name: "Working style" }).click();
    // A protection is Saved once this machine's hooks can read it (PRD agent-guardrails B3, B31).
    const guarded = () => (JSON.parse(fs.readFileSync(path.join(run.env.HOME!, ".hide", "agent-hooks", "guardrails.json"), "utf8")) as { projects: { root: string; rules: { files: boolean; paths: string[] } }[] }).projects;
    await expect.poll(() => guarded()).toEqual([]);
    await sheet.getByRole("switch", { name: "Guard files", exact: true }).click();
    await expect(sheet.locator('[data-project-guard="files"]')).toContainText("Saved");
    expect(guarded().map((project) => [project.root, project.rules.files, project.rules.paths])).toEqual([[repo, true, [".env", ".env.*", "*.pem", "*.key", "~/.ssh/"]]]);
    // Escape leaves the path field and keeps the sheet.
    await sheet.getByRole("button", { name: "Add path" }).click(); await page.keyboard.press("Escape");
    await expect(sheet.getByRole("button", { name: "Add path" })).toBeFocused();
    const location = sheet.getByRole("combobox", { name: "Start new work in" });
    await expect(location).toContainText("Existing checkout");
    // A real atomic-replacement failure, confined to this fixture's state file.
    const backup = `${stateFile}.before-failure`;
    fs.renameSync(stateFile, backup);
    fs.mkdirSync(stateFile);
    try {
      await location.click();
      await page.getByRole("option", { name: "New worktree", exact: true }).click();
      await expect(sheet).toContainText("Couldn't save");
      await expect(location).toContainText("Existing checkout");
      await screenshot(page, "project-settings-save-failed");
    } finally {
      fs.rmdirSync(stateFile);
      fs.renameSync(backup, stateFile);
    }
    await sheet.getByRole("button", { name: "Try again" }).click();
    await expect(location).toContainText("New worktree");
    await expect(sheet).toContainText("Saved");
    await expect.poll(() => Object.values(stored().project_settings).find((record) => record.root === repo)?.new_work).toBe("worktree");
    await captureNativeWindow(app, "project-settings-work-native", { state: run.env.HIDE_STATE_DIR, home: run.env.HOME, socket: herdr.socket, daemonPid: run.daemonPid(), build: "worktree debug", provider: "fixture discovery, no external GitHub" });
    await page.keyboard.press("Escape");
    await expect(sheet).toHaveCount(0);
    await expect(gear()).toBeFocused();
    // CI's work area can be a narrow window. Opening Memory must call the
    // lane forward on the Memory tool there too.
    await fitWindow(app, { width: 1024, height: 681 });
    await gear().click();
    await sheet.getByRole("tab", { name: "Memory", exact: true }).click();
    await sheet.getByRole("button", { name: "Open project Memory" }).click();
    await expect(sheet).toHaveCount(0);
    await expect(page.locator('aside[data-lane="memory"] [data-memory]')).toBeVisible();
    await captureNativeWindow(app, "project-settings-memory-open-native", { build: "worktree debug", project: repo, daemonPid: run.daemonPid() });
    await app.close();
    app = null;
    expect(run.hide(["stop"]).status).toBe(0);
    launched = await launch(run.env);
    app = launched.app;
    page = launched.page;
    await (await showProjectRow(page, gear())).click();
    sheet = page.locator('[data-project-settings]');
    await sheet.getByRole("tab", { name: "Working style" }).click();
    await expect(sheet.getByRole("combobox", { name: "Start new work in" })).toContainText("New worktree");
    // B18: project + opens the same confirmation, never starts blindly.
    await page.keyboard.press("Escape");
    await page.locator('[data-project]', { hasText: "sample" }).locator('[data-row-start]').click();
    const confirmation = page.locator('[data-start-worktree]');
    await expect(confirmation).toBeVisible();
    await expect(confirmation.getByRole("combobox")).toContainText("main");
    await expect(confirmation.getByRole("textbox")).toHaveValue("");
  } finally {
    if (app) await app.close();
    run.cleanup();
    herdr.stop();
  }
});

test("Project settings fit a narrow native window in every language and theme", async () => {
  const herdr = await startHerdr({ agents: false });
  const run = isolate(herdr, "project-settings-narrow");
  let app: ElectronApplication | null = null;
  try {
    const repo = path.join(run.env.HOME!, "projects", "sample");
    fs.mkdirSync(repo);
    execFileSync("git", ["init", "-q", "-b", "main"], { cwd: repo });
    fs.writeFileSync(path.join(repo, "README.md"), "Fixture\n");
    execFileSync("git", ["add", "README.md"], { cwd: repo });
    execFileSync("git", ["-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "-qm", "Initial"], { cwd: repo });
    herdr.run(["workspace", "create", "--cwd", repo, "--label", "sample 프로젝트 이름이 아주 길어지는 경우에도 설정 열기", "--env", `PATH=${herdr.fixturePath}`, "--no-focus"]);
    const launched = await launch(run.env);
    app = launched.app;
    const page = launched.page;
    const gear = () => page.locator('[data-project]', { hasText: "sample" }).locator('[data-row-settings]');
    await (await showProjectRow(page, gear())).click();
    const sheet = page.locator('[data-project-settings]');
    // Actual narrow window and largest supported interface text in all scripts.
    await fitWindow(app, { width: 720, height: 480 });
    for (const language of ["en", "ko", "ja", "zh-CN"]) {
      await page.keyboard.press("Escape");
      await page.locator('[data-open-settings]').click();
      await page.locator('[data-interface-language]').click();
      await page.locator(`[data-language-option="${language}"]`).click();
      await expect(page.locator("html")).toHaveAttribute("lang", language);
      // The closing select hands the keyboard back to its trigger from a
      // timer; wait for it, or that late hand-back takes the slider's End.
      await expect(page.locator("[data-interface-language]")).toBeFocused();
      await page.locator('[data-settings-tab="general"]').click();
      await page.locator('[data-font-size] [role="slider"]').focus();
      await page.keyboard.press("End");
      await expect(page.locator('[data-font-size] [role="slider"]')).toHaveAttribute("aria-valuenow", "17");
      await page.keyboard.press("Escape");
      await gear().click();
      const tabs = sheet.getByRole("tab");
      await tabs.first().focus();
      await page.keyboard.press("ArrowDown");
      await expect(tabs.nth(1)).toHaveAttribute("data-state", "active");
      for (const theme of ["light", "dark"]) {
        await page.keyboard.press("Escape");
        await page.locator('[data-open-settings]').click();
        await page.locator('[data-settings-tab="general"]').click();
        await page.locator(`[data-theme-option="${theme}"]`).click();
        await expect(page.locator("html")).toHaveClass(new RegExp(`\\b${theme}\\b`));
        await page.keyboard.press("Escape");
        await gear().click();
        for (let index = 0; index < 3; index++) {
          await tabs.nth(index).click();
          await sheet.locator("img").evaluateAll(async (images) => { await Promise.all(images.map((image) => (image as HTMLImageElement).decode())); });
          await animationsFinished(page);
          await compositorPresents(page);
          await screenshot(page, `project-settings-${language}-${theme}-${index}-narrow`);
          const overflow = await sheet.evaluate((node) => {
            const panel = node.querySelector('[role=tabpanel]')!;
            const nav = node.querySelector("nav")!;
            return { dialog: node.scrollWidth - node.clientWidth, panel: panel.scrollWidth - panel.clientWidth, nav: nav.scrollHeight - nav.clientHeight };
          });
          console.log({ language, theme, index, overflow });
          expect(overflow).toEqual({ dialog: 0, panel: 0, nav: 0 });
          if (language === "ja" && theme === "light" && index === 2) await captureNativeWindow(app, "project-settings-ja-light-memory-native", { build: "worktree debug", daemonPid: run.daemonPid(), themeRoute: "global appearance settings", imagesDecoded: true });
        }
      }
    }
    await captureNativeWindow(app, "project-settings-narrow-native", { build: "worktree debug", daemonPid: run.daemonPid(), width: 720, height: 480, scale: 17 / 13 });
  } finally {
    if (app) await app.close();
    run.cleanup();
    herdr.stop();
  }
});

test("Project and Overview menus return Settings focus to their own trigger", async () => {
  const herdr = await startHerdr({ agents: false });
  const run = isolate(herdr, "project-settings-menus");
  let app: ElectronApplication | null = null;
  try {
    const repo = path.join(run.env.HOME!, "projects", "sample");
    fs.mkdirSync(repo);
    execFileSync("git", ["init", "-q", "-b", "main"], { cwd: repo });
    fs.writeFileSync(path.join(repo, "README.md"), "Fixture\n");
    execFileSync("git", ["add", "README.md"], { cwd: repo });
    execFileSync("git", ["-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "-qm", "Initial"], { cwd: repo });
    herdr.run(["workspace", "create", "--cwd", repo, "--label", "sample", "--env", `PATH=${herdr.fixturePath}`, "--no-focus"]);
    const launched = await launch(run.env);
    app = launched.app;
    const page = launched.page;
    const more = page.locator('[data-project]', { hasText: "sample" }).locator('[data-row-more]');
    await (await showProjectRow(page, more)).focus();
    await page.keyboard.press("Enter");
    await page.locator('[data-menu-item="project_settings"]').focus();
    await page.keyboard.press("Enter");
    await expect(page.locator('[data-project-settings]')).toBeVisible();
    // A repository with no GitHub remote reads Local under Auto.
    await expect(page.locator('[data-project-settings]')).toContainText("No GitHub repository detected");
    await expect(page.locator('[data-project-settings]')).toContainText("Automatic (Local)");
    await page.keyboard.press("Escape");
    await expect(more).toBeFocused();
    // The Overview's Projects tab: a project's home carries Project settings.
    await (await projectRow(page, "sample")).getByRole("button").first().click();
    const panelSettings = page.locator("[data-overview-project-home] [data-project-panel-settings]");
    await panelSettings.focus();
    await page.keyboard.press("Enter");
    await expect(page.locator('[data-project-settings]')).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(panelSettings).toBeFocused();
    await captureNativeWindow(app, "project-settings-overview-focus-native", { build: "worktree debug", daemonPid: run.daemonPid(), menuFocusReturned: true });
  } finally {
    if (app) await app.close();
    run.cleanup();
    herdr.stop();
  }
});
