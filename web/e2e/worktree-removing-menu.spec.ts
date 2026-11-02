// A checkout row mid-removal: with the Delete worktree dialog hidden while Git
// still works, a right-click on the dimmed row draws no empty
// popover, and the row leaves once the removal finishes.

import { expect, test } from "@playwright/test";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { startFixture } from "./engine-fixture";
import { openProject, withControl } from "./daemon-actions";
import { startHided, type Daemon } from "./hided-fixture";
import { screenshot, showProjectRow } from "./wire";

test.describe.configure({ timeout: 180_000 });

const BRANCH = "feature/slow";

function git(cwd: string, args: string[]): string {
  return execFileSync("git", ["-c", "user.name=e2e", "-c", "user.email=e2e@example.invalid", "-c", "init.defaultBranch=main", ...args], { cwd, encoding: "utf8" });
}

/**
 * A `git` first on the daemon's PATH that holds `git worktree remove`, as a
 * removal Git does in place does, until `release` is called, and runs the real
 * Git for everything else. The hold gives up after 12 seconds, inside the
 * host's 15-second Git deadline, so a test that fails early still ends.
 */
function heldGit(root: string, fixturePath: string): { path: string; release: () => void } {
  const real = execFileSync("sh", ["-c", "command -v git"], { encoding: "utf8" }).trim();
  const bin = path.join(root, "held-git");
  const hold = path.join(root, "hold-worktree-remove");
  fs.mkdirSync(bin);
  fs.writeFileSync(hold, "");
  fs.writeFileSync(
    path.join(bin, "git"),
    `#!/bin/sh\ncase " $* " in *" worktree remove "*) i=0; while [ -e "${hold}" ] && [ $i -lt 120 ]; do sleep 0.1; i=$((i + 1)); done ;; esac\nexec "${real}" "$@"\n`,
    { mode: 0o755 },
  );
  return { path: `${bin}:${fixturePath}`, release: () => fs.rmSync(hold, { force: true }) };
}

test("a checkout row being deleted opens no empty menu", async ({ page }) => {
  await page.setViewportSize({ width: 1400, height: 900 });
  const herdr = await startFixture({ agents: false });
  let daemon: Daemon | null = null;
  try {
    const repo = path.join(herdr.root, "repo");
    fs.mkdirSync(repo);
    git(repo, ["init"]);
    fs.writeFileSync(path.join(repo, "README.md"), "# repo\n");
    git(repo, ["add", "README.md"]);
    git(repo, ["commit", "-m", "initial"]);
    git(repo, ["update-ref", "refs/remotes/origin/main", "HEAD"]);
    git(repo, ["symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/main"]);
    const worktree = path.join(herdr.root, "repo-slow");
    git(repo, ["worktree", "add", "-b", BRANCH, worktree]);
    // Unmerged, so the row stays among the active checkouts rather than folding into Inactive.
    git(worktree, ["commit", "--allow-empty", "-m", "not merged anywhere"]);

    const held = heldGit(herdr.root, herdr.fixturePath);
    daemon = await startHided(herdr, "worktree-removing-menu", undefined, { PATH: held.path });
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await expect(page.locator("[data-main-screen]")).toBeVisible({ timeout: 20_000 });
    // A pane in main keeps the project listed once the worktree goes; the core lists the worktree, with no tab of its own, once a client shows its repository.
    await withControl(daemon, (control) => openProject(control, repo, "repo"));

    const emptyFold = page.locator("[data-empty-worktrees]");
    // The project runs no agent, so it waits under No agents (PRD sidebar-titlebar-refresh B1).
    await showProjectRow(page, emptyFold);
    await expect(emptyFold).toHaveAttribute("aria-expanded", "false");
    await emptyFold.click();
    const feature = page.locator("[data-checkout-row]").filter({ has: page.locator(`[data-checkout][aria-label^="${BRANCH}"]`) });
    await expect(feature).toBeVisible({ timeout: 30_000 });
    await feature.locator("[data-checkout-menu]").click({ button: "right" });
    await page.getByRole("menu", { name: `${BRANCH} actions` }).locator('[data-menu-item="delete_worktree"]').click();

    const dialog = page.locator("[data-delete-worktree]");
    const confirm = dialog.locator("[data-delete-confirm]");
    await expect(confirm).toBeEnabled({ timeout: 30_000 });
    await confirm.click();
    await expect(dialog.locator('[data-delete-phase="removing"]')).toBeVisible({ timeout: 30_000 });
    // The operator stops watching; the removal goes on under the row.
    await dialog.locator("[data-delete-cancel]").click();
    await expect(dialog).toHaveCount(0);
    await expect(feature).toHaveAttribute("data-checkout-removing", "true");

    await feature.locator("[data-checkout-menu]").click({ button: "right" });
    await screenshot(page, "worktree-removing-right-click");
    // Before the fix the root opened with nothing to draw, so the open state is the check.
    await expect(feature.locator("[data-checkout-menu]")).toHaveAttribute("data-state", "closed");
    // Still mid-removal, so the right-click above landed on the dimmed row.
    await expect(feature).toHaveAttribute("data-checkout-removing", "true");

    held.release();

    await expect(feature).toHaveCount(0, { timeout: 30_000 });
    expect(fs.existsSync(worktree)).toBe(false);
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});
