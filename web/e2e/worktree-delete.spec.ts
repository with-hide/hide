// Delete worktree… on a worktree that would lose work: the menu item is
// enabled, the dialog names the dirt, the unmerged commit and the agent it
// stops, Delete waits for the discard checkbox, and with both boxes ticked
// the row is dimmed with a spinner while it goes, then the folder and its
// unmerged branch go and the row leaves the sidebar.

import { expect, test, type Page } from "@playwright/test";
import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { labelAgent, startHerdr, type HerdrFixture } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { countSent, screenshot, showProjectRow } from "./wire";

test.describe.configure({ timeout: 180_000 });

const BRANCH = "feature/delete";

async function deletionFixture(herdr: HerdrFixture) {
  const repo = path.join(herdr.root, "preflight-repo");
  fs.mkdirSync(repo);
  git(repo, ["init", "-q"]);
  fs.writeFileSync(path.join(repo, ".gitignore"), "target/\nnode_modules/\n");
  git(repo, ["add", ".gitignore"]);
  git(repo, ["commit", "-qm", "initial"]);
  git(repo, ["update-ref", "refs/remotes/origin/main", "HEAD"]);
  git(repo, ["symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/main"]);
  const worktree = path.join(herdr.root, "preflight-linked");
  git(repo, ["worktree", "add", "-qb", BRANCH, worktree]);
  const created = herdr.run(["workspace", "create", "--cwd", worktree, "--label", "preflight-linked", "--env", `PATH=${herdr.fixturePath}`, "--no-focus"]) as { result: { root_pane: { pane_id: string } } };
  return { repo, worktree, pane: created.result.root_pane.pane_id };
}

/**
 * B7 leaves the front checkout and local changes outside the empty-worktree
 * fold. Reach the branch either there or through its fold, then wait for
 * the Git facts that the deletion dialog needs, without prescribing focus.
 */
async function featureRow(page: Page) {
  const project = page.locator("[data-project]").filter({ has: page.locator("[data-project-row]").filter({ hasText: "preflight-repo" }) });
  const emptyFold = project.locator("[data-empty-worktrees]");
  // A project with no agent waits under No agents (PRD sidebar-titlebar-refresh B1).
  await showProjectRow(page, project);
  const feature = project.locator("[data-checkout-row]").filter({ has: page.locator(`[data-checkout][aria-label^="${BRANCH}"]`) });
  await expect.poll(async () => await feature.locator("[data-checkout-age]").isVisible() || await emptyFold.isVisible(), { timeout: 30_000 }).toBe(true);
  if (!await feature.isVisible()) {
    await expect(emptyFold).toHaveAttribute("aria-expanded", "false");
    await emptyFold.click();
  }
  await expect(feature.locator("[data-checkout-age]")).toBeVisible({ timeout: 30_000 });
  await expect(project.locator("[data-inactive-checkouts]")).toHaveCount(0);
  return feature;
}

async function openDeletion(page: Page, daemon: Daemon) {
  await page.goto(`${daemon.origin}/#token=${daemon.token}`);
  await expect(page.locator("[data-main-screen]")).toBeVisible({ timeout: 20_000 });
  const feature = await featureRow(page);
  await feature.locator("[data-checkout-menu]").click({ button: "right" });
  await page.getByRole("menu", { name: `${BRANCH} actions` }).locator('[data-menu-item="delete_worktree"]').click();
  return page.locator("[data-delete-worktree]");
}

