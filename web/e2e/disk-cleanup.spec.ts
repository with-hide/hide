// The Overview's disk cleanup sheet on an isolated engine and hided (PRD
// disk-layers): a private repository with a merged worktree and one that is
// not, each holding the same ignored folders a build leaves (`target/` with a
// CACHEDIR.TAG, `node_modules/`, `dist/` beside a package.json, and an
// `agents/runs` folder no rule knows). The disk number opens the sheet with
// its layers on hover (B1), the table sorts and folds and reads a skeleton
// until measured (B4, B5), the other layer has no checkbox (B8), a cache-only
// cleanup runs at once and leaves what git tracks and what no rule names
// (B7, B17, B21), a worktree goes through one confirmation and the folder and
// its registration are gone while its branch stays (B15, B18, B21), and a
// press is one core event (B23, B27). Every deletion happens in the fixture
// root; nothing points at an operator project (CONTRIBUTING.md). Light and
// Dark captures land in HIDE_E2E_SCREENSHOT_DIR.

import { expect, test, type Page } from "@playwright/test";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { focusTab, openProject, openWorktree, sendKeys, startedAgentIn, tabOfPane, viewPane, withControl } from "./daemon-actions";
import type { HidedControl } from "./hided-control.mjs";
import { startEngine } from "./engine-fixture";
import { setAgentState } from "./fixture-agents";
import { startHided, type Daemon } from "./hided-fixture";
import { fixtureProgram } from "./platform-fixture";
import { countSent, screenshot } from "./wire";
import { projectRow } from "./overview-entry";
import { initGithubRepository } from "./github-remote";

test.describe.configure({ timeout: 240_000 });

const SIGNATURE = "Signature: 8a477f597d28d172789f06886806bc55\n# Created by the disk cleanup e2e fixture\n";

function git(cwd: string, args: string[]): string {
  return execFileSync("git", ["-c", "user.name=e2e", "-c", "user.email=e2e@example.invalid", "-c", "init.defaultBranch=main", "-c", "commit.gpgsign=false", ...args], { cwd, encoding: "utf8" });
}

/**
 * A logged-in `gh` whose one pull request merged `branch` at `head`, the
 * commit its worktree still holds: the record that makes that worktree's work
 * landed, so the sheet reads it as finished. Git ancestry alone does not.
 */
function fakeGh(dir: string, branch: string, head: string): string {
  const bin = path.join(dir, "gh-bin");
  const pulls = JSON.stringify([
    {
      number: 7,
      title: "shipped work",
      statusCheckRollup: [],
      headRefName: branch,
      headRefOid: head,
      baseRefName: "main",
      state: "MERGED",
      reviewDecision: null,
      isDraft: false,
      url: "https://github.com/acme/repo/pull/7",
      mergedAt: "2026-09-27T00:00:00Z",
      updatedAt: "2026-09-27T00:00:00Z",
      closingIssuesReferences: [],
    },
  ]);
  fixtureProgram(
    bin,
    "gh",
    `const args = process.argv.slice(2);
const key = args.slice(0, 2).join(" ");
const answers = { "pr list": ${JSON.stringify(pulls)}, "repo view": '{"nameWithOwner":"acme/repo"}', "issue list": "[]" };
if (key === "auth status") process.exit(0);
if (key === "api user") { console.log("example-user"); process.exit(0); }
if (process.argv.slice(2).join(" ").includes("review-requested:@me")) { console.log("[]"); process.exit(0); }
if (process.argv.slice(2).join(" ").includes("HideHeadRuns")) { const repository = {}; for (let i = 0; i < process.argv.slice(2).join(" ").split("object(oid:").length - 1; i++) repository["c" + i] = { checkSuites: { totalCount: 0, nodes: [] } }; console.log(JSON.stringify({ data: { repository } })); process.exit(0); }
if (process.argv.slice(2).join(" ").includes("HidePrFacts")) { console.log('{"data":{"repository":{"pullRequests":{"nodes":[]}}}}'); process.exit(0); }
if (args.join(" ").includes("--state merged")) { console.log("[]"); process.exit(0); }
if (key in answers) { console.log(answers[key]); process.exit(0); }
console.error("unsupported: " + args.join(" "));
process.exit(1);
`,
  );
  return bin;
}

