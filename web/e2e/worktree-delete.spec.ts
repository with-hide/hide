// Delete worktree… on a worktree that would lose work: the menu item is
// enabled, the dialog names the dirt, the unmerged commit and the agent it
// stops, Delete waits for the discard checkbox, and with both boxes ticked
// the row is dimmed with a spinner while it goes, then the folder and its
// unmerged branch go and the row leaves the sidebar.

import { expect, test, type Page } from "@playwright/test";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { startFixture, type AgentFixture } from "./engine-fixture";
import { openProject, openWorktree, tabOfPane, viewPane, withControl } from "./daemon-actions";
import { labelAgentPane, startAgentIn } from "./fixture-agents";
import { startHided, type Daemon } from "./hided-fixture";
import { countSent, enterWorkspace, openOtherCheckouts, screenshot } from "./wire";

test.describe.configure({ timeout: 180_000 });

const BRANCH = "feature/delete";

/** The repository and its linked worktree on disk, before any daemon runs. */
function deletionFixture(fixture: AgentFixture) {
  const repo = path.join(fixture.root, "preflight-repo");
  fs.mkdirSync(repo);
  git(repo, ["init", "-q"]);
  fs.writeFileSync(path.join(repo, ".gitignore"), "target/\nnode_modules/\n");
  git(repo, ["add", ".gitignore"]);
  git(repo, ["commit", "-qm", "initial"]);
  git(repo, ["update-ref", "refs/remotes/origin/main", "HEAD"]);
  git(repo, ["symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/main"]);
  const worktree = path.join(fixture.root, "preflight-linked");
  git(repo, ["worktree", "add", "-qb", BRANCH, worktree]);
  return { repo, worktree };
}

/**
 * The repository as a project and a terminal in its linked worktree, made
 * through the daemon while the page shows it: the core reads a repository's
 * worktrees once a page asks, and `openWorktree` waits for that read.
 * Returns the terminal's pane.
 */
async function openDeletionProject(daemon: Daemon, repo: string, worktree: string): Promise<string> {
  return withControl(daemon, async (control) => {
    await openProject(control, repo, "preflight-repo");
    return (await openWorktree(control, worktree)).pane;
  });
}

/**
 * The linked worktree is not the project's main checkout, so its row waits in
 * the project's Checkouts fold, whether it is in front, holds local changes or
 * neither. Open the fold, then wait for the Git facts that the deletion
 * dialog needs, without prescribing focus.
 */
async function featureRow(page: Page) {
  const project = page.locator("[data-project]").filter({ has: page.locator("[data-project-row]").filter({ hasText: "preflight-repo" }) });
  await openOtherCheckouts(project);
  const feature = project.locator("[data-checkout-row]").filter({ has: page.locator(`[data-checkout][aria-label^="${BRANCH}"]`) });
  await expect(feature.locator("[data-checkout-age]")).toBeVisible({ timeout: 30_000 });
  return feature;
}

async function openDeletion(page: Page, daemon: Daemon, repo: string, worktree: string) {
  await page.goto(`${daemon.origin}/#token=${daemon.token}`);
  await enterWorkspace(page, "fixture");
  const pane = await openDeletionProject(daemon, repo, worktree);
  const feature = await featureRow(page);
  await feature.locator("[data-checkout-menu]").click({ button: "right" });
  await page.getByRole("menu", { name: `${BRANCH} actions` }).locator('[data-menu-item="delete_worktree"]').click();
  return { dialog: page.locator("[data-delete-worktree]"), pane };
}

