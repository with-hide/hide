// The work Hide AI does for a home, on a real core through an isolated pinned
// Herdr, hided and browser (PRD hide-ai-work-home B8-B11, B15-B17): a Memory
// card's promotion whose pull request a fake `gh` answers from a state file
// the spec moves. The Memory store and the coordination record are seeded in
// the daemon's own state folder while it is down, the way a promotion's start
// leaves them; nothing here is an operator's checkout, app or store.

import { expect, test, type Page } from "@playwright/test";
import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { initGithubRepository } from "./github-remote";
import { git, ledgerOf } from "./hide-ai-fixture";
import { sessionOf, spawnAgent, startHerdr, type HerdrFixture } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { fixtureProgram } from "./platform-fixture";
import { countSent, sendEvent, showProjectMemory, showProjectRow } from "./wire";

test.describe.configure({ timeout: 240_000 });

const BRANCH = "hide-memory-m43-fixture";
const PR = 924;

type Pull = { state: "OPEN" | "MERGED"; checks: "SUCCESS" | "FAILURE" };

/** A `gh` logged in for `acme/repo` that lists one pull request on the attempt's branch, as the state file says, at the branch's tip as it is now. */
function fakeGh(root: string, repo: string): { bin: string; state: string } {
  const bin = path.join(root, "gh-bin");
  const state = path.join(root, "gh-state.json");
  fs.mkdirSync(bin, { recursive: true });
  fs.writeFileSync(state, JSON.stringify({ state: "OPEN", checks: "SUCCESS" } satisfies Pull));
  fixtureProgram(
    bin,
    "gh",
    `const fs = require("fs");
const args = process.argv.slice(2);
const key = args.slice(0, 2).join(" ");
if (key === "auth status") process.exit(0);
if (key === "repo view") { console.log('{"nameWithOwner":"acme/repo"}'); process.exit(0); }
if (key === "issue list") { console.log("[]"); process.exit(0); }
// The signed-in login, the review asks and the merge-queue facts that the pull request read takes with the list (PRD work-tab-pr-lens D-25).
if (key === "api user") { console.log("example-user"); process.exit(0); }
if (key === "api graphql") { console.log('{"data":{"repository":{"pullRequests":{"nodes":[]}}}}'); process.exit(0); }
if (key === "pr list") {
  if (args.join(" ").includes("review-requested:@me")) { console.log("[]"); process.exit(0); }
  if (args.join(" ").includes("--state merged")) { console.log("[]"); process.exit(0); }
  const pull = JSON.parse(fs.readFileSync(${JSON.stringify(state)}, "utf8"));
  console.log(JSON.stringify([{
    number: ${PR}, title: "Add the card to the docs",
    statusCheckRollup: [{ __typename: "CheckRun", status: "COMPLETED", conclusion: pull.checks, name: "verify" }],
    headRefName: ${JSON.stringify(BRANCH)}, headRefOid: require("child_process").execFileSync("git", ["-C", ${JSON.stringify(repo)}, "rev-parse", ${JSON.stringify(BRANCH)}], { encoding: "utf8" }).trim(), baseRefName: "main",
    state: pull.state, reviewDecision: null, isDraft: false,
    url: "https://github.com/acme/repo/pull/${PR}",
    mergedAt: pull.state === "MERGED" ? "2026-10-15T00:00:00Z" : null,
    updatedAt: new Date().toISOString(), closingIssuesReferences: [],
  }]));
  process.exit(0);
}
console.error("unsupported: " + args.join(" "));
process.exit(1);
`,
  );
  return { bin, state };
}

/** Inserts the card and its promotion into the store the core made, as a promotion's start leaves them. */
function seedPromotion(database: string, agentId: string): void {
  const script = `
import json, sqlite3, sys, time
database, agent = sys.argv[1], sys.argv[2]
now = int(time.time() * 1000)
connection = sqlite3.connect(database)
project = connection.execute("SELECT id FROM projects").fetchone()[0]
connection.execute("UPDATE projects SET next_number=44 WHERE id=?", (project,))
connection.execute("INSERT INTO memory_items(id,project_id,lifecycle,current_revision,confidence,salience,created_at_ms,updated_at_ms,number) VALUES('item-43',?,'active',1,0.9,0.5,?,?,43)", (project, now, now))
connection.execute("INSERT INTO memory_revisions(id,item_id,revision,body,kind,lifecycle,created_at_ms,batch_id,title,docs_compared) VALUES('rev-43','item-43',1,'Run the linter before the tests.','rule','active',?,'batch-43','Lint before tests',1)", (now,))
connection.execute("INSERT INTO memory_fts(item_id,project_id,body,normalized_terms) VALUES('item-43',?,'Run the linter before the tests.','run linter before tests')", (project,))
connection.execute("INSERT INTO card_promotions(item_id,state,revision,attempt,agent_id,branch,pr_number,target_path,started_at_ms,updated_at_ms,repository) VALUES('item-43','pr_open',1,1,?,?,${PR},NULL,?,?,'acme/repo')", (agent, "${BRANCH}", now - 60000, now))
connection.commit()
connection.close()
`;
  if (!fs.existsSync(database)) throw new Error(`the core made no Memory store at ${database}; the folder holds ${fs.readdirSync(path.dirname(database)).join(", ")}`);
  const run = spawnSync("python3", ["-I", "-c", script, database, agentId], { encoding: "utf8" });
  if (run.status !== 0) throw new Error(`seeding the Memory store failed: ${run.stderr}`);
}

