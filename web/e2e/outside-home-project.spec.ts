// A project outside the daemon's home on this machine (PRD wsl-device B15,
// B19-B21), against a private Herdr and hided: it is registered only once
// confirmed, removing it removes that permission so adding it again asks
// again, and once registered it makes, selects and reads a worktree like a
// project inside home.
import { expect, test, type Page } from "@playwright/test";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { dumpOnFailure, herdrTabs } from "./failure-dump";
import { startHerdr } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { sendEvent, showExplorer } from "./wire";
import { toPage } from "../../desktop/src/main/wirePath";

test.describe.configure({ timeout: 180_000 });

const BRANCH = "outside-feature";

function git(cwd: string, args: string[]): string {
  return execFileSync("git", ["-c", "user.name=e2e", "-c", "user.email=e2e@example.invalid", "-c", "init.defaultBranch=main", "-c", "commit.gpgsign=false", ...args], { cwd, encoding: "utf8" });
}

type Seen = { paths: string[] | null; unread: Set<number>; paned: Set<string> };
type Listed = { id: string; checkouts?: { tabs?: { panes?: unknown[] }[] }[] };

/**
 * The folders the core lists as registered, the projects it lists with a
 * pane, and when it refused a worktree because it had not read the
 * repository's worktrees yet, from the snapshots the page receives.
 */
function watchCore(page: Page): Seen {
  const seen: Seen = { paths: null, unread: new Set(), paned: new Set() };
  page.on("websocket", (socket) => socket.on("framereceived", ({ payload }) => {
    try {
      const frame = JSON.parse(String(payload)) as { payload?: { rest?: { ui_state?: { workspace_registrations?: { path: string }[] }; navigator?: { workspaces?: Listed[] }; status?: { last_error?: { kind: string; occurred_at: number } | null } } } };
      const listed = frame.payload?.rest?.ui_state?.workspace_registrations;
      if (listed) seen.paths = listed.map((registration) => registration.path);
      const rows = frame.payload?.rest?.navigator?.workspaces;
      if (rows) seen.paned = new Set(rows.filter((row) => row.checkouts?.some((checkout) => checkout.tabs?.some((tab) => (tab.panes?.length ?? 0) > 0))).map((row) => row.id));
      const error = frame.payload?.rest?.status?.last_error;
      if (error?.kind === "worktree.create_unread") seen.unread.add(error.occurred_at);
    } catch { /* Terminal frames are binary. */ }
  }));
  return seen;
}

/** The worktrees Git lists for `repo`, other than `repo` itself. */
function worktreesOf(repo: string): string[] {
  return git(repo, ["worktree", "list", "--porcelain"]).split("\n").filter((line) => line.startsWith("worktree ")).map((line) => line.slice("worktree ".length)).filter((folder) => fs.realpathSync.native(folder) !== repo);
}

test("a project outside home is asked about again after its removal, and its worktree is made, selected and read", async ({ page }) => {
  await page.setViewportSize({ width: 1400, height: 900 });
  const herdr = await startHerdr();
  let daemon: Daemon | null = null;
  try {
    // The fixture root is not the daemon's home, so this repository is outside it.
    const folder = path.join(herdr.root, "outside-repo");
    fs.mkdirSync(folder);
    const repo = fs.realpathSync.native(folder);
    git(repo, ["init", "-q"]);
    fs.writeFileSync(path.join(repo, "README.md"), "# outside\n");
    git(repo, ["add", "README.md"]);
    git(repo, ["commit", "-q", "-m", "initial"]);
    daemon = await startHided(herdr, "outside-home-project");
    expect(path.relative(daemon.home, repo).startsWith("..")).toBe(true);
    const registered = watchCore(page);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await expect(page.locator("[data-main-screen]")).toBeVisible({ timeout: 20_000 });

    const add = (confirmed: boolean) => ({ path: toPage(repo), label: "outside-repo", initialize_git: false, ...(confirmed ? { confirmed_outside_home: toPage(repo) } : {}) });
    const asked = async () => {
      const answer = await sendEvent(page, daemon!, "create_workspace", add(false), "path_refused");
      return (answer?.payload as { reason?: string } | undefined)?.reason;
    };
    const project = page.locator("[data-project]").filter({ has: page.locator("[data-project-row]", { hasText: /^outside-repo/ }) });

    // B15: asked first, registered once confirmed.
    expect(await asked()).toBe("needs_confirmation");
    await sendEvent(page, daemon, "create_workspace", add(true));
    await expect(project).toBeVisible({ timeout: 30_000 });
    await expect.poll(() => registered.paths, { timeout: 30_000 }).toContain(toPage(repo));

    // B20: removing it removes the permission, so adding it again asks again.
    // Removal closes the panes the core lists under the project, so it waits
    // for the first one: removed before the core had published it, the
    // project lost its registration with Herdr's workspace still open there
    // (with-hide/hide-private#311).
    const id = await project.first().getAttribute("data-project");
    await expect.poll(() => registered.paned.has(id!), { timeout: 30_000 }).toBe(true);
    await sendEvent(page, daemon, "remove_workspace", { workspace_id: id });
    await dumpOnFailure("registrations after removal", () => ({ registered: registered.paths, paned: [...registered.paned], herdr: herdrTabs(herdr) }), () =>
      expect.poll(() => registered.paths, { timeout: 30_000 }).not.toContain(toPage(repo)),
    );
    expect(await asked()).toBe("needs_confirmation");
    await sendEvent(page, daemon, "create_workspace", add(true));
    await expect.poll(() => registered.paths, { timeout: 30_000 }).toContain(toPage(repo));

    // B19: the registered project makes a worktree where Herdr puts one,
    // lists it under itself, and its files are read once it is selected.
    // The core reads a project's worktrees off its lock after registering it,
    // and a worktree asked for before that read is refused as one to ask for
    // again in a moment (`worktree.create_unread`), so it is asked again.
    const create = () => sendEvent(page, daemon!, "create_worktree", { repository_root: toPage(repo), branch: BRANCH, base_branch: null, agent_kind: null, purpose: null });
    let refusals = registered.unread.size;
    await create();
    await expect.poll(async () => {
      if (registered.unread.size > refusals) {
        refusals = registered.unread.size;
        await create();
      }
      return worktreesOf(repo).length;
    }, { timeout: 60_000 }).toBe(1);
    const worktree = project.locator("[data-checkout-row]").filter({ has: page.locator(`[data-checkout][aria-label^="${BRANCH}"]`) });
    // The made worktree comes forward as the front checkout once the core
    // lists its pane, so it stands on its own line rather than in the fold of
    // worktrees with no agents (docs/ARCHITECTURE.md, a created worktree's pane).
    await expect(worktree.locator("[data-checkout]").first()).toHaveAttribute("aria-current", "true", { timeout: 60_000 });
    const [made] = worktreesOf(repo);
    await worktree.locator("[data-checkout]").first().click();
    await showExplorer(page);
    await expect(page.locator(`[data-explorer-row="${toPage(fs.realpathSync.native(made))}/README.md"]`)).toBeVisible({ timeout: 30_000 });
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});