test("a lock acquired after the dialog read refuses before closing the pane and names the reason and unlock action", { tag: "@platform" }, async ({ page }) => {
  const herdr = await startHerdr();
  let daemon: Daemon | null = null;
  try {
    const fixture = await deletionFixture(herdr);
    const reason = "검토 끝날 때까지 보관 $(literal)";
    // Preserve the ordinary browser confirmation, then change Git at the
    // transport boundary before the request reaches the real core.
    await page.routeWebSocket(/\/ws$/, (socket) => {
      const server = socket.connectToServer();
      socket.onMessage((message) => {
        if (typeof message === "string" && message.includes('"kind":"remove_worktree"')) git(fixture.repo, ["worktree", "lock", "--reason", reason, fixture.worktree]);
        server.send(message);
      });
      server.onMessage((message) => socket.send(message));
    });
    daemon = await startHided(herdr, "worktree-locked-preflight");
    const dialog = await openDeletion(page, daemon);
    const confirm = dialog.locator("[data-delete-confirm]");
    await expect(confirm).toBeEnabled({ timeout: 30_000 });
    await confirm.click();
    await expect(dialog.locator('[data-delete-result="failed"]')).toContainText(reason);
    await expect(dialog).toContainText(`Worktree ${BRANCH} is locked`);
    await expect(dialog).toContainText("git worktree unlock");
    await expect(dialog).toContainText("No panes were closed");
    const pane = herdr.run(["pane", "get", fixture.pane]) as { result: { pane: { pane_id: string } } };
    expect(pane.result.pane.pane_id).toBe(fixture.pane);
    expect(fs.existsSync(fixture.worktree)).toBe(true);
    const evidence = process.env.HIDE_E2E_SCREENSHOT_DIR;
    if (evidence) await dialog.screenshot({ path: path.join(evidence, "worktree-locked-preflight.png") });
  } finally { daemon?.stop(); herdr.stop(); }
});

test("Discard names each ignored nested repository and deletes them only after explicit confirmation", { tag: "@platform" }, async ({ page }) => {
  const herdr = await startHerdr();
  let daemon: Daemon | null = null;
  try {
    const fixture = await deletionFixture(herdr);
    const names = ["node_modules/vendor/beta", "target/deep/alpha"];
    for (const name of names) {
      const nested = path.join(fixture.worktree, name);
      fs.mkdirSync(nested, { recursive: true });
      git(nested, ["init", "-q"]);
    }
    daemon = await startHided(herdr, "worktree-ignored-preflight");
    const dialog = await openDeletion(page, daemon);
    const discard = dialog.locator("label").filter({ has: page.locator("[data-delete-discard]") });
    for (const name of names) await expect(discard).toContainText(name, { timeout: 30_000 });
    const confirm = dialog.locator("[data-delete-confirm]");
    await expect(confirm).toBeDisabled();
    expect(fs.existsSync(fixture.worktree)).toBe(true);
    const evidence = process.env.HIDE_E2E_SCREENSHOT_DIR;
    if (evidence) await dialog.screenshot({ path: path.join(evidence, "worktree-ignored-repositories.png") });
    await dialog.locator("[data-delete-discard]").click();
    await confirm.click();
    await expect(dialog.locator('[data-delete-result="finished"]')).toBeVisible({ timeout: 60_000 });
    expect(fs.existsSync(fixture.worktree)).toBe(false);
  } finally { daemon?.stop(); herdr.stop(); }
});

