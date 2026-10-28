// A project outside the daemon's home on this machine (PRD wsl-device B15,
// B19-B21), against a private Herdr and hided: it is registered only once
// confirmed, removing it removes that permission so adding it again asks
// again, and once registered it makes, selects and reads a worktree like a
// project inside home.
import { expect, test, type Page } from "@playwright/test";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { startHerdr } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { sendEvent, showExplorer } from "./wire";
import { toPage } from "../../desktop/src/main/wirePath";

test.describe.configure({ timeout: 180_000 });

const BRANCH = "outside-feature";

function git(cwd: string, args: string[]): string {
  return execFileSync("git", ["-c", "user.name=e2e", "-c", "user.email=e2e@example.invalid", "-c", "init.defaultBranch=main", "-c", "commit.gpgsign=false", ...args], { cwd, encoding: "utf8" });
}

/** The folders the core lists as registered, from the snapshots the page receives. */
function registrations(page: Page): { paths: string[] | null } {
  const seen: { paths: string[] | null } = { paths: null };
  page.on("websocket", (socket) => socket.on("framereceived", ({ payload }) => {
    try {
      const frame = JSON.parse(String(payload)) as { payload?: { rest?: { ui_state?: { workspace_registrations?: { path: string }[] } } } };
      const listed = frame.payload?.rest?.ui_state?.workspace_registrations;
      if (listed) seen.paths = listed.map((registration) => registration.path);
    } catch { /* Terminal frames are binary. */ }
  }));
  return seen;
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
    const registered = registrations(page);
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
    const id = await project.first().getAttribute("data-project");
    await sendEvent(page, daemon, "remove_workspace", { workspace_id: id });
    await expect.poll(() => registered.paths, { timeout: 30_000 }).not.toContain(toPage(repo));
    expect(await asked()).toBe("needs_confirmation");
    await sendEvent(page, daemon, "create_workspace", add(true));
    await expect.poll(() => registered.paths, { timeout: 30_000 }).toContain(toPage(repo));

    // B19: the registered project makes a worktree where Herdr puts one,
    // lists it under itself, and its files are read once it is selected.
    await sendEvent(page, daemon, "create_worktree", { repository_root: toPage(repo), branch: BRANCH, base_branch: null, agent_kind: null, purpose: null });
    const worktree = project.locator("[data-checkout-row]").filter({ has: page.locator(`[data-checkout][aria-label^="${BRANCH}"]`) });
    // A worktree with no agent yet stands in this fold (session UI B9).
    const quiet = project.getByRole("button", { name: /^Worktrees with no agents/ });
    await expect(quiet).toBeVisible({ timeout: 60_000 });
    await quiet.click();
    await expect(worktree).toBeVisible({ timeout: 30_000 });
    const listed = git(repo, ["worktree", "list", "--porcelain"]).split("\n").filter((line) => line.startsWith("worktree ")).map((line) => line.slice("worktree ".length));
    const made = listed.find((folder) => fs.realpathSync.native(folder) !== repo);
    expect(made, listed.join(", ")).toBeTruthy();
    await worktree.locator("[data-checkout]").first().click();
    await showExplorer(page);
    await expect(page.locator(`[data-explorer-row="${toPage(fs.realpathSync.native(made!))}/README.md"]`)).toBeVisible({ timeout: 30_000 });
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});