test("a lock acquired after the dialog read refuses before closing the pane and names the reason and unlock action", { tag: "@platform" }, async ({ page }) => {
  const fixture = await startFixture({ agents: false });
  let daemon: Daemon | null = null;
  try {
    const linked = deletionFixture(fixture);
    const reason = "검토 끝날 때까지 보관 $(literal)";
    // Preserve the ordinary browser confirmation, then change Git at the
    // transport boundary before the request reaches the real core.
    await page.routeWebSocket(/\/ws$/, (socket) => {
      const server = socket.connectToServer();
      socket.onMessage((message) => {
        if (typeof message === "string" && message.includes('"kind":"remove_worktree"')) git(linked.repo, ["worktree", "lock", "--reason", reason, linked.worktree]);
        server.send(message);
      });
      server.onMessage((message) => socket.send(message));
    });
    daemon = await startHided(fixture, "worktree-locked-preflight");
    const { dialog, pane } = await openDeletion(page, daemon, linked.repo, linked.worktree);
    const confirm = dialog.locator("[data-delete-confirm]");
    await expect(confirm).toBeEnabled({ timeout: 30_000 });
    await confirm.click();
    await expect(dialog.locator('[data-delete-result="failed"]')).toContainText(reason);
    await expect(dialog).toContainText(`Worktree ${BRANCH} is locked`);
    await expect(dialog).toContainText("git worktree unlock");
    await expect(dialog).toContainText("No panes were closed");
    await withControl(daemon, (control) => control.until((rest) => tabOfPane(rest, pane), `${pane} still in a tab`));
    expect(fs.existsSync(linked.worktree)).toBe(true);
    const evidence = process.env.HIDE_E2E_SCREENSHOT_DIR;
    if (evidence) await dialog.screenshot({ path: path.join(evidence, "worktree-locked-preflight.png") });
  } finally { daemon?.stop(); fixture.stop(); }
});

test("Discard names each ignored nested repository and deletes them only after explicit confirmation", { tag: "@platform" }, async ({ page }) => {
  const fixture = await startFixture({ agents: false });
  let daemon: Daemon | null = null;
  try {
    const linked = deletionFixture(fixture);
    const names = ["node_modules/vendor/beta", "target/deep/alpha"];
    for (const name of names) {
      const nested = path.join(linked.worktree, name);
      fs.mkdirSync(nested, { recursive: true });
      git(nested, ["init", "-q"]);
    }
    daemon = await startHided(fixture, "worktree-ignored-preflight");
    const { dialog } = await openDeletion(page, daemon, linked.repo, linked.worktree);
    const discard = dialog.locator("label").filter({ has: page.locator("[data-delete-discard]") });
    for (const name of names) await expect(discard).toContainText(name, { timeout: 30_000 });
    const confirm = dialog.locator("[data-delete-confirm]");
    await expect(confirm).toBeDisabled();
    expect(fs.existsSync(linked.worktree)).toBe(true);
    const evidence = process.env.HIDE_E2E_SCREENSHOT_DIR;
    if (evidence) await dialog.screenshot({ path: path.join(evidence, "worktree-ignored-repositories.png") });
    await dialog.locator("[data-delete-discard]").click();
    await confirm.click();
    await expect(dialog.locator('[data-delete-result="finished"]')).toBeVisible({ timeout: 60_000 });
    expect(fs.existsSync(linked.worktree)).toBe(false);
  } finally { daemon?.stop(); fixture.stop(); }
});