type Stack = { herdr: HerdrFixture; daemon: Daemon; repo: string; worktree: string; operatorPane: string; hideAiPane: string; gh: { bin: string; state: string } };

/** A GitHub-backed project: the operator's agent in the main checkout, and the promotion's agent in its own worktree on the attempt's branch. */
async function start(): Promise<Stack> {
  const herdr = await startHerdr({ agents: false });
  const repo = path.join(herdr.root, "scratch");
  fs.mkdirSync(repo);
  initGithubRepository(repo);
  git(repo, "-c", "user.name=fixture", "-c", "user.email=fixture@example.invalid", "commit", "-q", "--allow-empty", "-m", "first");
  const worktree = `${fs.realpathSync(repo)}.worktrees/${BRANCH}`;
  git(repo, "worktree", "add", "-q", "-b", BRANCH, worktree);
  const operatorPane = await spawnAgent(herdr, "scratch", null, fs.realpathSync(repo), { task: "사이드바 가독성 개선" });
  const hideAiPane = await spawnAgent(herdr, "memory-promote-m43-1", null, worktree, { task: "m43 규칙을 문서로 옮기는 중" });
  const gh = fakeGh(herdr.root, fs.realpathSync(repo));
  const daemon = await startHided(herdr, "hide-ai-home", undefined, { PATH: `${gh.bin}${path.delimiter}${herdr.fixturePath}` });
  return { herdr, daemon, repo: fs.realpathSync(repo), worktree, operatorPane, hideAiPane, gh };
}

async function open(page: Page, daemon: Daemon): Promise<void> {
  await page.goto("about:blank");
  await page.goto(`${daemon.origin}/#token=${daemon.token}`);
}

const move = (stack: Stack, pull: Pull) => fs.writeFileSync(stack.gh.state, JSON.stringify(pull));

/** Asks the core to read the Project's pull requests again, as General's Check again does, until `done` holds. */
async function untilRead(page: Page, stack: Stack, workspace: string, done: () => Promise<boolean>, timeout = 150_000): Promise<void> {
  try {
    await expect
      .poll(
        async () => {
          await sendEvent(page, stack.daemon, "github_request", { workspace_id: workspace, refresh: true });
          return done();
        },
        { timeout, intervals: [8_000] },
      )
      .toBe(true);
  } catch (error) {
    dump(stack);
    throw error;
  }
}

/** What the core said, for whoever reads a failure. */
function dump(stack: Stack): void {
  const logs = path.join(stack.daemon.stateDir, "Logs");
  for (const file of fs.existsSync(logs) ? fs.readdirSync(logs) : []) {
    for (const line of fs.readFileSync(path.join(logs, file), "utf8").split("\n")) if (/promotion|github|retire/i.test(line)) console.log(`[diag] ${line.slice(0, 400)}`);
  }
}

const evidence = (stack: Stack) => process.env.HIDE_E2E_SCREENSHOT_DIR ?? stack.herdr.root;