/** The agent the stand-in runs in the shell of a first terminal in the linked worktree `dir`, typed at its prompt; no page shows the pane, so it is sized first. */
async function agentInWorktree(control: HidedControl, dir: string): Promise<string> {
  const { pane } = await openWorktree(control, dir);
  viewPane(control, pane);
  sendKeys(control, pane, "claude\r");
  await control.until((rest) => startedAgentIn(rest, pane), `an agent started in ${pane}`, 30_000);
  return pane;
}

/** What a build leaves in a checkout: ignored folders, sized so each layer has bytes. */
function build(dir: string): void {
  const file = (relative: string, kilobytes: number) => {
    const target = path.join(dir, relative);
    fs.mkdirSync(path.dirname(target), { recursive: true });
    fs.writeFileSync(target, Buffer.alloc(kilobytes * 1024, 1));
  };
  fs.mkdirSync(path.join(dir, "target"), { recursive: true });
  fs.writeFileSync(path.join(dir, "target", "CACHEDIR.TAG"), SIGNATURE);
  file("target/debug/blob", 256);
  file("node_modules/pkg/index.js", 128);
  file("dist/bundle.js", 64);
  file("agents/runs/session.log", 64);
}

/** A capture in the other theme: the sheet is modal, so the class the theme setting toggles is toggled directly. */
async function showTheme(page: Page, theme: "light" | "dark"): Promise<void> {
  await page.evaluate((next) => {
    const root = document.documentElement;
    root.classList.toggle("dark", next === "dark");
    root.classList.toggle("light", next === "light");
  }, theme);
  await expect(page.locator("html")).toHaveClass(theme === "dark" ? /(^|\s)dark(\s|$)/ : /(^|\s)light(\s|$)/);
  // Colour transitions the class change started finish before the capture; a skeleton's endless pulse is not waited on.
  await page.evaluate(() =>
    Promise.all(
      document
        .getAnimations()
        .filter((animation) => animation.effect?.getComputedTiming().iterations !== Infinity)
        .map((animation) => animation.finished.catch(() => undefined)),
    ),
  );
}

