// The Project tool's results column on an isolated pinned Herdr and hided
// (PRD project-panel-results): a private repository with two worktrees, each
// holding a build folder. Opening the tool counts the project's worktrees
// (B1) and measures the project's disk while it shows (B18, B19), and closing
// the tool ends the watch, so the core measures nothing nobody looks at (B20). The run prints
// the daemon's CPU time and wall time for that one measurement, and Light and
// Dark captures land in HIDE_E2E_SCREENSHOT_DIR.

import { expect, test } from "@playwright/test";
import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { startHerdr } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { chooseTheme, countSent, screenshot, showProjectRow, showTool } from "./wire";
import { initGithubRepository } from "./github-remote";

test.describe.configure({ timeout: 180_000 });

function git(cwd: string, args: string[]): string {
  return execFileSync("git", ["-c", "user.name=e2e", "-c", "user.email=e2e@example.invalid", "-c", "init.defaultBranch=main", "-c", "commit.gpgsign=false", ...args], { cwd, encoding: "utf8" });
}

/** The daemon's CPU time so far, user and system, in milliseconds. */
function cpuMs(pid: number): number {
  const out = spawnSync("ps", ["-o", "time=", "-p", String(pid)], { encoding: "utf8" }).stdout.trim();
  const parts = out.split(/[:]/).map(Number);
  const seconds = parts.reduce((sum, part) => sum * 60 + part, 0);
  return Math.round(seconds * 1000);
}

// @platform: disk measuring on the platform's filesystem.
test("the Project tool counts the worktrees, and measures the project's disk only while it shows (project-panel-results B1, B18-B20)", { tag: "@platform" }, async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 1000 });
  const herdr = await startHerdr();
  let daemon: Daemon | null = null;
  try {
    const repo = path.join(herdr.root, "repo");
    fs.mkdirSync(repo);
    initGithubRepository(repo);
    fs.writeFileSync(path.join(repo, "README.md"), "# repo\n");
    fs.writeFileSync(path.join(repo, ".gitignore"), "target/\n");
    git(repo, ["add", "."]);
    git(repo, ["commit", "-m", "initial"]);
    const origin = path.join(herdr.root, "origin.git");
    git(herdr.root, ["init", "--bare", origin]);
    git(repo, ["remote", "add", "origin", origin]);
    git(repo, ["push", "-q", "origin", "main"]);
    git(repo, ["remote", "set-head", "origin", "main"]);
    const first = path.join(herdr.root, "repo-first");
    const second = path.join(herdr.root, "repo-second");
    git(repo, ["worktree", "add", "-q", "-b", "prd/first", first]);
    git(repo, ["worktree", "add", "-q", "-b", "prd/second", second]);
    for (const dir of [repo, first, second]) {
      fs.mkdirSync(path.join(dir, "target", "debug"), { recursive: true });
      fs.writeFileSync(path.join(dir, "target", "debug", "blob"), Buffer.alloc(256 * 1024, 1));
    }
    herdr.run(["workspace", "create", "--cwd", repo, "--label", "repo", "--env", `PATH=${herdr.fixturePath}`, "--no-focus"]);

    daemon = await startHided(herdr, "project-panel");
    const last = new Map<string, Record<string, unknown>>();
    const sent = countSent(page, last);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await (await showProjectRow(page, page.locator("[data-project]", { hasText: "repo" }).locator("[data-checkout]").first())).click();
    await chooseTheme(page, "light");

    const cpuBefore = cpuMs(daemon.pid);
    const startedAt = Date.now();
    await showTool(page, "project");
    const tool = page.locator("aside[data-lane=\"project\"]");
    // The tool asks the core to watch this one project's disk (B20).
    await expect.poll(() => (last.get("disk_watch")?.workspace_ids as string[] | undefined)?.length ?? 0, { timeout: 10_000 }).toBe(1);
    const disk = tool.locator("[data-project-disk]");
    await expect(disk).toHaveAttribute("data-project-disk", "measured", { timeout: 60_000 });
    const measuredMs = Date.now() - startedAt;
    const cpuSpentMs = cpuMs(daemon.pid) - cpuBefore;
    console.log(JSON.stringify({ kind: "project_panel_disk_measure", wall_ms: measuredMs, daemon_cpu_ms: cpuSpentMs, checkouts: 3 }));

    // B1: the count line names the project's worktrees.
    await expect(tool.locator('[data-project-count="worktrees"] [data-count-value]')).toHaveAttribute("data-count-value", "3", { timeout: 30_000 });
    // B18: the disk line reads the project's measured size.
    await expect(disk).toContainText(/\d/);
    await screenshot(page, "project-panel-light");
    await chooseTheme(page, "dark");
    await screenshot(page, "project-panel-dark");

    // B20: closing the tool ends the watch.
    const watches = sent.get("disk_watch") ?? 0;
    await page.locator('[data-lane-tool="project"]').click();
    await expect.poll(() => sent.get("disk_watch") ?? 0).toBeGreaterThan(watches);
    expect((last.get("disk_watch")?.workspace_ids as string[] | undefined) ?? []).toEqual([]);
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});
