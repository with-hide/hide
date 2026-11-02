// Herdr's refusal to close a primary checkout's last tab or pane, observed on a repository and worktree owned
// by this fixture, and what the operator sees when Hide closes them on either engine.
import { expect, test, type Page } from "@playwright/test";
import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { checkoutAt, tabOfPane, withControl, openProject, openWorktree } from "./daemon-actions";
import { startEngine, type EngineServer } from "./engine-fixture";
import { startHerdr } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { toPage } from "../../desktop/src/main/wirePath";
import { enterWorkspace, screenshot } from "./wire";

// Each contract has its own private engine and linked worktree; only the UI contracts start hided.
test.describe.configure({ timeout: 120_000 });

type Created = { result: { workspace: { workspace_id: string }; tab: { tab_id: string }; root_pane: { pane_id: string } } };
type Snapshot = { result: { snapshot: { tabs: { tab_id: string; workspace_id: string }[]; panes: { pane_id: string; tab_id: string }[]; workspaces: { workspace_id: string }[] } } };

// A primary checkout with one linked worktree, both opened as Herdr workspaces.
async function startRefusalFixture() {
  const herdr = await startHerdr({ agents: false });
  try {
    const repo = path.join(herdr.root, "primary");
    const linked = path.join(herdr.root, "linked");
    fs.mkdirSync(repo);
    const git = (...args: string[]) => execFileSync("git", ["-C", repo, ...args], { env: herdr.env, encoding: "utf8", stdio: "pipe" });
    git("init", "-b", "main");
    git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "--allow-empty", "-m", "fixture");
    git("worktree", "add", "-b", "linked", linked);
    const open = (at: string) => herdr.run(["worktree", "open", "--cwd", repo, "--path", at, "--no-focus"]) as Created;
    const primary = open(repo);
    const sibling = open(linked);
    const snapshot = () => (herdr.run(["api", "snapshot"]) as Snapshot).result.snapshot;
    return { herdr, repo, open, primary, sibling, snapshot };
  } catch (error) {
    herdr.stop();
    throw error;
  }
}

type Stack = { server: EngineServer; daemon: Daemon; primary: { folder: string; workspace: string; tab: string; pane: string }; linkedFolder: string };

/** A primary checkout with one linked worktree, each with its first tab, in a project the daemon opened. */
async function startStack(page: Page, label: string): Promise<Stack> {
  const server = await startEngine();
  try {
    const repo = path.join(server.root, "primary");
    const linked = path.join(server.root, "linked");
    fs.mkdirSync(repo);
    const git = (...args: string[]) => execFileSync("git", ["-C", repo, ...args], { encoding: "utf8", stdio: "pipe" });
    git("init", "-b", "main");
    git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "--allow-empty", "-m", "fixture");
    git("worktree", "add", "-b", "linked", linked);
    const daemon = await startHided(server, label);
    try {
      // The core lists a linked worktree once a client shows its repository.
      await page.goto(`${daemon.origin}/#token=${daemon.token}`);
      const primary = await withControl(daemon, (control) => openProject(control, repo, "primary"));
      await withControl(daemon, (control) => openWorktree(control, linked));
      return { server, daemon, primary, linkedFolder: toPage(fs.realpathSync.native(linked)) };
    } catch (error) {
      daemon.stop();
      throw error;
    }
  } catch (error) {
    server.stop();
    throw error;
  }
}

/** The tabs of the primary checkout, as the daemon lists them. */
const primaryTabs = (stack: Stack) => withControl(stack.daemon, (control) => control.until((rest) => checkoutAt(rest, stack.primary.folder)?.tabs, "the primary checkout's tabs", 20_000));

async function openPrimary(page: Page) {
  await enterWorkspace(page, "primary");
  await page.locator('[data-project]', { hasText: "primary" }).locator('[data-checkout][aria-label^="main"]').click();
}

