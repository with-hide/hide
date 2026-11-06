// A sidebar row's pull request with no screen asking (issue #391): on an
// isolated engine and hided with a fake `gh` that knows one pull request
// in each of three repositories. The first daemon only chooses a Workspace, so
// the next start opens on it with the right panel on Explorer and the merged
// pull request's Inactive fold already open: no Overview, no
// project Overview and no palette ever opens, and each page's wire carries no
// `github_request` and no `overview_refresh` (the wire it does carry is the
// terminal and the usage hint). Both rows still draw their lifecycle from the
// core's own read. A restart with `gh` down draws the previous run's pull
// requests muted as stale instead of falling back to a branch glyph, and a
// restart with `gh` back replaces them, still with no screen asking. The third
// repository's pull request is merged and names its head commit: it reattaches
// to the checkout on that commit from the saved answer alone. The repositories
// are tiny, so the worktree reader answers within milliseconds and this flow
// proves the end state; that the row does not wait for the reader is owned by
// the unit test in `herdr-core/src/runtime/tests/github_reads.rs`.

import { expect, test } from "@playwright/test";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { withControl, openProject } from "./daemon-actions";
import { startEngine, type EngineServer } from "./engine-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { fixtureProgram } from "./platform-fixture";
import { toPage } from "../../desktop/src/main/wirePath";
import { countSent, enterWorkspace, showProjectRow } from "./wire";
import { initGithubRepository } from "./github-remote";

test.describe.configure({ timeout: 180_000 });

const REPOSITORIES = [
  { name: "alpha", branch: "feature/alpha", number: 41, state: "OPEN" },
  { name: "bravo", branch: "feature/bravo", number: 42, state: "OPEN", draft: true },
  // Merged: GitHub names the head commit, and the row takes the pull request only while its checkout is still on that commit (#408).
  { name: "charlie", branch: "feature/charlie", number: 43, state: "MERGED" },
];

function git(cwd: string, args: string[]): void {
  execFileSync("git", ["-c", "user.name=e2e", "-c", "user.email=e2e@example.invalid", "-c", "init.defaultBranch=main", "-c", "commit.gpgsign=false", ...args], { cwd, stdio: "ignore" });
}

/**
 * A `gh` that is logged in and lists the pull request of the repository it
 * runs in, until `gh-down` exists beside it; then every call fails the way a
 * missing network does.
 */
function fakeGh(dir: string, heads: Map<string, string>): { bin: string; down: string } {
  const bin = path.join(dir, "gh-bin");
  fs.mkdirSync(bin, { recursive: true });
  const down = path.join(bin, "gh-down");
  const answer = (repository: (typeof REPOSITORIES)[number]) =>
    JSON.stringify([
      {
        number: repository.number,
        title: `Work in ${repository.name}`,
        statusCheckRollup: [{ __typename: "CheckRun", status: "COMPLETED", conclusion: "SUCCESS", name: "verify" }],
        headRefName: repository.branch,
        headRefOid: heads.get(repository.name) ?? "",
        baseRefName: "main",
        state: repository.state,
        reviewDecision: null,
        isDraft: "draft" in repository,
        url: `https://example.invalid/${repository.name}/pull/${repository.number}`,
        mergedAt: repository.state === "MERGED" ? "2026-09-27T00:00:00Z" : null,
        updatedAt: "2026-09-27T00:00:00Z",
        closedAt: repository.state === "MERGED" ? "2026-09-27T00:00:00Z" : null,
        closingIssuesReferences: [],
      },
    ]);
  // The reader asks twice at once: every state without checks, and the open ones with only their number and checks.
  const openChecks = (repository: (typeof REPOSITORIES)[number]) =>
    JSON.stringify([{ number: repository.number, statusCheckRollup: [{ __typename: "CheckRun", status: "COMPLETED", conclusion: "SUCCESS", name: "verify" }] }]);
  const byRepository = Object.fromEntries(REPOSITORIES.map((repository) => [repository.name, { open: openChecks(repository), all: answer(repository) }]));
  // The repository is the folder the call runs in, spelled with `/` as the wire does.
  fixtureProgram(
    bin,
    "gh",
    `const fs = require("fs");
const args = process.argv.slice(2);
const key = args.slice(0, 2).join(" ");
const byRepository = ${JSON.stringify(byRepository)};
if (fs.existsSync(${JSON.stringify(down)})) { console.error("dial tcp: network is unreachable"); process.exit(1); }
if (key === "auth status") process.exit(0);
if (key === "api user") { console.log("example-user"); process.exit(0); }
if (process.argv.slice(2).join(" ").includes("review-requested:@me")) { console.log("[]"); process.exit(0); }
if (process.argv.slice(2).join(" ").includes("HideHeadRuns")) { const repository = {}; for (let i = 0; i < process.argv.slice(2).join(" ").split("object(oid:").length - 1; i++) repository["c" + i] = { checkSuites: { totalCount: 0, nodes: [] } }; console.log(JSON.stringify({ data: { repository } })); process.exit(0); }
if (process.argv.slice(2).join(" ").includes("HidePrFacts")) { console.log('{"data":{"repository":{"pullRequests":{"nodes":[]}}}}'); process.exit(0); }
if (key === "pr list") {
  const repository = byRepository[process.cwd().replaceAll("\\\\", "/").split("/").pop()];
  console.log(!repository ? "[]" : args.join(" ").includes("--state open") ? repository.open : repository.all);
  process.exit(0);
}
if (key === "repo view") { console.log('{"nameWithOwner":"acme/repo"}'); process.exit(0); }
if (key === "issue list") { console.log("[]"); process.exit(0); }
console.error("unsupported: " + args.join(" "));
process.exit(1);
`,
  );
  return { bin, down };
}