// @platform: Disk measuring and removal, and Trash, on the platform's filesystem.
test("the disk cleanup sheet: layers, a cache-only cleanup at once, and a worktree after one confirmation", { tag: "@platform" }, async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 1000 });
  const herdr = await startEngine();
  let daemon: Daemon | null = null;
  try {
    const repo = path.join(herdr.root, "repo");
    fs.mkdirSync(repo);
    initGithubRepository(repo);
    fs.writeFileSync(path.join(repo, "README.md"), "# repo\n");
    fs.writeFileSync(path.join(repo, "Cargo.toml"), '[package]\nname = "fixture"\n');
    fs.writeFileSync(path.join(repo, "package.json"), '{"name":"fixture"}\n');
    fs.writeFileSync(path.join(repo, ".gitignore"), "target/\nnode_modules/\ndist/\nagents/runs/\n");
    git(repo, ["add", "."]);
    git(repo, ["commit", "-m", "initial"]);
    // A repository with an origin, as a real one has: the core measures each branch against origin's default.
    const origin = path.join(herdr.root, "origin.git");
    git(herdr.root, ["init", "--bare", origin]);
    git(repo, ["remote", "add", "origin", origin]);
    git(repo, ["push", "-q", "origin", "main"]);
    git(repo, ["remote", "set-head", "origin", "main"]);
    const shipped = path.join(herdr.root, "repo-shipped");
    const active = path.join(herdr.root, "repo-active");
    git(repo, ["worktree", "add", "-b", "prd/shipped", shipped]);
    git(shipped, ["commit", "--allow-empty", "-m", "shipped work"]);
    git(repo, ["merge", "--ff-only", "prd/shipped"]);
    git(repo, ["worktree", "add", "-b", "prd/active", active]);
    git(active, ["commit", "--allow-empty", "-m", "active work"]);
    for (const dir of [repo, shipped, active]) build(dir);

    daemon = await startHided(herdr, "disk-cleanup", undefined, { PATH: `${fakeGh(herdr.root, "prd/shipped", git(shipped, ["rev-parse", "HEAD"]).trim())}${path.delimiter}${herdr.fixturePath}` });
    await withControl(daemon, (control) => openProject(control, repo, "repo"));
    const sent = countSent(page);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await expect(page.locator("[data-main-screen]").or(page.locator("[data-workspace-screen]"))).toBeVisible({ timeout: 20_000 });
    // B1 (PRD overview-device-tasks B51): the Projects tab's size lists the layers on hover and opens the sheet.
    const entrance = (await projectRow(page, "repo")).locator("[data-projects-disk-open]");
    await expect(entrance).toBeEnabled({ timeout: 30_000 });
    await entrance.hover();
    const tooltip = page.getByRole("tooltip");
    for (const layer of ["Build cache", "Dependencies", "Worktree source", "Other", "Shared Git"]) await expect(tooltip).toContainText(layer);
    await page.mouse.move(2, 998);
    const reviewsBefore = sent.get("cleanup_review") ?? 0;
    await entrance.click();
    const sheet = page.locator("[data-disk-sheet]");
    await expect(sheet).toBeVisible();
    // B27: opening the sheet is one review event, and hover, filters and ticks add none.
    await expect.poll(() => (sent.get("cleanup_review") ?? 0) - reviewsBefore).toBe(1);
    await expect(sheet).toHaveAttribute("data-disk-state", "ready", { timeout: 60_000 });

    // B4, D-21: main on top, and every fixture checkout is under 1 GB, so the rest fold.
    const row = (suffix: string) => sheet.locator(`[data-disk-row$="${suffix}"]`);
    await expect(row("/repo")).toBeVisible();
    await expect(sheet.locator("[data-disk-fold]")).toHaveAttribute("data-disk-fold", "closed");
    await sheet.locator("[data-disk-fold-toggle]").click();
    await expect(row("/repo-shipped")).toBeVisible();
    await expect(row("/repo-active")).toBeVisible();
    // D-22: the worktree whose pull request merged is finished, the other rests, and each filter states its count.
    await expect(row("/repo-shipped")).toHaveAttribute("data-disk-bucket", "done");
    await expect(row("/repo-active")).toHaveAttribute("data-disk-bucket", "resting");
    await expect(sheet.locator('[data-disk-filter-item="done"]')).toContainText("1");

    // B7, B8: git-visible files are in no cell, the unknown folder is `other` and has no checkbox.
    const other = row("/repo-active").locator('[data-disk-cell$=":other"]');
    await expect(other).toContainText(/\d/);
    await expect(other.locator('input, button[role="checkbox"]')).toHaveCount(0);
    await expect(row("/repo-active").locator('[data-disk-check$=":build_cache"]')).toBeEnabled();
    await expect(row("/repo").locator('[data-disk-cell$=":worktree"]')).toHaveAttribute("data-disk-cell-state", "none");
    // B15: the worktree that is not merged cannot be ticked and says why.
    await expect(row("/repo-active").locator('[data-disk-cell$=":worktree"]')).toHaveAttribute("data-disk-cell-state", "blocked");
    await expect(row("/repo-shipped").locator('[data-disk-check$=":worktree"]')).toBeEnabled();

    // B10: the top-left checkbox ticks every visible cache; the footer counts them (B16).
    await sheet.locator('[data-disk-bundle="all"]').click();
    await expect(sheet.locator('[data-disk-bundle="all"]')).toHaveAttribute("data-disk-bundle-state", "checked");
    await expect(sheet.locator("[data-disk-summary]")).toHaveText(/^6 cells · [\d.]+ (KB|MB)$/);
    // The sheet is trimmed to what the table says: no subtitle, filter hint, column status lines, legend or footer sentences.
    for (const gone of ["크기는 할당된 블록", "체크박스는 보이는 행에만", "선택됨", "고를 수 있는", "지우지 않음", "크기순", "다음 install", "폴더째 지워진다"]) {
      await expect(sheet).not.toContainText(gone);
    }
    // Each bar segment says its layer and size on hover instead of a legend.
    await sheet.locator('[data-disk-bar-part="build_cache"]').hover();
    await expect(page.getByRole("tooltip")).toContainText(/Build cache [\d.]+ (KB|MB)/);
    await page.mouse.move(0, 0);
    await expect(page.getByRole("tooltip")).toHaveCount(0);
    await showTheme(page, "light");
    await screenshot(page, "disk-cleanup-sheet-light");
    await showTheme(page, "dark");
    await screenshot(page, "disk-cleanup-sheet-dark");

    // B17, B21: cache-only runs at once, without a confirmation, and the tracked files stay.
    const confirmsBefore = sent.get("cleanup_confirm") ?? 0;
    await sheet.locator("[data-disk-clean]").click();
    await expect(sheet.locator("[data-disk-result]")).toBeVisible({ timeout: 60_000 });
    expect((sent.get("cleanup_confirm") ?? 0) - confirmsBefore).toBe(1);
    await expect(page.locator("[data-disk-confirm]")).toHaveCount(0);
    for (const dir of [repo, shipped, active]) {
      expect(fs.existsSync(path.join(dir, "target"))).toBe(false);
      expect(fs.existsSync(path.join(dir, "node_modules"))).toBe(false);
      expect(fs.existsSync(path.join(dir, "dist"))).toBe(false);
      expect(fs.existsSync(path.join(dir, "agents", "runs", "session.log"))).toBe(true);
      expect(fs.existsSync(path.join(dir, "README.md"))).toBe(true);
    }
    // B22: one line of free space before and after, then a line for each cell; no allocated-total line.
    await expect(sheet.locator("[data-disk-free-change]")).toContainText(/^Free [\d.]+ GB→[\d.]+ GB$/);
    await expect(sheet).not.toContainText("Allocated total");
    await expect(sheet.locator('[data-disk-result-line="removed"]').first()).toBeVisible();
    await showTheme(page, "light");
    await screenshot(page, "disk-cleanup-result-light");
    await showTheme(page, "dark");
    await screenshot(page, "disk-cleanup-result-dark");

    // B22, B24: review again measures again and returns to the table.
    await sheet.locator("[data-disk-review-again]").click();
    await expect(sheet).toHaveAttribute("data-disk-state", "ready", { timeout: 60_000 });
    // The fold keeps the way the operator left it (open).
    await expect(sheet.locator("[data-disk-fold]")).toHaveAttribute("data-disk-fold", "open");
    await expect(row("/repo-shipped").locator('[data-disk-cell$=":build_cache"]')).toHaveAttribute("data-disk-cell-state", "empty");

    // B11, B18: a worktree includes its caches, asks once, and Back deletes nothing.
    await sheet.locator('[data-disk-check$="/repo-shipped:worktree"]').click();
    await expect(sheet.locator("[data-disk-summary]")).toContainText("1 worktree");
    await sheet.locator("[data-disk-clean]").click();
    const confirm = page.locator("[data-disk-confirm]");
    await expect(confirm).toContainText("Delete the entire prd/shipped folder");
    await expect(confirm).toContainText("Branches are kept");
    await expect(confirm.locator("[data-disk-confirm-list]")).toContainText("prd/shipped");
    await expect(confirm.locator("[data-disk-confirm-run]")).toHaveText("Clean up 1 worktree and caches");
    // Neither button holds the keyboard when the confirmation opens (design 6).
    expect(await page.evaluate(() => document.activeElement?.hasAttribute("data-disk-confirm-run") || document.activeElement?.hasAttribute("data-disk-confirm-back"))).toBe(false);
    await showTheme(page, "light");
    await screenshot(page, "disk-cleanup-confirm-light");
    await showTheme(page, "dark");
    await screenshot(page, "disk-cleanup-confirm-dark");
    await confirm.locator("[data-disk-confirm-back]").click();
    await expect(confirm).toHaveCount(0);
    expect(fs.existsSync(shipped)).toBe(true);
    await sheet.locator("[data-disk-clean]").click();
    await confirm.locator("[data-disk-confirm-run]").click();
    await expect(sheet.locator("[data-disk-result]")).toBeVisible({ timeout: 60_000 });
    // B21: the folder and its registration go, the branch stays.
    expect(fs.existsSync(shipped)).toBe(false);
    expect(git(repo, ["worktree", "list", "--porcelain"])).not.toContain("repo-shipped");
    expect(git(repo, ["branch", "--list", "prd/shipped"]).trim()).not.toBe("");
    expect(fs.existsSync(active)).toBe(true);
    await sheet.locator("[data-disk-close]").click();
    await expect(sheet).toHaveCount(0);
    // B27: two presses of Clean up were two confirm events, one per cleanup.
    expect((sent.get("cleanup_confirm") ?? 0) - confirmsBefore).toBe(2);
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});

