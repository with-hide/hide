import { expect, test, type Page } from "@playwright/test";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { startHerdr, type HerdrFixture } from "./herdr-fixture";
import { waitForTerminal } from "./group-start-helper";
import { startHided, type Daemon } from "./hided-fixture";
import { enterWorkspace, screenshot } from "./wire";
import { chord } from "./chords";
import { dumpOnFailure, tabsBothSides } from "./failure-dump";

// PRD checkout-workspace-binding B1, B2, B13: a tab Hide creates goes to the
// checkout's owner Herdr workspace, never to the workspace its other tabs sit
// in. Two quick terminal requests each
// reuse one owner. These sequential starts do not exercise concurrent owner creation.

type Workspace = {
  workspace_id: string;
  label: string;
  tab_count?: number;
  worktree?: { checkout_path: string } | null;
};

function workspaces(herdr: HerdrFixture): Workspace[] {
  const listed = herdr.run(["workspace", "list"]) as { result: { workspaces: Workspace[] } };
  return listed.result.workspaces;
}

function tabCount(herdr: HerdrFixture, workspaceId: string): number {
  const listed = herdr.run(["tab", "list", "--workspace", workspaceId]) as { result: { tabs: unknown[] } };
  return listed.result.tabs.length;
}

/** The agent tab strip holds `count` tabs; if it never does, the failure carries Herdr's tab lists and the strip as drawn. */
async function expectTabs(page: Page, herdr: HerdrFixture, count: number, timeout?: number): Promise<void> {
  await dumpOnFailure("agent tabs", () => tabsBothSides(herdr, page), () =>
    expect(page.locator('[data-agent-tab-bar] [role="tab"]')).toHaveCount(count, timeout === undefined ? undefined : { timeout }),
  );
}

/** Two quick new-tab requests must both create terminals in the checkout's owner. */
async function twoTerminals(page: Page, herdr: HerdrFixture): Promise<void> {
  const shownBefore = await page.locator('[data-agent-tab-bar] [role="tab"]').count();
  const before = workspaces(herdr).reduce((sum, workspace) => sum + tabCount(herdr, workspace.workspace_id), 0);
  await page.locator("[data-terminal-host]").first().click();
  await page.keyboard.press(chord("new_tab"));
  await page.keyboard.press(chord("new_tab"));
  await expect.poll(() => workspaces(herdr).reduce((sum, workspace) => sum + tabCount(herdr, workspace.workspace_id), 0)).toBe(before + 2);
  // Server creation can precede the page snapshot. Capture the selected tab
  // only after both requested tabs have reached the strip.
  await expectTabs(page, herdr, shownBefore + 2);
  await waitForTerminal(page);
}