/** A repository with its `branch` checked out as a linked worktree; returns the worktree's head commit. */
function repositoryWithWorktree(server: EngineServer, name: string, branch: string): string {
  const repo = path.join(server.root, name);
  fs.mkdirSync(repo);
  initGithubRepository(repo);
  fs.writeFileSync(path.join(repo, "README.md"), `# ${name}\n`);
  git(repo, ["add", "README.md"]);
  git(repo, ["commit", "-m", "initial"]);
  const worktree = path.join(server.root, `${name}-wt`);
  git(repo, ["worktree", "add", "-b", branch, worktree]);
  git(worktree, ["commit", "--allow-empty", "-m", "work"]);
  return execFileSync("git", ["rev-parse", "HEAD"], { cwd: worktree, encoding: "utf8" }).trim();
}

test("the sidebar draws every project's pull request with no screen asking, and a restart keeps them as stale", async ({ page }) => {
  await page.setViewportSize({ width: 1400, height: 900 });
  const engine = await startEngine();
  let daemon = null as Daemon | null;
  try {
    const heads = new Map<string, string>();
    for (const { name, branch } of REPOSITORIES) heads.set(name, repositoryWithWorktree(engine, name, branch));
    const gh = fakeGh(engine.root, heads);
    daemon = await startHided(engine, "sidebar-pr-start", undefined, { PATH: `${gh.bin}${path.delimiter}${engine.fixturePath}` });
    // Each repository is a project, as Add project makes it; its linked worktree is listed under it.
    await withControl(daemon, async (control) => {
      for (const { name } of REPOSITORIES) await openProject(control, path.join(engine.root, name), name);
    });

    // The first run opens on All projects and chooses a Workspace, so the next start resumes on it.
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "alpha");
    const statePath = path.join(daemon.stateDir, "core-state.json");
    await expect.poll(() => (JSON.parse(fs.readFileSync(statePath, "utf8")) as { focused_checkout_id?: string | null }).focused_checkout_id ?? null).not.toBeNull();

    const row = (branch: string) => page.locator(`[data-checkout][aria-label^="${branch}"]`);
    const glyph = (number: number) => page.locator(`[data-checkout-pr-glyph="${number}"]`);
    /** The pull request's mark inside a checkout's glyph: its state, and `data-stale` once GitHub could not be read again. */
    const mark = (number: number) => glyph(number).locator("[data-pr-mark]");
    /** A fresh page on the daemon: on the Workspace, the sidebar's Projects tab, and nothing else opened. */
    const attach = async (next: Daemon) => {
      const sent = countSent(page);
      await page.goto("about:blank");
      await page.goto(`${next.origin}/#token=${next.token}`);
      await expect(page.locator("[data-workspace-screen]")).toBeVisible({ timeout: 20_000 });
      return sent;
    };
    /**
     * A merged pull request's checkout is settled, so its row sits in the project's Inactive fold. The fold is
     * seeded open in the state the first start reads, not clicked open: a click is a toggle sent from a page that
     * has read `aria-expanded` and waits for the core to answer, which says nothing about the pull request glyph
     * this flow proves. That the click sends the toggle is owned by `web/src/inactiveFold.test.tsx`, and what the
     * core does with it by the core test in `herdr-core/src/runtime/tests/projects.rs`.
     */
    const mergedGlyph = async () => {
      await expect(page.locator('[data-inactive-checkouts$="/charlie"]')).toHaveAttribute("aria-expanded", "true", { timeout: 30_000 });
      return glyph(43);
    };
    const nothingAsked = (sent: Map<string, number>) => {
      expect(sent.get("github_request") ?? 0).toBe(0);
      expect(sent.get("overview_refresh") ?? 0).toBe(0);
    };

    // Start 1: GitHub answers. Both rows draw their pull request's mark, fresh.
    daemon = await daemon.restart((stateDir) => {
      const file = path.join(stateDir, "core-state.json");
      const state = JSON.parse(fs.readFileSync(file, "utf8")) as Record<string, unknown>;
      fs.writeFileSync(file, JSON.stringify({ ...state, right_panel_visible: true, right_panel_section: "explorer", expanded_inactive_checkout_project_paths: [toPage(path.join(engine.root, "charlie"))] }));
    });
    let sent = await attach(daemon);
    // B7 defaults each agentless fold closed. Wait for both catalog answers
    // before opening them; an immediate .all() can miss a later project.
    for (const name of ["alpha", "bravo"]) {
      const project = page.locator("[data-project]").filter({ has: page.locator("[data-project-row]", { hasText: new RegExp(`^${name}$`) }) });
      // A project with no agent waits under No agents (PRD sidebar-titlebar-refresh B1).
      await showProjectRow(page, project);
      const fold = project.locator("[data-empty-worktrees]");
      await expect(fold).toHaveAttribute("aria-expanded", "false");
      await fold.click();
      await expect(fold).toHaveAttribute("aria-expanded", "true");
    }
    await expect(row("feature/alpha")).toHaveAttribute("data-checkout-kind", "pull_request", { timeout: 30_000 });
    await expect(mark(41)).toHaveAttribute("data-pr-mark", "mergeable");
    await expect(mark(42)).toHaveAttribute("data-pr-mark", "draft");
    await expect(mark(41)).not.toHaveAttribute("data-stale");
    await expect(await mergedGlyph()).toHaveAttribute("aria-label", "Open pull request #43");
    await expect(mark(43)).toHaveAttribute("data-pr-mark", "merged");
    await expect(mark(43)).not.toHaveAttribute("data-stale");
    nothingAsked(sent);

    // Start 2: GitHub is unreachable. The previous run's answer is drawn and kept, dimmed.
    fs.writeFileSync(gh.down, "");
    daemon = await daemon.restart();
    sent = await attach(daemon);
    await expect(mark(41)).toHaveAttribute("data-pr-mark", "mergeable", { timeout: 30_000 });
    await expect(mark(42)).toHaveAttribute("data-pr-mark", "draft");
    await expect(mark(41)).toHaveAttribute("data-stale", "true");
    await expect(mark(42)).toHaveAttribute("data-stale", "true");
    // The merged one is drawn from the saved answer too: the checkout's head comes from Git's files, not from the slow worktree catalog.
    await mergedGlyph();
    await expect(mark(43)).toHaveAttribute("data-stale", "true");
    nothingAsked(sent);

    // Start 3: GitHub is back. The core's own read replaces the stale answer.
    fs.rmSync(gh.down);
    daemon = await daemon.restart();
    sent = await attach(daemon);
    await expect(mark(41)).not.toHaveAttribute("data-stale", "true", { timeout: 30_000 });
    await expect(mark(42)).not.toHaveAttribute("data-stale");
    await mergedGlyph();
    await expect(mark(43)).not.toHaveAttribute("data-stale");
    nothingAsked(sent);
  } finally {
    daemon?.stop();
    engine.stop();
  }
});