test("an open deletion dialog receives refreshed Git facts and retires Discard through an A-to-B-to-A return", { tag: "@platform" }, async ({ page }) => {
  const fixture = await startFixture({ agents: false });
  let daemon: Daemon | null = null;
  try {
    const linked = deletionFixture(fixture);
    const names = ["node_modules/vendor/beta", "target/deep/alpha"];
    for (const name of names) {
      const nested = path.join(linked.worktree, name);
      fs.mkdirSync(nested, { recursive: true });
      git(nested, ["init", "-q"]);
    }
    const payloads = new Map<string, Record<string, unknown>>();
    const sent = countSent(page, payloads);
    await page.addInitScript(() => {
      const observed = window as Window & {
        __preflightRendererSocket?: WebSocket;
        __preflightRestoreSend?: () => void;
      };
      const nativeSend = WebSocket.prototype.send;
      WebSocket.prototype.send = function (data) {
        if (typeof data === "string") {
          try {
            if (JSON.parse(data).client_kind === "web" && new URL(this.url).pathname === "/ws") observed.__preflightRendererSocket = this;
          } catch { /* terminal bytes are not a handshake */ }
        }
        return nativeSend.call(this, data);
      };
      observed.__preflightRestoreSend = () => { WebSocket.prototype.send = nativeSend; };
    });
    const loading: boolean[] = [];
    let rendererConnections = 0;
    let owningConnections = 0;
    let rendererClosed = false;
    page.on("websocket", (socket) => {
      if (new URL(socket.url()).pathname === "/ws") owningConnections += 1;
      let renderer = false;
      socket.on("close", () => { if (renderer) rendererClosed = true; });
      socket.on("framesent", (frame) => {
        try {
          const value = JSON.parse(String(frame.payload)) as { client_kind?: string };
          if (value.client_kind === "web") { renderer = true; rendererConnections += 1; }
        } catch { /* terminal bytes are not a handshake */ }
      });
      socket.on("framereceived", (frame) => {
        if (!renderer) return;
        try {
          const value = JSON.parse(String(frame.payload)) as { payload?: { rest?: { git_worktrees_loading?: boolean } } };
          const next = value.payload?.rest?.git_worktrees_loading;
          if (typeof next === "boolean" && loading.at(-1) !== next) loading.push(next);
        } catch { /* terminal bytes are not a snapshot */ }
      });
    });
    daemon = await startHided(fixture, "worktree-refreshed-consent");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    const pane = await openDeletionProject(daemon, linked.repo, linked.worktree);
    const project = page.locator("[data-project]").filter({ has: page.locator("[data-project-row]").filter({ hasText: "preflight-repo" }) });
    const feature = await featureRow(page);
    await feature.locator("[data-checkout-menu]").click({ button: "right" });
    await page.getByRole("menu", { name: `${BRANCH} actions` }).locator('[data-menu-item="delete_worktree"]').click();
    const dialog = page.locator("[data-delete-worktree]");
    const workspaceIdLocator = project;
    await expect(workspaceIdLocator).toHaveAttribute("data-project", /./);
    const workspaceId = await workspaceIdLocator.getAttribute("data-project");
    const refresh = () => page.evaluate((workspace_id) => {
      const socket = (window as Window & { __preflightRendererSocket?: WebSocket }).__preflightRendererSocket;
      if (!socket || socket.readyState !== WebSocket.OPEN) throw new Error("Existing renderer socket is unavailable");
      socket.send(JSON.stringify({ schema_version: 2, kind: "overview_refresh", payload: { workspace_id } }));
    }, workspaceId);
    const discard = dialog.locator("[data-delete-discard]");
    const confirm = dialog.locator("[data-delete-confirm]");
    for (const name of names) await expect(dialog).toContainText(name);
    await discard.click();
    await expect(confirm).toBeEnabled();
    const beforeEvents = new Map(sent);

    const reason = "검토 끝날 때까지 보관 $(literal)";
    git(linked.repo, ["worktree", "lock", "--reason", reason, linked.worktree]);
    await refresh();
    await expect(dialog.locator("[data-delete-blocked]")).toContainText(reason, { timeout: 15_000 });
    await expect(confirm).toHaveCount(0);
    git(linked.repo, ["worktree", "unlock", linked.worktree]);
    await refresh();
    await expect(dialog.locator("[data-delete-blocked]")).toHaveCount(0, { timeout: 15_000 });
    await expect(discard).not.toBeChecked();
    await expect(confirm).toBeDisabled();
    await expect.poll(() => loading.slice(-2), { timeout: 15_000 }).toEqual([true, false]);
    await discard.click();

    const late = path.join(linked.worktree, "target/late");
    fs.mkdirSync(late);
    git(late, ["init", "-q"]);
    await refresh();
    // Keep the original page, socket and dialog. A reconnect would conceal
    // a missing publication because its first snapshot is already current.
    await expect(dialog).toContainText("target/late", { timeout: 15_000 });
    await expect(discard).not.toBeChecked();
    await expect(confirm).toBeDisabled();
    // A refresh with unchanged facts still completes its loading transition.
    const beforeRefresh = loading.length;
    await refresh();
    await expect.poll(() => loading.slice(beforeRefresh), { timeout: 15_000 }).toEqual([true, false]);
    expect(rendererConnections).toBe(1);
    fs.rmSync(late, { recursive: true });
    await refresh();
    await expect(dialog).not.toContainText("target/late", { timeout: 15_000 });
    await expect(discard).not.toBeChecked();
    await expect(confirm).toBeDisabled();
    expect(rendererConnections).toBe(1);
    expect(owningConnections).toBe(1);
    expect(rendererClosed).toBe(false);
    for (const [kind, count] of sent) {
      if (kind !== "overview_refresh") expect(count, `Unexpected client event ${kind} during refresh`).toBe(beforeEvents.get(kind) ?? 0);
    }
    await withControl(daemon, (control) => control.until((rest) => tabOfPane(rest, pane), `${pane} still in a tab`));
    expect(fs.existsSync(linked.worktree)).toBe(true);

    await discard.click();
    await confirm.click();
    await expect(dialog.locator('[data-delete-result="finished"]')).toBeVisible({ timeout: 60_000 });
    expect(payloads.get("remove_worktree")?.discard_changes).toBe(true);
    expect(payloads.get("remove_worktree")?.expected_ignored_repositories).toEqual(names);
    expect(rendererConnections).toBe(1);
    expect(owningConnections).toBe(1);
    expect(rendererClosed).toBe(false);
    expect(fs.existsSync(linked.worktree)).toBe(false);
  } finally {
    if (!page.isClosed()) await page.evaluate(() => {
      (window as Window & { __preflightRestoreSend?: () => void }).__preflightRestoreSend?.();
    }).catch(() => { /* the renderer may already have closed */ });
    daemon?.stop();
    fixture.stop();
  }
});