test("A new tab in a Git checkout goes to the workspace Herdr binds to it, not the unbound one it was asked from", async ({ page }) => {
  test.setTimeout(120_000);
  const herdr = await startHerdr({ agents: false });
  let daemon: Daemon | null = null;
  try {
    const root = path.join(herdr.root, "fixture");
    const git = (...args: string[]) => execFileSync("git", ["-C", root, ...args], { env: herdr.env });
    fs.mkdirSync(path.join(root, "src"));
    fs.writeFileSync(path.join(root, "src", "note.txt"), "original\n");
    git("init", "-q", "-b", "main"); git("add", ".");
    git("-c", "user.name=Test", "-c", "user.email=test@example.com", "commit", "-qm", "Initial files");
    // The Mac's shape (w9J and w8P): Herdr binds the fixture's workspace to
    // the checkout, and a second workspace, unbound, holds a tab in it.
    const bound = herdr.run(["worktree", "open", "--path", root, "--cwd", root, "--no-focus"]) as { result: { workspace: { workspace_id: string } } };
    const ownerId = bound.result.workspace.workspace_id;
    const created = herdr.run(["workspace", "create", "--cwd", path.join(root, "src"), "--label", "home-graph", "--focus"]) as {
      result: { workspace: { workspace_id: string }; tab: { tab_id: string } };
    };
    const unboundId = created.result.workspace.workspace_id;
    expect(workspaces(herdr).find((workspace) => workspace.workspace_id === unboundId)?.worktree ?? null).toBeNull();

    daemon = await startHided(herdr, "checkout-owner-git");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    const tabs = page.locator('[data-agent-tab-bar] [role="tab"]');
    await expectTabs(page, herdr, 2);
    // Asked from the unbound workspace's own pane.
    await page.locator(`[data-agent-tab-bar] [data-tab="${created.result.tab.tab_id}"]`).click();
    await twoTerminals(page, herdr);
    await expectTabs(page, herdr, 4, 20_000);
    expect(workspaces(herdr)).toHaveLength(2);
    expect(tabCount(herdr, ownerId)).toBe(3);
    expect(tabCount(herdr, unboundId)).toBe(1);
    // B6: no Herdr workspace label names a tab.
    await expect(tabs.filter({ hasText: "home-graph" })).toHaveCount(0);
    await screenshot(page, "checkout-owner-git");
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});

test("With no workspace bound, Herdr binds the unbound one already at the checkout, and Hide leaves its name", async ({ page }) => {
  test.setTimeout(120_000);
  const herdr = await startHerdr({ agents: false });
  let daemon: Daemon | null = null;
  try {
    const root = path.join(herdr.root, "fixture");
    const git = (...args: string[]) => execFileSync("git", ["-C", root, ...args], { env: herdr.env });
    fs.writeFileSync(path.join(root, "note.txt"), "original\n");
    git("init", "-q", "-b", "main"); git("add", ".");
    git("-c", "user.name=Test", "-c", "user.email=test@example.com", "commit", "-qm", "Initial files");
    const [unbound] = workspaces(herdr);
    expect(unbound.worktree ?? null).toBeNull();

    daemon = await startHided(herdr, "checkout-owner-adopt");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    await expectTabs(page, herdr, 1);
    await twoTerminals(page, herdr);
    await expectTabs(page, herdr, 3, 20_000);
    // worktree.open answered already_open with the fixture's own workspace.
    const after = workspaces(herdr);
    expect(after).toHaveLength(1);
    expect(after[0].workspace_id).toBe(unbound.workspace_id);
    expect(fs.realpathSync(after[0].worktree!.checkout_path)).toBe(fs.realpathSync(root));
    expect(after[0].label).toBe(unbound.label);
    expect(tabCount(herdr, unbound.workspace_id)).toBe(3);
    await screenshot(page, "checkout-owner-adopt");
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});

// @platform: A linked worktree's path: macOS reaches /tmp through a symlink and ignores case.
test("A new tab in a linked worktree with no workspace opens one bound to it, named by its branch, and the next tab reuses it", { tag: "@platform" }, async ({ page }) => {
  test.setTimeout(120_000);
  await page.setViewportSize({ width: 1400, height: 900 });
  const herdr = await startHerdr({ agents: false });
  let daemon: Daemon | null = null;
  try {
    const root = path.join(herdr.root, "fixture");
    const linked = path.join(herdr.root, "fixture-feature");
    const git = (...args: string[]) => execFileSync("git", ["-C", root, ...args], { env: herdr.env });
    fs.writeFileSync(path.join(root, "note.txt"), "original\n");
    git("init", "-q", "-b", "main"); git("add", ".");
    git("-c", "user.name=Test", "-c", "user.email=test@example.com", "commit", "-qm", "Initial files");
    git("worktree", "add", "-q", "-b", "feature", linked);
    const before = workspaces(herdr).map((workspace) => workspace.workspace_id);
    const boundTo = (checkout: string) =>
      workspaces(herdr).filter((workspace) => workspace.worktree && fs.realpathSync(workspace.worktree.checkout_path) === fs.realpathSync(checkout));

    daemon = await startHided(herdr, "checkout-owner-linked");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await expect(page.locator("[data-main-screen]")).toBeVisible({ timeout: 20_000 });
    const project = page.locator("[data-project]").filter({ has: page.locator("[data-project-row]", { hasText: /^fixture/ }) });
    const feature = project.locator("[data-checkout-row]").filter({ has: page.locator('[data-checkout][aria-label^="feature"]') });
    // Session UI B9: a clean worktree with no agent starts in this fold.
    await project.getByRole("button", { name: /^No agents/ }).click();
    await expect(feature).toBeVisible({ timeout: 30_000 });
    const newTabHere = async () => {
      await feature.locator("[data-checkout-menu]").click({ button: "right" });
      const menu = page.getByRole("menu", { name: "feature actions" });
      await menu.locator('[data-menu-item="new_tab_here"]').click();
    };

    await newTabHere();
    await waitForTerminal(page);
    await expect.poll(() => boundTo(linked).length, { timeout: 20_000 }).toBe(1);
    const [owner] = boundTo(linked);
    expect(before).not.toContain(owner.workspace_id);
    // D-14: the workspace Hide opened is named as the sidebar names the checkout.
    await expect.poll(() => boundTo(linked)[0].label, { timeout: 10_000 }).toBe("feature");
    expect(tabCount(herdr, owner.workspace_id)).toBe(1);
    // Opening the terminal moves the checkout out of the empty fold.
    // The next menu is opened on the current row after that transition.
    await expect(feature.locator("[data-checkout]")).toHaveAttribute("aria-current", "true");
    await expect(project.getByRole("button", { name: /^No agents/ })).toHaveCount(0);

    await newTabHere();
    await waitForTerminal(page);
    await expect.poll(() => tabCount(herdr, owner.workspace_id), { timeout: 20_000 }).toBe(2);
    expect(boundTo(linked)).toHaveLength(1);
    await screenshot(page, "checkout-owner-linked");
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});

test("A new tab in a plain folder opens one marked workspace and the next tab reuses it", async ({ page }) => {
  test.setTimeout(120_000);
  const herdr = await startHerdr({ agents: false });
  let daemon: Daemon | null = null;
  try {
    const [unmarked] = workspaces(herdr);
    daemon = await startHided(herdr, "checkout-owner-folder");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    await expectTabs(page, herdr, 1);

    await twoTerminals(page, herdr);
    await expectTabs(page, herdr, 3, 20_000);
    const after = workspaces(herdr);
    expect(after).toHaveLength(2);
    const owner = after.find((workspace) => workspace.workspace_id !== unmarked.workspace_id)!;
    expect(owner.worktree ?? null).toBeNull();
    expect(tabCount(herdr, owner.workspace_id)).toBe(2);
    expect(tabCount(herdr, unmarked.workspace_id)).toBe(1);
    await screenshot(page, "checkout-owner-folder");
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});
