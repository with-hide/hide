// Closing a primary checkout's last tab or pane while a linked worktree remains, on a repository and worktree
// owned by this fixture: the tab goes away like any other and no shell comes back in its place.
import { expect, test, type Page } from "@playwright/test";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { checkoutAt, tabOfPane, withControl, openProject, openWorktree } from "./daemon-actions";
import { startEngine, type EngineServer } from "./engine-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { toPage } from "../../desktop/src/main/wirePath";
import { enterWorkspace } from "./wire";

// Each contract has its own private engine and linked worktree; only the UI contracts start hided.
test.describe.configure({ timeout: 120_000 });

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


test("closing a primary checkout's last tab in the UI closes it for good and keeps the linked workspace", async ({ page }) => {
  const stack = await startStack(page, "primary-close-tab");
  try {
    await openPrimary(page);
    const originalTab = page.locator(`[data-agent-tab-bar] [data-tab="${stack.primary.tab}"]`);
    await expect(originalTab).toBeVisible();
    await originalTab.getByRole("button", { name: /^Close tab/ }).click();
    await expect(originalTab).toHaveCount(0);
    await expect.poll(async () => (await primaryTabs(stack)).length).toBe(0);
    const linked = await withControl(stack.daemon, (control) => control.until((rest) => checkoutAt(rest, stack.linkedFolder), "the linked worktree listed", 20_000));
    expect(linked.tabs).toHaveLength(1);
  } finally { stack.daemon.stop(); stack.server.stop(); }
});

test("closing a primary checkout's last pane in the UI closes its tab and keeps the linked workspace", async ({ page }) => {
  const stack = await startStack(page, "primary-close-pane");
  try {
    const pane = stack.primary.pane;
    await openPrimary(page);
    await page.locator(`[data-pane-view="${pane}"]`).getByRole("button", { name: /Close/ }).click();
    await withControl(stack.daemon, (control) => control.until((rest) => tabOfPane(rest, pane) === undefined || undefined, `the close of ${pane}`, 20_000));
    await expect.poll(async () => (await primaryTabs(stack)).length).toBe(0);
    const linked = await withControl(stack.daemon, (control) => control.until((rest) => checkoutAt(rest, stack.linkedFolder), "the linked worktree listed", 20_000));
    expect(linked.tabs).toHaveLength(1);
  } finally { stack.daemon.stop(); stack.server.stop(); }
});
