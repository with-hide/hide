// What Hide AI started, told from what the operator started, on a real core
// through an isolated engine, hided and browser (PRD hide-ai-origin-mark
// B5, B12, B13; PRD hide-ai-work-home B1, B3, B20). The fixture is a scratch repository with no remote;
// the ledger is seeded in the daemon's own state folder while it is down, the
// way a Memory promotion's spawn leaves it, and never an operator's.

import { expect, test, type Page } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { openProject, openWorktree, withControl } from "./daemon-actions";
import { startEngine, type EngineServer } from "./engine-fixture";
import { sessionIn, startAgentInProject } from "./fixture-agents";
import { git, ledgerOf } from "./hide-ai-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { openOtherCheckouts } from "./wire";

test.describe.configure({ timeout: 150_000 });

type Stack = { server: EngineServer; daemon: Daemon; repo: string; operatorPane: string; hideAiPane: string; hideAiPath: string };

/** A project whose main checkout has the operator's agent, a worktree with an agent Hide started, and a worktree the operator made whose branch is named like Hide AI's. */
async function start(page: Page): Promise<Stack> {
  const server = await startEngine();
  try {
    fs.mkdirSync(path.join(server.root, "scratch"));
    const repo = fs.realpathSync(path.join(server.root, "scratch"));
    git(repo, "init", "-q", "-b", "main");
    git(repo, "-c", "user.name=fixture", "-c", "user.email=fixture@example.invalid", "commit", "-q", "--allow-empty", "-m", "first");
    const hideAiPath = `${repo}.worktrees/hide-memory-m43-fixture`;
    const lookalikePath = `${repo}.worktrees/hide-memory-m9-by-hand`;
    git(repo, "worktree", "add", "-q", "-b", "hide-memory-m43-fixture", hideAiPath);
    git(repo, "worktree", "add", "-q", "-b", "hide-memory-m9-by-hand", lookalikePath);
    const daemon = await startHided(server, "hide-ai-origin");
    try {
      // The core lists a linked worktree once a client shows its repository.
      await open(page, daemon);
      const project = await withControl(daemon, (control) => openProject(control, repo, "scratch"));
      const operator = await startAgentInProject(server, daemon, project, { task: "사이드바 가독성 개선" });
      const hideAi = await startAgentInProject(server, daemon, await withControl(daemon, (control) => openWorktree(control, hideAiPath)), { task: "m43 규칙을 문서로 옮기는 중" });
      await startAgentInProject(server, daemon, await withControl(daemon, (control) => openWorktree(control, lookalikePath)), { task: "m9 손으로 만든 작업" });
      return { server, daemon, repo, operatorPane: operator.pane, hideAiPane: hideAi.pane, hideAiPath };
    } catch (error) {
      daemon.stop();
      throw error;
    }
  } catch (error) {
    server.stop();
    throw error;
  }
}

async function open(page: Page, daemon: Daemon): Promise<void> {
  await page.goto("about:blank");
  await page.goto(`${daemon.origin}/#token=${daemon.token}`);
}

/** Restarts the daemon with Hide's record of its start in the ledger, as a promotion leaves it. */
async function seed(stack: Stack, task: { kind: string; target: string } | null): Promise<void> {
  const session = await sessionIn(stack.daemon, stack.hideAiPane);
  stack.daemon = await stack.daemon.restart((dir) => {
    const file = path.join(dir, "delivery-ledger.json");
    fs.writeFileSync(file, ledgerOf(stack.daemon.node, [{ pane: stack.hideAiPane, session, path: stack.hideAiPath, branch: "hide-memory-m43-fixture", task }]), { mode: 0o600 });
  });
}

test("an agent Hide AI started for a home leaves the project tree with no head, and a branch named like Hide AI's stays the operator's", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const stack = await start(page);
  try {
    await open(page, stack.daemon);
    const project = page.locator("[data-project]", { hasText: "scratch" });
    await expect(project.locator("[data-checkout-row]").first()).toBeVisible({ timeout: 20_000 });
    // Every checkout but the main one waits in the project's Checkouts fold.
    await openOtherCheckouts(project);
    // Before Hide AI's record exists, every checkout is the operator's and its agent is drawn (B5).
    await expect(project.locator("[data-checkout-row]", { hasText: "hide-memory-m43" })).toHaveCount(1);
    await expect(page.locator(`[data-pane="${stack.hideAiPane}"]`)).toHaveCount(1);

    await seed(stack, { kind: "memory_promote", target: "m43" });
    await open(page, stack.daemon);
    await openOtherCheckouts(project);
    // The head is gone, and so is the agent's row: its home, the Memory tab, shows its work (B1).
    await expect(page.locator(`[data-pane="${stack.hideAiPane}"]`)).toHaveCount(0, { timeout: 30_000 });
    await expect(page.locator("[data-hide-ai-group]")).toHaveCount(0);
    // Nothing of the operator's is in the checkout Hide AI made and it is not in front, so the tree does not draw it (B3).
    await expect(project.locator("[data-checkout-row]", { hasText: "hide-memory-m43" })).toHaveCount(0);
    // The operator's checkouts, the lookalike branch included, and the operator's agent stay.
    await expect(project.locator("[data-checkout-row]", { hasText: "hide-memory-m9-by-hand" })).toHaveCount(1);
    await expect(page.locator(`[data-pane="${stack.operatorPane}"]`)).toHaveCount(1);
    await expect(page.locator(`[data-pane="${stack.operatorPane}"] [data-agent-task]`)).toHaveCount(0);
  } finally {
    stack.daemon.stop();
    stack.server.stop();
  }
});

test("a start whose kind this build has no home for stays an operator's row naming its task, and it all holds across a restart of hided", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const stack = await start(page);
  try {
    // A record with no task, or one a newer build wrote, loads with the rest of the ledger (B20).
    await seed(stack, null);
    await open(page, stack.daemon);
    await openOtherCheckouts(page.locator("[data-project]", { hasText: "scratch" }));
    const row = page.locator(`[data-pane="${stack.hideAiPane}"]`);
    await expect(row.locator("[data-agent-task]")).toHaveText("Hide AI", { timeout: 30_000 });

    const session = await sessionIn(stack.daemon, stack.hideAiPane);
    stack.daemon = await stack.daemon.restart((dir) => {
      const file = path.join(dir, "delivery-ledger.json");
      fs.writeFileSync(file, ledgerOf(stack.daemon.node, [{ pane: stack.hideAiPane, session, path: stack.hideAiPath, branch: "hide-memory-m43-fixture", task: { kind: "future_kind", target: "m43" } }]), { mode: 0o600 });
    });
    await open(page, stack.daemon);
    await expect(row.locator("[data-agent-task]")).toHaveText("future_kind · m43", { timeout: 30_000 });
    await expect(page.locator("[data-hide-ai-group]")).toHaveCount(0);
  } finally {
    stack.daemon.stop();
    stack.server.stop();
  }
});