test("a checkout with a working agent cannot be ticked, and an open pane alone does not block main", async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 1000 });
  const herdr = await startEngine();
  let daemon: Daemon | null = null;
  try {
    fs.writeFileSync(path.join(herdr.root, "home", ".zshenv"), `export PATH="${path.join(herdr.root, "bin")}:$PATH"\n`);
    const repo = path.join(herdr.root, "repo");
    fs.mkdirSync(repo);
    git(repo, ["init"]);
    fs.writeFileSync(path.join(repo, "package.json"), '{"name":"fixture"}\n');
    fs.writeFileSync(path.join(repo, ".gitignore"), "node_modules/\ndist/\n");
    git(repo, ["add", "."]);
    git(repo, ["commit", "-m", "initial"]);
    const busy = path.join(herdr.root, "repo-busy");
    git(repo, ["worktree", "add", "-b", "prd/busy", busy]);
    git(busy, ["commit", "--allow-empty", "-m", "busy work"]);
    for (const dir of [repo, busy]) build(dir);

    daemon = await startHided(herdr, "disk-cleanup-busy");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await expect(page.locator("[data-main-screen]").or(page.locator("[data-workspace-screen]"))).toBeVisible({ timeout: 20_000 });
    // The core lists the worktree once a client shows its repository, so the page is open first.
    const pane = await withControl(daemon, async (control) => {
      const main = await openProject(control, repo, "repo");
      const pane = await agentInWorktree(control, busy);
      // The worktree's first terminal took the screen; the checkout in front is not one to clean up, so main is shown again.
      await focusTab(control, main, main.tab);
      return pane;
    });
    await setAgentState(daemon, pane, "working");
    await (await projectRow(page, "repo")).locator("[data-projects-disk-open]").click({ timeout: 30_000 });
    const sheet = page.locator("[data-disk-sheet]");
    await expect(sheet).toHaveAttribute("data-disk-state", "ready", { timeout: 60_000 });
    await sheet.locator("[data-disk-fold-toggle]").click();
    const busyRow = sheet.locator('[data-disk-row$="/repo-busy"]');
    await expect(busyRow).toHaveAttribute("data-disk-bucket", "working");
    await expect(busyRow.locator("[data-disk-in-use]")).toHaveText("Agent is working");
    await expect(busyRow.locator('[data-disk-check$=":build_cache"]')).toBeDisabled();
    await expect(busyRow.locator('[data-disk-cell$=":build_cache"]')).toHaveAttribute("data-disk-cell-state", "blocked");
    // Main has a pane open and is still ticked: an open pane alone blocks nothing (D-14).
    await expect(sheet.locator('[data-disk-row$="/repo"] [data-disk-check$=":build_cache"]')).toBeEnabled();
    await sheet.locator('[data-disk-bundle="all"]').click();
    await expect(sheet.locator("[data-disk-summary]")).toContainText("2 cells");
    await sheet.locator("[data-disk-cancel]").click();
    await expect(sheet).toHaveCount(0);
    expect(fs.existsSync(path.join(busy, "node_modules"))).toBe(true);
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});

