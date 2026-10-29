// A Hide AI fix waiting on its branch, on a real core through an isolated
// pinned Herdr, hided and browser (PRD hide-ai-branch-first-apply B3, B4, B6,
// B7, B13): two 제안 lines whose agents left a commit on a local branch of a
// scratch repository with no remote, seeded into the daemon's own suggestion
// store while it is down, the way a stopped agent leaves them. The core reads
// each branch from git, the 보기 slot shows the branch and one file's diff,
// and `Merge into main` merges on the node without pushing. Nothing here is an
// operator's checkout, app or store.

import { expect, test, type Page } from "@playwright/test";
import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { digest, git } from "./hide-ai-fixture";
import { startHerdr } from "./herdr-fixture";
import { startHided } from "./hided-fixture";
import { countSent, showProjectMemory, showProjectRow } from "./wire";

test.describe.configure({ timeout: 240_000 });

const FIX = "hide-suggestion-s1-fixture";
const DIRTY = "hide-suggestion-s2-fixture";

const commit = (repo: string, message: string) => git(repo, "-c", "user.name=fixture", "-c", "user.email=fixture@example.invalid", "commit", "-q", "-am", message);
const head = (repo: string, ref: string) => execFileSync("git", ["rev-parse", ref], { cwd: repo, encoding: "utf8" }).trim();

/** A repository with no remote: main, a fix branch that changes two documents, and one that changes the file the operator is editing. */
function repository(root: string): string {
  const repo = path.join(root, "branchy");
  fs.mkdirSync(path.join(repo, "docs"), { recursive: true });
  git(repo, "init", "-q", "-b", "main");
  fs.writeFileSync(path.join(repo, "AGENTS.md"), "# Agents\n\nRun the tests.\n");
  fs.writeFileSync(path.join(repo, "docs", "guide.md"), "# Guide\n");
  fs.writeFileSync(path.join(repo, "notes.md"), "notes\n");
  git(repo, "add", ".");
  commit(repo, "first");
  git(repo, "checkout", "-q", "-b", FIX);
  fs.writeFileSync(path.join(repo, "AGENTS.md"), "# Agents\n\nRun the linter before the tests.\nKeep each commit small.\n");
  fs.writeFileSync(path.join(repo, "docs", "guide.md"), "# Guide\n\nThe linter runs first.\n");
  commit(repo, "docs: run the linter before the tests");
  git(repo, "checkout", "-q", "main");
  git(repo, "checkout", "-q", "-b", DIRTY);
  fs.writeFileSync(path.join(repo, "notes.md"), "notes\nfrom the agent\n");
  commit(repo, "docs: note the agent's finding");
  git(repo, "checkout", "-q", "main");
  return fs.realpathSync(repo);
}

/** Two lines whose agents stopped on their branches, as the worker keeps them; the core reads each branch again from git. */
function seedLines(database: string, project: string, node: string, repo: string): void {
  const now = Date.now();
  const line = (id: number, discomfort: string) => JSON.stringify({
    id, discomfort, cause: "The docs do not say it", fix: "Write it in AGENTS.md", outcome: "pr", basis: null,
    records: [], struck: null, stage: "applying", level: 0, created_at_unix_ms: now - 600_000, updated_at_unix_ms: now - 60_000,
  });
  const life = (branch: string) => JSON.stringify({
    raised_at_unix_ms: now - 600_000,
    attempt: {
      started_at_unix_ms: now - 300_000, auto: false, phase: "ready", branch, agent_name: `suggestion-apply-${branch}`,
      purpose: "branch", branch_facts: { base: "main", tip: "0".repeat(40), ahead: 0, files: [], files_total: 0, added: 0, removed: 0 },
    },
  });
  const script = `
import json, sqlite3, sys
database, project, node, repo, lines = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4], json.loads(sys.argv[5])
connection = sqlite3.connect(database)
row = connection.execute("SELECT state FROM projects WHERE id=?", (project,)).fetchone()
state = json.loads(row[0]) if row else {}
state["location"] = {"node": node, "path": repo}
if row:
    connection.execute("UPDATE projects SET state=?, next_line=3 WHERE id=?", (json.dumps(state), project))
else:
    connection.execute("INSERT INTO projects VALUES(?,?,1,3)", (project, json.dumps(state)))
for (id, line, life) in lines:
    connection.execute("INSERT INTO lines VALUES(?,?,?,?)", (project, id, line, life))
connection.commit()
connection.close()
`;
  if (!fs.existsSync(database)) throw new Error(`the core made no suggestion store at ${database}; the folder holds ${fs.readdirSync(path.dirname(database)).join(", ")}`);
  const lines = [[1, line(1, "Agents keep running the tests before the linter"), life(FIX)], [2, line(2, "The agent's finding is not in the notes"), life(DIRTY)]];
  const run = spawnSync("python3", ["-I", "-c", script, database, project, node, repo, JSON.stringify(lines)], { encoding: "utf8" });
  if (run.status !== 0) throw new Error(`seeding the suggestion store failed: ${run.stderr}`);
}

async function open(page: Page, origin: string, token: string): Promise<void> {
  await page.goto("about:blank");
  await page.goto(`${origin}/#token=${token}`);
}