// Herdr only: it reads Herdr's own refusal (`confirmation_required`) from its CLI.
test("Herdr refuses to close the last tab or pane of a primary checkout without confirmation @herdr", async () => {
  const { herdr, open, primary, sibling, snapshot } = await startRefusalFixture();
  try {
    const close = (args: string[]) => {
      const result = spawnSync(herdr.bin, args, { env: herdr.env, encoding: "utf8", timeout: 10_000 });
      return { exit: result.status, stdout: result.stdout, stderr: result.stderr };
    };
    const before = snapshot();
    const tabClose = close(["tab", "close", primary.result.tab.tab_id]);
    expect(tabClose.exit).toBe(1);
    expect(JSON.parse(tabClose.stderr).error.code).toBe("confirmation_required");
    const afterTab = snapshot();
    const secondPrimary = open(path.join(herdr.root, "primary"));
    const beforePane = snapshot();
    const paneClose = close(["pane", "close", secondPrimary.result.root_pane.pane_id]);
    expect(paneClose.exit).toBe(1);
    expect(JSON.parse(paneClose.stderr).error.code).toBe("confirmation_required");
    const afterPane = snapshot();
    const output = process.env.HIDE_E2E_SCREENSHOT_DIR;
    if (output) {
      const record = { version: execFileSync(herdr.bin, ["--version"], { encoding: "utf8" }).trim(), primary: primary.result, linked: sibling.result, before, tabClose, afterTab, beforePane, paneClose, afterPane };
      fs.writeFileSync(path.join(output, "primary-close-contract.json"), JSON.stringify(record, null, 2));
    }
    expect(JSON.stringify(afterTab)).toContain(sibling.result.workspace.workspace_id);
    expect(JSON.stringify(afterPane)).toContain(sibling.result.workspace.workspace_id);
  } finally { herdr.stop(); }
});


test("closing a primary checkout's last tab in the UI leaves a replacement tab in the same area and keeps the linked workspace", async ({ page }) => {
  const stack = await startStack(page, "replacement-close-tab");
  try {
    await openPrimary(page);
    const originalTab = page.locator(`[data-agent-tab-bar] [data-tab="${stack.primary.tab}"]`);
    await expect(originalTab).toBeVisible();
    const area = await originalTab.locator("xpath=ancestor::*[@data-agent-area-id]").getAttribute("data-agent-area-id");
    await originalTab.getByRole("button", { name: /^Close tab/ }).click();
    await expect(originalTab).toHaveCount(0);
    await expect.poll(async () => (await primaryTabs(stack)).length).toBe(1);
    const replacement = (await primaryTabs(stack))[0]!;
    expect(replacement).not.toBe(stack.primary.tab);
    const replacementTab = page.locator(`[data-agent-tab-bar] [data-tab="${replacement}"]`);
    await expect(replacementTab).toBeVisible();
    await expect(replacementTab.locator("xpath=ancestor::*[@data-agent-area-id]")).toHaveAttribute("data-agent-area-id", area);
    const linked = await withControl(stack.daemon, (control) => control.until((rest) => checkoutAt(rest, stack.linkedFolder), "the linked worktree listed", 20_000));
    expect(linked.tabs).toHaveLength(1);
    await screenshot(page, "agent-primary-replacement-tab");
  } finally { stack.daemon.stop(); stack.server.stop(); }
});

test("closing a primary checkout's last pane in the UI keeps one tab for it and the linked workspace", async ({ page }) => {
  const stack = await startStack(page, "replacement-close-pane");
  try {
    const pane = stack.primary.pane;
    await openPrimary(page);
    await page.locator(`[data-pane-view="${pane}"]`).getByRole("button", { name: /Close/ }).click();
    await withControl(stack.daemon, (control) => control.until((rest) => tabOfPane(rest, pane) === undefined || undefined, `the close of ${pane}`, 20_000));
    await expect.poll(async () => (await primaryTabs(stack)).length).toBe(1);
    const linked = await withControl(stack.daemon, (control) => control.until((rest) => checkoutAt(rest, stack.linkedFolder), "the linked worktree listed", 20_000));
    expect(linked.tabs).toHaveLength(1);
    await screenshot(page, "agent-primary-replacement-pane");
  } finally { stack.daemon.stop(); stack.server.stop(); }
});