test("a promotion's agent is in Needs You only while its pull request waits for the operator, opens its row in 제안, and leaves with its cleanup on merge", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const stack = await start();
  try {
    const sent = new Map<string, Record<string, unknown>>();
    countSent(page, sent);
    await open(page, stack.daemon);
    const project = page.locator("[data-project]", { hasText: "scratch" });
    await expect(project.locator("[data-checkout-row]").first()).toBeVisible({ timeout: 20_000 });
    // Opening the Memory tab makes the Project's store; the card and its promotion are seeded into it while the core is down.
    await (await showProjectRow(page, project.locator("[data-checkout]").first())).click();
    await showProjectMemory(page);
    await page.locator('[data-lane="memory"] [data-hide-ai-tool-tab="memory"]').click();
    await expect(page.locator('[data-lane="memory"] [data-memory-switch="off"]')).toBeEnabled({ timeout: 20_000 });
    await page.locator('[data-lane="memory"] [data-memory-switch="off"]').click();
    await expect(page.locator('[data-lane="memory"] [data-memory="on"]')).toBeVisible({ timeout: 20_000 });
    const workspace = String(sent.get("memory_open")?.workspace_id);
    const session = sessionOf(stack.herdr, stack.hideAiPane);
    stack.daemon = await stack.daemon.restart((dir) => {
      seedPromotion(path.join(dir, "project-memory.sqlite3"), "agent-2");
      fs.writeFileSync(path.join(dir, "delivery-ledger.json"), ledgerOf(stack.daemon.node, [{ pane: stack.hideAiPane, session, path: stack.worktree, branch: BRANCH, task: { kind: "memory_promote", target: "m43" } }]), { mode: 0o600 });
    });
    await open(page, stack.daemon);

    // B8: the pull request is open, ready and passing, so the operator's turn is the review.
    const needsYou = page.locator('[data-raised-list] [data-raised-group="needs_you"]');
    const row = needsYou.locator(`[data-pane="${stack.hideAiPane}"]`);
    await expect(row).toHaveCount(1, { timeout: 60_000 });
    await expect(row).toContainText("Promote Memory · m43");
    await expect(row.locator("[data-agent-home-turn]")).toHaveText(`PR ${"#"}${PR} review`);
    // B1: it is in no tree and no Sessions group of its own, so the one row is the raised one.
    await expect(page.locator(`nav[data-sidebar] [data-pane="${stack.hideAiPane}"]`)).toHaveCount(1);
    await expect(project.locator(`[data-pane="${stack.hideAiPane}"]`)).toHaveCount(0);
    await needsYou.screenshot({ path: path.join(evidence(stack), "home-needs-you-pr-review.png") });

    // B11 (hide-ai-suggestions): choosing it opens the Hide AI tool on its 제안 tab with the promotion's row m43 open, and does not go to its pane.
    await page.locator('[data-lane="memory"] [data-hide-ai-tool-tab="memory"]').click();
    await row.locator("[data-agent-open]").click();
    const lane = page.locator('[data-lane="memory"]');
    await expect(lane.locator('[data-hide-ai-tool-tab="suggestions"]')).toHaveAttribute("aria-selected", "true", { timeout: 20_000 });
    const home = lane.locator('[data-suggestion="m43"]');
    await expect(home).toHaveAttribute("data-suggestion-open", "true", { timeout: 20_000 });
    await expect(home).toHaveAttribute("data-suggestion-source", "memory");
    await expect(home.locator("[data-suggestion-attempt]")).toHaveText(`PR ${"#"}${PR} review`);
    await expect(page.locator(`[data-pane-view="${stack.hideAiPane}"][data-focused="true"]`)).toHaveCount(0);
    await lane.screenshot({ path: path.join(evidence(stack), "home-suggestions-tab-opened.png") });

    // B9: checks fail while the agent rests, so the word becomes CI failed, and it is still one row.
    move(stack, { state: "OPEN", checks: "FAILURE" });
    await untilRead(page, stack, workspace, async () => (await row.locator("[data-agent-home-turn]").textContent({ timeout: 1_000 }).catch(() => null)) === "CI failed");
    await expect(needsYou.locator(`[data-pane="${stack.hideAiPane}"]`)).toHaveCount(1);
    await needsYou.screenshot({ path: path.join(evidence(stack), "home-needs-you-ci-failed.png") });

    // B15, B17: the pull request merges, so the row leaves, the agent's pane closes, the clean worktree is removed and the branch stays.
    move(stack, { state: "MERGED", checks: "SUCCESS" });
    await untilRead(page, stack, workspace, async () => (await page.locator(`nav[data-sidebar] [data-pane="${stack.hideAiPane}"]`).count()) === 0, 180_000);
    await expect.poll(() => fs.existsSync(stack.worktree), { timeout: 60_000 }).toBe(false).catch((error) => {
      dump(stack);
      throw error;
    });
    expect(execFileSync("git", ["rev-parse", "--verify", BRANCH], { cwd: stack.repo, encoding: "utf8" }).trim()).toMatch(/^[0-9a-f]{40}$/);
    const panes = execFileSync(stack.herdr.bin, ["pane", "list"], { env: stack.herdr.env, encoding: "utf8" });
    expect(panes).not.toContain(stack.hideAiPane);
    expect(panes).toContain(stack.operatorPane);
  } finally {
    stack.daemon.stop();
    stack.herdr.stop();
  }
});