test("a fix waits on its local branch, shows its files and diff in the 보기 slot, and merges into main on the press without a remote", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const herdr = await startHerdr({ readiness: "registered" });
  let daemon: Awaited<ReturnType<typeof startHided>> | null = null;
  try {
    const repo = repository(herdr.root);
    herdr.run(["workspace", "create", "--cwd", repo, "--label", "branchy", "--env", `PATH=${herdr.fixturePath}`, "--no-focus"]);
    daemon = await startHided(herdr, "hide-ai-branch");
    const sent = new Map<string, Record<string, unknown>>();
    countSent(page, sent);
    await open(page, daemon.origin, daemon.token);
    const project = page.locator("[data-project]", { hasText: "branchy" });
    // Opening the Hide AI tool makes the Project's suggestion store; the lines are seeded into it while the core is down.
    await (await showProjectRow(page, project.locator("[data-checkout]").first())).click();
    await showProjectMemory(page);
    await expect.poll(() => sent.get("memory_open")?.workspace_id).toBeTruthy();
    const lane = page.locator('[data-lane="memory"]');
    await expect(lane.locator("[data-suggestions]")).toBeVisible({ timeout: 20_000 });
    const node = daemon.node;
    daemon = await daemon.restart((dir) => seedLines(path.join(dir, "suggestions.sqlite3"), `project:${digest(`${node}\0${repo}`)}`, node, repo));
    await open(page, daemon.origin, daemon.token);
    await (await showProjectRow(page, project.locator("[data-checkout]").first())).click();
    await showProjectMemory(page);
    const evidence = process.env.HIDE_E2E_SCREENSHOT_DIR ?? herdr.root;

    // B3: the core read each branch from git, so the row is the operator's turn with the branch, its files and lines.
    const fix = lane.locator('[data-suggestion="s1"]');
    await expect(fix.locator(`[data-suggestion-branch="${FIX}"]`)).toContainText("2 files", { timeout: 60_000 });
    await expect(fix.locator("[data-suggestion-mark]")).toHaveAttribute("data-suggestion-mark", "my_turn");
    await expect(fix.locator(`[data-suggestion-branch="${FIX}"]`)).toContainText("+4−1");
    await expect(fix.locator('[data-suggestion-merge="s1"]')).toHaveText("Merge into main");
    // D-03: no GitHub here, so no pull request is offered.
    await expect(fix.locator('[data-suggestion-open-pr="s1"]')).toHaveCount(0);
    await lane.screenshot({ path: path.join(evidence, "branch-row-ready.png") });

    // B4: the card in the 보기 slot lists each file; a file opens its diff against main, read on the node.
    await fix.locator('[data-suggestion-branch-view="s1"]').click();
    const card = page.locator('[data-lane="detail"] [data-branch-card="s1"]');
    await expect(card.locator("[data-branch-card-file]")).toHaveCount(2, { timeout: 20_000 });
    await expect(card).toContainText(FIX);
    await page.locator('[data-lane="detail"]').screenshot({ path: path.join(evidence, "branch-card.png") });
    await card.locator('[data-branch-card-file="AGENTS.md"]').click();
    const diff = page.locator('[data-lane="detail"] [data-branch-card-diff="AGENTS.md"]');
    await expect(diff.locator("[data-patch-view]")).toContainText("Run the linter before the tests.", { timeout: 20_000 });
    await page.locator('[data-lane="detail"]').screenshot({ path: path.join(evidence, "branch-card-diff.png") });
    // The checkout in front did not move for it.
    expect(head(repo, "HEAD")).toBe(head(repo, "main"));

    // B7: the operator's own change to the file the other branch changes keeps that merge from writing anything.
    fs.writeFileSync(path.join(repo, "notes.md"), "notes\nmine, not saved\n");
    const before = head(repo, "main");
    const dirty = lane.locator('[data-suggestion="s2"]');
    await expect(dirty.locator('[data-suggestion-merge="s2"]')).toBeVisible({ timeout: 30_000 });
    await dirty.locator('[data-suggestion-merge="s2"]').click();
    await expect(dirty.locator('[data-suggestion-failure="merge_dirty"]')).toContainText("main has unsaved changes in the same files", { timeout: 30_000 });
    await expect(dirty.locator('[data-suggestion-merge="s2"]')).toHaveText("Try again");
    expect(head(repo, "main")).toBe(before);
    expect(fs.readFileSync(path.join(repo, "notes.md"), "utf8")).toBe("notes\nmine, not saved\n");
    await dirty.screenshot({ path: path.join(evidence, "branch-row-merge-dirty.png") });

    // B6: the press merges into main on the node, the row folds under 반영됨, the branch stays and nothing is pushed.
    const tip = head(repo, FIX);
    await fix.locator('[data-suggestion-merge="s1"]').click();
    await expect.poll(() => head(repo, "main"), { timeout: 30_000 }).toBe(tip);
    await expect(lane.locator('[data-suggestions-group="applying"] [data-suggestion="s1"]')).toHaveCount(0, { timeout: 30_000 });
    await expect(lane.locator('[data-suggestions-group="applied"]')).toBeVisible();
    expect(fs.readFileSync(path.join(repo, "AGENTS.md"), "utf8")).toContain("Run the linter before the tests.");
    expect(head(repo, FIX)).toBe(tip);
    expect(execFileSync("git", ["remote"], { cwd: repo, encoding: "utf8" }).trim()).toBe("");
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});