test("an open deletion dialog receives refreshed Git facts and retires Discard through an A-to-B-to-A return", { tag: "@platform" }, async ({ page }) => {
  const herdr = await startHerdr({ agents: false });
  let daemon: Daemon | null = null;
  try {
    const fixture = await deletionFixture(herdr);
    const names = ["node_modules/vendor/beta", "target/deep/alpha"];
    for (const name of names) {
      const nested = path.join(fixture.worktree, name);
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
    daemon = await startHided(herdr, "worktree-refreshed-consent");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await expect(page.locator("[data-main-screen]")).toBeVisible({ timeout: 20_000 });
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
    git(fixture.repo, ["worktree", "lock", "--reason", reason, fixture.worktree]);
    await refresh();
    await expect(dialog.locator("[data-delete-blocked]")).toContainText(reason, { timeout: 15_000 });
    await expect(confirm).toHaveCount(0);
    git(fixture.repo, ["worktree", "unlock", fixture.worktree]);
    await refresh();
    await expect(dialog.locator("[data-delete-blocked]")).toHaveCount(0, { timeout: 15_000 });
    await expect(discard).not.toBeChecked();
    await expect(confirm).toBeDisabled();
    await expect.poll(() => loading.slice(-2), { timeout: 15_000 }).toEqual([true, false]);
    await discard.click();

    const late = path.join(fixture.worktree, "target/late");
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
    const pane = herdr.run(["pane", "get", fixture.pane]) as { result: { pane: { pane_id: string } } };
    expect(pane.result.pane.pane_id).toBe(fixture.pane);
    expect(fs.existsSync(fixture.worktree)).toBe(true);

    await discard.click();
    await confirm.click();
    await expect(dialog.locator('[data-delete-result="finished"]')).toBeVisible({ timeout: 60_000 });
    expect(payloads.get("remove_worktree")?.discard_changes).toBe(true);
    expect(payloads.get("remove_worktree")?.expected_ignored_repositories).toEqual(names);
    expect(rendererConnections).toBe(1);
    expect(owningConnections).toBe(1);
    expect(rendererClosed).toBe(false);
    expect(fs.existsSync(fixture.worktree)).toBe(false);
  } finally {
    if (!page.isClosed()) await page.evaluate(() => {
      (window as Window & { __preflightRestoreSend?: () => void }).__preflightRestoreSend?.();
    }).catch(() => { /* the renderer may already have closed */ });
    daemon?.stop();
    herdr.stop();
  }
});

function git(cwd: string, args: string[]): string {
  return execFileSync("git", ["-c", "commit.gpgsign=false", "-c", "user.name=e2e", "-c", "user.email=e2e@example.invalid", "-c", "init.defaultBranch=main", ...args], { cwd, encoding: "utf8" });
}

async function prompt(herdr: HerdrFixture, pane: string): Promise<void> {
  const deadline = Date.now() + 15_000;
  while (Date.now() < deadline) {
    const read = spawnSync(herdr.bin, ["pane", "read", pane, "--source", "visible", "--format", "text"], { env: herdr.env, encoding: "utf8", timeout: 10_000 });
    if (read.status === 0 && read.stdout.includes("fixture %")) return;
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error(`no prompt in pane ${pane}`);
}

// @platform: Deleting a worktree stops an agent's processes and removes its folder.
test("a dirty worktree with an unmerged branch and an agent is deleted once both boxes are ticked", { tag: "@platform" }, async ({ page }) => {
  await page.setViewportSize({ width: 1400, height: 900 });
  const herdr = await startHerdr();
  let daemon: Daemon | null = null;
  try {
    const repo = path.join(herdr.root, "repo");
    fs.mkdirSync(repo);
    git(repo, ["init"]);
    fs.writeFileSync(path.join(repo, "README.md"), "# repo\n");
    git(repo, ["add", "README.md"]);
    git(repo, ["commit", "-m", "initial"]);
    // The base is read from origin/HEAD, as a clone has it.
    git(repo, ["update-ref", "refs/remotes/origin/main", "HEAD"]);
    git(repo, ["symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/main"]);
    const worktree = path.join(herdr.root, "repo-delete");
    git(repo, ["worktree", "add", "-b", BRANCH, worktree]);
    git(worktree, ["commit", "--allow-empty", "-m", "not merged anywhere"]);
    fs.writeFileSync(path.join(worktree, "draft.txt"), "work in progress\n");

    const created = herdr.run(["workspace", "create", "--cwd", worktree, "--label", "repo-delete", "--env", `PATH=${herdr.fixturePath}`, "--no-focus"]) as {
      result: { root_pane: { pane_id: string } };
    };
    const pane = created.result.root_pane.pane_id;
    await prompt(herdr, pane);
    herdr.run(["agent", "start", "agent-delete", "--kind", "claude", "--pane", pane]);
    labelAgent(herdr, pane, { task: "삭제 대상 작업 정리" });

    daemon = await startHided(herdr, "worktree-delete");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await expect(page.locator("[data-main-screen]")).toBeVisible({ timeout: 20_000 });

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
    herdr.stop();
  }
});