function git(cwd: string, args: string[]): string {
  return execFileSync("git", ["-c", "commit.gpgsign=false", "-c", "user.name=e2e", "-c", "user.email=e2e@example.invalid", "-c", "init.defaultBranch=main", ...args], { cwd, encoding: "utf8" });
}

// @platform: Deleting a worktree stops an agent's processes and removes its folder.
test("a dirty worktree with an unmerged branch and an agent is deleted once both boxes are ticked", { tag: "@platform" }, async ({ page }) => {
  await page.setViewportSize({ width: 1400, height: 900 });
  const fixture = await startFixture({ agents: false });
  let daemon: Daemon | null = null;
  try {
    const repo = path.join(fixture.root, "repo");
    fs.mkdirSync(repo);
    git(repo, ["init"]);
    fs.writeFileSync(path.join(repo, "README.md"), "# repo\n");
    git(repo, ["add", "README.md"]);
    git(repo, ["commit", "-m", "initial"]);
    // The base is read from origin/HEAD, as a clone has it.
    git(repo, ["update-ref", "refs/remotes/origin/main", "HEAD"]);
    git(repo, ["symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/main"]);
    const worktree = path.join(fixture.root, "repo-delete");
    git(repo, ["worktree", "add", "-b", BRANCH, worktree]);
    git(worktree, ["commit", "--allow-empty", "-m", "not merged anywhere"]);
    fs.writeFileSync(path.join(worktree, "draft.txt"), "work in progress\n");

    daemon = await startHided(fixture, "worktree-delete");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    // The worktree's terminal runs the agent, typed at its prompt as the operator starts one.
    const pane = await withControl(daemon, async (control) => {
      await openProject(control, repo, "repo");
      const opened = await openWorktree(control, worktree);
      viewPane(control, opened.pane);
      return opened.pane;
    });
    await startAgentIn(fixture, daemon, pane);
    await labelAgentPane(fixture, daemon, pane, { task: "삭제 대상 작업 정리" });

    // The worktree is not the project's main checkout, so its row waits in the project's Checkouts fold.
    await openOtherCheckouts(page.locator("[data-project]").filter({ has: page.locator("[data-project-row]", { hasText: /^repo/ }) }));
    const feature = page.locator("[data-checkout-row]").filter({ has: page.locator(`[data-checkout][aria-label^="${BRANCH}"]`) });
    await expect(feature).toBeVisible({ timeout: 30_000 });
    await feature.locator("[data-checkout-menu]").click({ button: "right" });
    const menu = page.getByRole("menu", { name: `${BRANCH} actions` });
    const item = menu.locator('[data-menu-item="delete_worktree"]');
    await expect(item).not.toHaveAttribute("data-disabled", "");
    await item.click();

    const dialog = page.locator("[data-delete-worktree]");
    await expect(dialog).toBeVisible();
    // The row may arrive after the dialog opened; the consequences follow it.
    const consequences = dialog.locator("[data-delete-consequences]");
    await expect(consequences).toContainText("1 changed file not committed", { timeout: 30_000 });
    await expect(consequences).toContainText("Unmerged, 1 ahead");
    await expect(consequences).toContainText(/stops 1 agent: 삭제 대상 작업 정리 \(/);
    await expect(dialog.locator("[data-delete-branch-warning]")).toHaveText("1 commit not on main is lost with it");
    await expect(dialog).toContainText("Discard 1 changed file");

    const confirm = dialog.locator("[data-delete-confirm]");
    await expect(confirm).toHaveText("Close 1 pane and delete");
    await expect(confirm).toBeDisabled();
    await screenshot(page, "worktree-delete-dialog-unticked");
    await dialog.locator("[data-delete-branch]").click();
    await dialog.locator("[data-delete-discard]").click();
    await expect(confirm).toBeEnabled();
    await screenshot(page, "worktree-delete-dialog-ticked");
    // Every state the row passes through, however briefly it is drawn.
    await page.evaluate((branch) => {
      const seen: string[] = [];
      (window as unknown as { __removingSeen: string[] }).__removingSeen = seen;
      new MutationObserver(() => {
        const row = [...document.querySelectorAll("[data-checkout-row]")].find((li) => li.querySelector(`[data-checkout][aria-label^="${branch}"]`));
        if (row?.getAttribute("data-checkout-removing") === "true" && row.querySelector("[data-checkout-removing-mark]") && row.querySelector("[data-checkout]:disabled")) seen.push("removing");
      }).observe(document.body, { subtree: true, childList: true, attributes: true });
    }, BRANCH);
    await confirm.click();

    // A failure names the phase the dialog stopped in and what it said, so a
    // refused removal and one that never answered read differently.
    const result = dialog.locator("[data-delete-result]");
    await expect(result).toBeVisible({ timeout: 60_000 }).catch(async (error: Error) => {
      throw new Error(`no result: phase ${await dialog.locator("[data-delete-phase]").getAttribute("data-delete-phase", { timeout: 1_000 }).catch(() => "none")}: ${(await dialog.innerText()).replace(/\s+/g, " ")}\n${error.message}`);
    });
    await expect(result, await result.innerText()).toHaveAttribute("data-delete-result", "finished", { timeout: 1_000 });
    await expect(feature).toHaveCount(0, { timeout: 30_000 });
    expect(await page.evaluate(() => (window as unknown as { __removingSeen: string[] }).__removingSeen.length)).toBeGreaterThan(0);
    expect(fs.existsSync(worktree)).toBe(false);
    expect(git(repo, ["branch", "--list", BRANCH]).trim()).toBe("");
  } finally {
    daemon?.stop();
    fixture.stop();
  }
});