// A finished agent's pane does not keep a merged worktree from the sheet: it is chosen, the sheet counts the pane
// that closes with it, and the confirmation closes that pane before the folder goes. An agent waiting on an
// approval still blocks it.
test("a worktree left with only a finished agent's pane is chosen and its pane closes with it, and a waiting agent blocks it", async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 1000 });
  const herdr = await startEngine();
  let daemon: Daemon | null = null;
  try {
    fs.writeFileSync(path.join(herdr.root, "home", ".zshenv"), `export PATH="${path.join(herdr.root, "bin")}:$PATH"\n`);
    const repo = path.join(herdr.root, "repo");
    fs.mkdirSync(repo);
    git(repo, ["init"]);
    fs.writeFileSync(path.join(repo, "package.json"), '{"name":"fixture"}\n');
    fs.writeFileSync(path.join(repo, ".gitignore"), "target/\nnode_modules/\ndist/\nagents/runs/\n");
    git(repo, ["add", "."]);
    git(repo, ["commit", "-m", "initial"]);
    const origin = path.join(herdr.root, "origin.git");
    git(herdr.root, ["init", "--bare", origin]);
    git(repo, ["remote", "add", "origin", origin]);
    git(repo, ["push", "-q", "origin", "main"]);
    git(repo, ["remote", "set-head", "origin", "main"]);
    const shipped = path.join(herdr.root, "repo-shipped");
    git(repo, ["worktree", "add", "-b", "prd/shipped", shipped]);
    git(shipped, ["commit", "--allow-empty", "-m", "shipped work"]);
    git(repo, ["merge", "--ff-only", "prd/shipped"]);
    git(repo, ["push", "-q", "origin", "main"]);
    build(shipped);

    daemon = await startHided(herdr, "disk-cleanup-panes");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await expect(page.locator("[data-main-screen]").or(page.locator("[data-workspace-screen]"))).toBeVisible({ timeout: 20_000 });
    // The core lists the worktree once a client shows its repository, so the page is open first.
    const pane = await withControl(daemon, async (control) => {
      const main = await openProject(control, repo, "repo");
      const pane = await agentInWorktree(control, shipped);
      // The worktree's first terminal took the screen; the checkout in front is not one to clean up, so main is shown again.
      await focusTab(control, main, main.tab);
      return pane;
    });
    await setAgentState(daemon, pane, "blocked");
    const sheet = page.locator("[data-disk-sheet]");
    const open = async () => {
      // The project rests once its turn is over and folds under Quiet projects, so its row is found again each time.
      await (await projectRow(page, "repo")).locator("[data-projects-disk-open]").click({ timeout: 30_000 });
      await expect(sheet).toHaveAttribute("data-disk-state", "ready", { timeout: 60_000 });
      await sheet.locator("[data-disk-fold-toggle]").click();
    };
    const row = () => sheet.locator('[data-disk-row$="/repo-shipped"]');

    // An agent waiting on an approval keeps the worktree.
    await open();
    await expect(row().locator("[data-disk-in-use]")).toHaveText("Agent is waiting for input");
    await expect(row().locator('[data-disk-cell$=":worktree"]')).toHaveAttribute("data-disk-cell-state", "blocked");
    await sheet.locator("[data-disk-cancel]").click();
    await expect(sheet).toHaveCount(0);

    // Once its turn is over, the same worktree is chosen and the sheet counts the pane that closes with it.
    const status = (await setAgentState(daemon, pane, "idle")).status_code;
    // A review reads the agents the core holds, the same rows the sidebar draws: the review opens once the
    // agent's row shows the turn over. A `done` with no label verdict is a stop to read, worded Stopped (status-marks D-03).
    await expect(page.locator(`[data-pane="${pane}"] [data-agent-status]`).first()).toHaveAttribute("data-agent-status", status === "done" ? "stopped" : status, { timeout: 30_000 });
    await open();
    await expect(row().locator("[data-disk-in-use]")).toHaveCount(0);
    await expect(row().locator('[data-disk-cell$=":worktree"]')).toHaveAttribute("data-disk-cell-state", "selectable");
    await expect(row().locator("[data-disk-panes]")).toHaveAttribute("data-disk-panes", "1");
    await row().locator('[data-disk-check$=":worktree"]').click();
    await sheet.locator("[data-disk-clean]").click();
    const confirm = page.locator("[data-disk-confirm]");
    await expect(confirm.locator("[data-disk-confirm-list] [data-disk-panes]")).toHaveAttribute("data-disk-panes", "1");
    await expect(confirm.locator("[data-disk-confirm-run]")).toHaveText("Close 1 pane, clean up 1 worktree and caches");
    await screenshot(page, "disk-cleanup-finished-agent-confirm");
    await confirm.locator("[data-disk-confirm-run]").click();
    await expect(sheet.locator("[data-disk-result]")).toBeVisible({ timeout: 60_000 });

    // The pane closed and the folder and its registration went; the branch stays.
    expect(fs.existsSync(shipped)).toBe(false);
    expect(git(repo, ["worktree", "list", "--porcelain"])).not.toContain("repo-shipped");
    expect(git(repo, ["branch", "--list", "prd/shipped"]).trim()).not.toBe("");
    await withControl(daemon, (control) => control.until((rest) => tabOfPane(rest, pane) === undefined || undefined, `${pane} closed`, 20_000));
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});
