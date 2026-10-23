// One sidebar on an isolated Herdr and daemon: new checkouts open by default,
// roots own direct-child badges, and project/checkout folds persist.
// Existing row geometry, context menus and type scaling remain stable.

import { expect, test, type Locator, type Page } from "@playwright/test";
import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { declareParent, elsewhereTab, finishFixtureTurn, labelAgent, setFixtureLifecycle, startHerdr, type HerdrFixture } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { countSent, keyboardFocus, rest, rowGeometry, screenshot, sendEvent, sidebarColumns, sidebarOverflow, sidebarRowsFit } from "./wire";
import { chord } from "./chords";
import { animationsFinished, quietFor } from "./wait";

test.describe.configure({ timeout: 180_000 });

const promptNow = Date.parse("2026-10-03T09:00:00Z");

function git(cwd: string, args: string[]): void {
  execFileSync("git", ["-c", "user.name=e2e", "-c", "user.email=e2e@example.invalid", "-c", "init.defaultBranch=main", "-c", "commit.gpgsign=false", ...args], { cwd, stdio: "ignore" });
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

/** A Herdr workspace at `cwd`, with a fake `claude` agent titled `task` unless it is null. */
async function workspaceAt(herdr: HerdrFixture, cwd: string, task: string | null, at?: number): Promise<string> {
  const created = herdr.run(["workspace", "create", "--cwd", cwd, "--label", path.basename(cwd), "--env", `PATH=${herdr.fixturePath}`, "--no-focus"]) as {
    result: { root_pane: { pane_id: string } };
  };
  const pane = created.result.root_pane.pane_id;
  await prompt(herdr, pane);
  if (task) {
    herdr.run(["agent", "start", `agent-${path.basename(cwd)}`, "--kind", "claude", "--pane", pane]);
    labelAgent(herdr, pane, { task, at });
  }
  return pane;
}

async function open(page: Page, daemon: Daemon): Promise<void> {
  await page.goto(`${daemon.origin}/#token=${daemon.token}`);
  await expect(page.locator("[data-main-screen]").or(page.locator("[data-workspace-screen]"))).toBeVisible({ timeout: 20_000 });
}

/** Whether the project row's Overview button, its click target and focus ring, fills the row's whole height. */
async function projectButtonFills(page: Page): Promise<boolean> {
  return page.locator("[data-project-row]").first().evaluate((button) => {
    const row = button.parentElement!.getBoundingClientRect();
    return Math.abs(button.getBoundingClientRect().height - row.height) < 0.5;
  });
}

/** The font size, in px, of a `text-body` probe placed inside `selector`: what the interface's body text measures there. */
async function probeTextSize(page: Page, selector: string): Promise<number> {
  return page.evaluate((within) => {
    const probe = document.createElement("span");
    probe.className = "text-body";
    probe.textContent = "x";
    document.querySelector(within)!.append(probe);
    const size = Number.parseFloat(getComputedStyle(probe).fontSize);
    probe.remove();
    return size;
  }, selector);
}

async function chooseTheme(page: Page, theme: "light" | "dark"): Promise<void> {
  await page.keyboard.press(chord("settings"));
  await expect(page.locator('[data-settings="true"]')).toBeVisible();
  await page.locator('[data-settings-tab="general"]').click();
  await page.locator(`[data-theme-option="${theme}"]`).click();
  await expect(page.locator("html")).toHaveClass(new RegExp(`\\b${theme}\\b`));
  await page.keyboard.press("Escape");
  await expect(page.locator('[data-settings="true"]')).toHaveCount(0);
  await animationsFinished(page);
}

test("a delegated-child-only checkout follows the front checkout across project reopening", async ({ page }) => {
  const herdr = await startHerdr();
  let daemon: Daemon | null = null;
  try {
    const repo = path.join(herdr.root, "repo");
    fs.mkdirSync(repo);
    git(repo, ["init"]);
    git(repo, ["commit", "--allow-empty", "-m", "initial"]);
    const worktree = path.join(herdr.root, "repo-rows");
    git(repo, ["worktree", "add", "-b", "feature/sidebar-rows", worktree]);
    const notes = path.join(herdr.root, "notes");
    fs.mkdirSync(notes);
    const mainPane = await workspaceAt(herdr, repo, "메인 체크아웃 정리");
    const rowsPane = await workspaceAt(herdr, worktree, "사이드바 행 구현");
    await workspaceAt(herdr, notes, "회의록 요약 정리");
    daemon = await startHided(herdr, "sidebar-child-only");
    const last = new Map<string, Record<string, unknown>>();
    countSent(page, last);
    await open(page, daemon);
    const project = page.locator("[data-project]").filter({ has: page.locator("[data-project-row]", { hasText: /^repo/ }) });
    const primary = project.locator('[data-checkout][aria-label^="main"]');
    const feature = project.locator("[data-checkout-row]").filter({ has: page.locator('[data-checkout][aria-label^="feature/sidebar-rows"]') });
    const toggle = project.locator("[data-project-toggle]");
    const projectRow = project.locator("[data-project-row]");
    await feature.locator("[data-checkout]").click();
    await expect(feature.locator("[data-checkout]")).toHaveAttribute("aria-current", "true");
    const featureId = await feature.locator("[data-checkout]").getAttribute("data-checkout");
    await page.locator('[data-project] [data-checkout][aria-label^="notes"]').click();
    declareParent(herdr, rowsPane, mainPane);
    const parentRow = project.locator(`[data-pane="${mainPane}"]`);
    const chevron = parentRow.locator(`[data-tree-chevron="${mainPane}"]`);
    await expect(chevron).toBeVisible();
    await expect(feature).toHaveCount(0);
    await toggle.click();
    await expect(toggle).toHaveAttribute("aria-expanded", "false");
    await open(page, daemon);
    await expect(toggle).toHaveAttribute("aria-expanded", "false");
    await projectRow.click();
    await expect(toggle).toHaveAttribute("aria-expanded", "true");
    expect(last.get("focus_checkout")?.checkout_id).toBe(featureId);
    // Reopening returns to the last checkout: the front exception keeps its line.
    await expect(feature).toHaveCount(1);
    await expect(feature.locator("[data-checkout]")).toHaveAttribute("aria-current", "true");
    await expect(page.locator(`nav[data-sidebar] [data-pane="${rowsPane}"]`)).toHaveCount(0);
    // Returning to its parent hides the clean child-only checkout again.
    await primary.click();
    await expect(primary).toHaveAttribute("aria-current", "true");
    await expect(feature).toHaveCount(0);
    await chevron.click();
    const child = parentRow.locator("xpath=..").locator(`[data-pane="${rowsPane}"]`);
    await expect(child).toHaveAttribute("data-depth", "1");
    await child.locator("[data-agent-open]").click();
    await expect(page.locator(`[data-pane-view="${rowsPane}"]`)).toBeVisible();
    await expect(feature.locator("[data-checkout]")).toHaveAttribute("aria-current", "true");
    await screenshot(page, "sidebar-child-only-front");
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});

test("the sidebar: kind, age, status badges, opened checkouts and folded projects", async ({ page }) => {
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
    const worktree = path.join(herdr.root, "repo-rows");
    git(repo, ["worktree", "add", "-b", "feature/sidebar-rows", worktree]);
    git(worktree, ["commit", "--allow-empty", "-m", "rows"]);
    // The branch description is a purpose the core reads from Git itself.
    git(repo, ["config", "branch.feature/sidebar-rows.description", "Projects 탭 행 다시 그리기"]);
    const emptyTree = path.join(herdr.root, "repo-empty");
    git(repo, ["worktree", "add", "-b", "empty", emptyTree]);
    const notes = path.join(herdr.root, "notes");
    fs.mkdirSync(notes);

    const mainPane = await workspaceAt(herdr, repo, "메인 체크아웃 정리");
    const rowsPane = await workspaceAt(herdr, worktree, "사이드바 행 구현", promptNow - 3 * 3_600_000);
    const notesPane = await workspaceAt(herdr, notes, "회의록 요약 정리", promptNow - 2 * 86_400_000);

    daemon = await startHided(herdr, "projects-sidebar");
    const last = new Map<string, Record<string, unknown>>();
    const sent = countSent(page, last);
    await open(page, daemon);

    const project = page.locator("[data-project]").filter({ has: page.locator("[data-project-row]", { hasText: /^repo/ }) });
    const projectToggle = project.locator("[data-project-toggle]");
    await expect(projectToggle).toHaveAttribute("aria-expanded", "true");
    const primary = project.locator("[data-checkout-row]", { hasText: /^main/ });
    const feature = project.locator("[data-checkout-row]").filter({ has: page.locator(`[data-checkout][aria-label^="feature/sidebar-rows"]`) });
    await expect(primary.locator("[data-checkout]")).toHaveAttribute("data-checkout-kind", "primary");
    await expect(feature.locator("[data-checkout]")).toHaveAttribute("data-checkout-kind", "branch");
    // The primary checkout leads its project although the worktree moved later.
    await expect(project.locator("[data-checkout]").first()).toHaveAttribute("data-checkout-kind", "primary");
    // The worktree's commit was just made: its age is the first minute.
    await expect(feature.locator("[data-checkout-age]")).toHaveText("now");

    // Session UI B5: checkout agents start open; the operator can fold them, and its
    // status badge counts its agents under the mark each row draws, and its one
    // quiet agent draws none (status-marks B19). Line two is the purpose with the age ending it. The project's
    // badge counts both checkouts' agents.
    const featureToggle = feature.locator("[data-checkout-toggle]");
    await expect(featureToggle).toHaveAttribute("aria-expanded", "true");
    await featureToggle.click();
    await expect(featureToggle).toHaveAttribute("aria-expanded", "false");
    await expect(feature.locator("[data-checkout-agents-open]")).toHaveCount(0);
    await expect(feature.locator("[data-checkout-status]")).toHaveCount(0);
    await expect(feature.locator("[data-purpose]")).toHaveText("Projects 탭 행 다시 그리기");
    const lineOf = async (part: Locator) => Math.round((await part.boundingBox())!.y);
    expect(await lineOf(feature.locator("[data-checkout-age]"))).toBeGreaterThan(await lineOf(feature.getByText("feature/sidebar-rows", { exact: true })));
    expect(await lineOf(feature.locator("[data-checkout-age]"))).toBe(await lineOf(feature.locator("[data-purpose]")));
    // agent-hierarchy-screens B35: while its agents are open, the open
    // checkout is one line; its purpose comes back when they fold (below).
    await expect(primary.locator("[data-purpose]")).toHaveCount(0);
    // The project row carries no agent counts: its checkouts and their agents say that.
    await expect(project.locator("[data-project-row]").locator("xpath=..").locator("[data-badge-part]")).toHaveCount(0);
    await expect(project.locator("[data-project-row]")).toHaveAccessibleName("repo");
    await screenshot(page, "projects-sidebar-closed");

    // sidebar-readability B2, B4, B7: the fold sits on the right in a slot
    // kept at rest. A folded chevron is always shown, an unfolded one waits for
    // the pointer; nothing on the row stands for its menu, which a right-click
    // or ⇧F10 opens; the age stays while it is open; and hover, keyboard
    // focus, an open menu or a selection move neither the name, the age, the
    // row's height nor the row after it.
    const primaryRow = primary.locator("[data-checkout]").locator("xpath=..");
    const primaryParts = [primary.getByText("main", { exact: true }), primary.locator("[data-checkout-age]")];
    const primaryToggle = primary.locator("[data-checkout-toggle]");
    await expect(primaryToggle).toHaveAttribute("aria-expanded", "true");
    const togglesBefore = sent.get("checkout_agents_toggle") ?? 0;
    await primaryToggle.click();
    // The fold is the core's: the row closes when its snapshot arrives, not on
    // the click, and the rest geometry is measured on the closed row.
    await expect(primaryToggle).toHaveAttribute("aria-expanded", "false");
    expect(sent.get("checkout_agents_toggle") ?? 0).toBe(togglesBefore + 1);
    // PRD sidebar-typography B4: folded, line two is the purpose, here the
    // agent's title standing in for one, with the age ending it. Agents alone
    // earn no line two (projects.test.ts); a checkout with neither is one line.
    await expect(primary.locator("[data-purpose]")).toHaveText("메인 체크아웃 정리");
    expect(await lineOf(primary.locator("[data-checkout-age]"))).toBe(await lineOf(primary.locator("[data-purpose]")));
    const primaryMenu = page.getByRole("menu", { name: "main actions" });
    const beforeLooking = new Map(sent);
    await rest(page);
    const atRest = await rowGeometry(primaryRow, feature, primaryParts);
    expect(await projectButtonFills(page)).toBe(true);
    await expect(primaryToggle).toHaveCSS("opacity", "1");
    await expect(projectToggle).toHaveCSS("opacity", "0");
    await expect(page.locator("[data-project-list]").getByRole("button", { name: /actions$/ })).toHaveCount(0);
    await project.locator("[data-project-row]").hover();
    await expect(projectToggle).toHaveCSS("opacity", "1");
    await primaryRow.hover();
    expect(await rowGeometry(primaryRow, feature, primaryParts)).toEqual(atRest);
    await rest(page);
    await keyboardFocus(page, primary.locator("[data-checkout]"));
    expect(await rowGeometry(primaryRow, feature, primaryParts)).toEqual(atRest);
    await page.keyboard.press("Shift+F10");
    await expect(primaryMenu).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(primaryMenu).toHaveCount(0);
    await rest(page);
    await primaryRow.click({ button: "right" });
    await expect(primaryMenu).toBeVisible();
    await expect(primary.locator("[data-checkout-age]")).toHaveCSS("opacity", "1");
    expect(await rowGeometry(primaryRow, feature, primaryParts)).toEqual(atRest);
    await screenshot(page, "projects-sidebar-menu-open");
    await page.keyboard.press("Escape");
    await expect(primaryMenu).toHaveCount(0);
    // An unfolded chevron stays shown while its row's menu is open, with the
    // pointer and focus gone from the row.
    await project.locator("[data-project-row]").click({ button: "right" });
    const projectMenu = page.getByRole("menu", { name: /actions$/ });
    await expect(projectMenu).toBeVisible();
    await page.mouse.move(900, 600);
    await expect(projectToggle).toHaveCSS("opacity", "1");
    await page.keyboard.press("Escape");
    await expect(projectMenu).toHaveCount(0);
    // Looking at a row sends nothing: hover, focus and an open menu are the list's own.
    await quietFor(page, 300, "looking at a row sends nothing");
    expect([...sent].filter(([kind, count]) => count !== (beforeLooking.get(kind) ?? 0)).map(([kind]) => kind)).toEqual([]);
    const overview = page.locator("[data-sidebar-overview]");
    await expect(project.locator("[data-project-overview]")).toHaveCount(0);
    // All projects is the page a first run starts on, with no Workspace in front.
    await expect(page.locator("[data-overview-page]")).toBeVisible();
    await expect(overview).toHaveAttribute("aria-current", "page");
    await expect(project.locator("[data-project-row]")).not.toHaveAttribute("aria-current");
    await expect(project.locator("[data-project-row]").locator("xpath=..")).not.toHaveClass(/bg-secondary/);
    const beforeOpen = new Map(sent);
    await primary.locator("[data-checkout]").click();
    await expect(primary.locator("[data-checkout]")).toHaveAttribute("aria-current", "true");
    await expect(primaryToggle).toHaveAttribute("aria-expanded", "true");
    expect((sent.get("focus_checkout") ?? 0) - (beforeOpen.get("focus_checkout") ?? 0)).toBe(1);
    expect(sent.get("ui_state_update") ?? 0).toBe(beforeOpen.get("ui_state_update") ?? 0);
    expect(last.get("focus_checkout")?.expanded).toBe(true);
    await expect(overview).not.toHaveAttribute("aria-current");
    await screenshot(page, "row-click-expanded");
    await primary.locator("[data-checkout]").click();
    await expect(primaryToggle).toHaveAttribute("aria-expanded", "false");
    await expect(primary.locator("[data-checkout]")).toHaveAttribute("aria-current", "true");
    await keyboardFocus(page, primary.locator("[data-checkout]"));
    await page.keyboard.press("Enter");
    await expect(primaryToggle).toHaveAttribute("aria-expanded", "true");
    await page.keyboard.press("Space");
    await expect(primaryToggle).toHaveAttribute("aria-expanded", "false");
    await expect(primary.locator("[data-purpose]")).toHaveText("메인 체크아웃 정리");
    await rest(page);
    expect(await rowGeometry(primaryRow, feature, primaryParts)).toEqual(atRest);

    // An unvisited agentless checkout uses the same single-line row height.
    // Opening it draws no collection header: it holds no named group for an
    // ungrouped header to be told apart from, and it has no rows at all
    // (group-tabs D-14).
    await project.locator("[data-empty-worktrees]").click();
    const emptyRow = project.locator("[data-checkout-row]").filter({ has: page.locator('[data-checkout][aria-label^="empty"]') });
    const emptyButton = emptyRow.locator("[data-checkout]");
    await expect(emptyRow.locator("[data-checkout-toggle]")).toHaveCount(0);
    expect((await emptyButton.boundingBox())!.height).toBe(await emptyButton.evaluate((node) => parseFloat(getComputedStyle(node).getPropertyValue("--size-checkout-row"))));
    await emptyButton.click();
    await expect(emptyButton).toHaveAttribute("aria-current", "true");
    const emptyId = await emptyButton.getAttribute("data-checkout");
    await expect.poll(() => last.get("focus_checkout")?.checkout_id).toBe(emptyId);
    await expect(emptyRow.locator("[data-checkout-toggle]")).toHaveAttribute("aria-expanded", "true");
    await expect(emptyRow.locator("[data-sidebar-named-group]")).toHaveCount(0);
    await expect(emptyRow.getByRole("button", { name: "Ungrouped", exact: true })).toHaveCount(0);
    await expect(emptyRow.locator("[data-pane]")).toHaveCount(0);

    // The chevron opens the agent rows below the row: their own marks stand
    // for the checkout's badge, which goes, and the row becomes one line with
    // its age, its purpose waiting for the fold (agent-hierarchy-screens B35).
    const featureButton = feature.locator("[data-checkout]");
    await featureToggle.click();
    await expect(featureToggle).toHaveAttribute("aria-expanded", "true");
    await expect(feature.locator(`[data-checkout-agents-open] [data-pane="${rowsPane}"]`)).toBeVisible();
    await expect(feature.locator("[data-checkout-status]")).toHaveCount(0);
    await expect(feature.locator("[data-purpose]")).toHaveCount(0);
    await expect(feature.locator("[data-checkout-age]")).toBeVisible();
    // Indent D (sidebar-quiet-mark-indent B10-B12): a project with checkouts
    // draws no glyph, so its name and its checkouts' glyphs share one column;
    // a checkout's name starts the next, and while the checkout holds no named
    // group its agents' marks stand on that name column too, with no collection
    // header and no collection indent of their own (group-tabs D-14).
    // A title still follows its mark by the mark and one gap.
    await expect(feature.locator("[data-sidebar-named-group]")).toHaveCount(0);
    await expect(project.locator("[data-project-row] svg")).toHaveCount(0);
    const left = async (part: Locator) => (await part.boundingBox())!.x;
    const projectName = await left(project.locator("[data-project-row] [data-row-name]"));
    const checkoutGlyph = await left(feature.locator("[data-checkout-glyph]"));
    const checkoutName = await left(feature.locator("[data-checkout-menu] [data-row-name]"));
    const agentMark = await left(feature.locator(`[data-pane="${rowsPane}"] [data-agent-status-mark]`));
    const agentTitle = await left(feature.locator(`[data-pane="${rowsPane}"] [data-agent-title]`));
    expect([checkoutGlyph - projectName, checkoutName - checkoutGlyph, agentMark - checkoutName, agentTitle - agentMark]).toEqual([0, 18, 0, 16]);
    expect((await featureButton.boundingBox())!.height).toBe(await featureButton.evaluate((node) => parseFloat(getComputedStyle(node).getPropertyValue("--size-checkout-row"))));
    await screenshot(page, "projects-sidebar-open");

    // The row itself still opens the checkout.
    await feature.locator("[data-checkout]").click();
    await expect(feature.locator("[data-checkout]")).toHaveAttribute("aria-current", "true");

    // A plain folder is one row: no project row or fold of its own, a folder
    // glyph, and no agent counts, which its agent rows say once its chevron opens them.
    const folder = page.locator("[data-project]", { hasText: /^notes/ });
    await expect(folder.locator("[data-project-row]")).toHaveCount(0);
    await expect(folder.locator("[data-project-toggle]")).toHaveCount(0);
    const folderRow = folder.locator("[data-checkout]");
    await expect(folderRow).toHaveCount(1);
    await expect(folderRow).toHaveAttribute("data-checkout-kind", "folder");
    await expect(folderRow.locator("xpath=..").locator("[data-badge-part]")).toHaveCount(0);
    const folderToggle = folder.locator("[data-checkout-toggle]");
    await expect(folderToggle).toHaveAttribute("aria-expanded", "true");
    await folderToggle.click();
    await expect(folderToggle).toHaveAttribute("aria-expanded", "false");
    // The row opens the folder's checkout, not an Overview.
    await folderRow.click();
    await expect(folderRow).toHaveAttribute("aria-current", "true");
    await expect(page.locator("[data-workspace-screen]")).toBeVisible();
    await expect(feature.locator("[data-checkout]")).not.toHaveAttribute("aria-current", "true");
    await expect(folder.locator("[data-project-overview]")).toHaveCount(0);
    await expect(folderToggle).toHaveAttribute("aria-expanded", "true");
    await expect(featureToggle).toHaveAttribute("aria-expanded", "true");
    await expect(folder.locator(`[data-checkout-agents-open] [data-pane="${notesPane}"]`)).toBeVisible();
    // Prompt elapsed B1: exact native request ages, independent of state and daemon age.
    await page.clock.setFixedTime(promptNow);
    await expect(folder.locator(`[data-pane="${notesPane}"] [data-agent-elapsed]`)).toHaveText("2d");
    await expect(feature.locator(`[data-pane="${rowsPane}"] [data-agent-elapsed]`)).toHaveText("3h");
    await page.clock.setSystemTime(Date.now());
    await folderToggle.click();
    await expect(folder.locator("[data-checkout-agents-open]")).toHaveCount(0);

    // Delegated children leave the checkout tree they work in and open under
    // their root, one line with no branch (agent-hierarchy-screens B10, B14).
    declareParent(herdr, rowsPane, mainPane);
    await primaryToggle.click();
    const parentRow = primary.locator(`[data-checkout-agents-open] [data-pane="${mainPane}"]`);
    const chevron = parentRow.locator(`[data-tree-chevron="${mainPane}"]`);
    await expect(chevron).toBeVisible({ timeout: 20_000 });
    await expect(chevron).toHaveAttribute("aria-expanded", "false");
    await expect(page.locator(`nav[data-sidebar] [data-pane="${rowsPane}"]`)).toHaveCount(0);
    const beforeTree = new Map(sent);
    await chevron.click();
    const childRow = primary.locator(`[data-checkout-agents-open] [data-pane="${rowsPane}"]`);
    await expect(childRow).toHaveAttribute("data-depth", "1");
    await expect(childRow.locator("[data-branch-chip]")).toHaveCount(0);
    await screenshot(page, "projects-sidebar-child-tree");
    await chevron.click();
    await expect(childRow).toHaveCount(0);
    expect([...sent].filter(([kind, count]) => count !== (beforeTree.get(kind) ?? 0)).map(([kind]) => kind)).toEqual(["agent_tree_toggle"]);
    expect(await sidebarColumns(page)).toEqual({ times: [expect.any(Number)], chevrons: [expect.any(Number)] });
    const screenBefore = await page.locator("[data-workspace-screen]").count();
    const quiet = new Map(sent);
    await primaryToggle.click();
    await primaryToggle.click();
    await projectToggle.click();
    await projectToggle.click();
    await expect(projectToggle).toHaveAttribute("aria-expanded", "true");
    const moved = [...sent].filter(([kind, count]) => count !== (quiet.get(kind) ?? 0)).map(([kind]) => kind);
    expect(moved.filter((kind) => !["checkout_agents_toggle", "project_checkouts_fold", "ui_state_update", "ui_state_update.usage_hint"].includes(kind))).toEqual([]);
    expect(await page.locator("[data-workspace-screen]").count()).toBe(screenBefore);
    await expect(folderRow).toHaveAttribute("aria-current", "true");

    // Reopen this project on its parent, so the child-only checkout stays in the background.
    await primary.locator("[data-checkout]").click();
    // Folding the project hides its checkouts; both folds survive a reload.
    await projectToggle.click();
    await expect(projectToggle).toHaveAttribute("aria-expanded", "false");
    await expect(project.locator("[data-checkout]")).toHaveCount(0);
    await expect(project.locator("[data-project-overview]")).toHaveCount(0);
    await open(page, daemon);
    await expect(projectToggle).toHaveAttribute("aria-expanded", "false");
    await expect(project.locator("[data-checkout]")).toHaveCount(0);
    // One project-row event returns to the last checkout and carries its fold intent.
    const projectRow = project.locator("[data-project-row]");
    const beforeRow = new Map(sent);
    await projectRow.click();
    await expect(page.locator("[data-workspace-screen]")).toBeVisible();
    await expect(projectToggle).toHaveAttribute("aria-expanded", "true");
    expect((sent.get("focus_checkout") ?? 0) - (beforeRow.get("focus_checkout") ?? 0)).toBe(1);
    expect(last.get("focus_checkout")).toMatchObject({ workspace_id: await project.getAttribute("data-project"), project_expanded: true });
    expect(sent.get("project_checkouts_fold") ?? 0).toBe(beforeRow.get("project_checkouts_fold") ?? 0);
    await projectRow.click();
    await expect(projectToggle).toHaveAttribute("aria-expanded", "false");
    await expect(project.locator("[data-checkout]")).toHaveCount(0);
    await expect(page.locator("[data-workspace-screen]")).toBeVisible();
    await projectRow.click();
    // B1: this checkout now contains only the delegated child, so it has no
    // line of its own; the child is reached from its root's tree.
    await expect(feature).toHaveCount(0);
    await expect(primaryToggle).toHaveAttribute("aria-expanded", "true");
    await expect(parentRow.locator(`[data-tree-chevron="${mainPane}"]`)).toBeVisible();
    await expect(page.locator(`nav[data-sidebar] [data-pane="${rowsPane}"]`)).toHaveCount(0);
    // B25: every level open, at the design width and the app's minimum, nothing runs sideways.
    for (const width of ["calc(240px + var(--size-rail))", "calc(var(--size-sidebar-min) + var(--size-rail))"]) expect(await sidebarOverflow(page, width)).toEqual([]);
    await sidebarOverflow(page, "");

    // B4 in Agents: a quiet row and the row after it stay put through hover,
    // keyboard focus and selection.
    await folderToggle.click();
    await expect(folderToggle).toHaveAttribute("aria-expanded", "true");
    const quietRows = page.locator("nav[data-sidebar] li[data-pane]:not(:has([data-agent-line])):has([data-agent-elapsed])");
    const [agentRow, nextRow] = [quietRows.nth(0), quietRows.nth(1)];
    const agentParts = [agentRow.locator("[data-agent-title]"), agentRow.locator("[data-agent-elapsed]")];
    await rest(page);
    const agentAtRest = await rowGeometry(agentRow, nextRow, agentParts);
    await agentRow.hover();
    expect(await rowGeometry(agentRow, nextRow, agentParts)).toEqual(agentAtRest);
    await keyboardFocus(page, agentRow.locator("[data-agent-open]"));
    expect(await rowGeometry(agentRow, nextRow, agentParts)).toEqual(agentAtRest);
    await agentRow.locator("[data-agent-open]").click();
    await expect(agentRow.locator("[data-agent-open]")).toHaveAttribute("aria-current", "true");
    await rest(page);
    expect(await rowGeometry(agentRow, nextRow, agentParts)).toEqual(agentAtRest);
    await folderToggle.click();
    await expect(folderToggle).toHaveAttribute("aria-expanded", "false");

    for (const theme of ["light", "dark"] as const) {
      await chooseTheme(page, theme);
      // The Settings shortcut leaves keyboard focus on the last clicked control; its ring is not part of the rows.
      await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
      await page.locator("[data-project-list]").hover({ position: { x: 1, y: 1 } });
      await screenshot(page, `projects-sidebar-${theme}`);
    }

    // The sidebar follows Interface text with the rest of the interface: at
    // the largest size its text grows as the text outside it does, every row
    // still holds its text, and hover still moves nothing.
    await rest(page);
    const outsideAtDefault = await probeTextSize(page, "main");
    const sidebarAtDefault = await probeTextSize(page, "nav[data-sidebar]");
    await page.keyboard.press(chord("settings"));
    await page.locator('[data-settings-tab="general"]').click();
    const fontSize = page.locator('[data-font-size="true"] [role="slider"]');
    await fontSize.focus();
    await page.keyboard.press("End");
    await expect(fontSize).toHaveAttribute("aria-valuenow", "17");
    await page.keyboard.press("Escape");
    await expect(page.locator('[data-settings="true"]')).toHaveCount(0);
    // The scale is already set at the default size, so wait for its effect, not its presence.
    await expect.poll(() => probeTextSize(page, "main")).toBeGreaterThan(outsideAtDefault);
    await rest(page);
    expect(await probeTextSize(page, "nav[data-sidebar]")).toBeGreaterThan(sidebarAtDefault);
    const projectsAtLarge = await rowGeometry(primaryRow, parentRow, primaryParts);
    expect(await sidebarRowsFit(page)).toEqual([]);
    expect(await projectButtonFills(page)).toBe(true);
    await primaryRow.hover();
    expect(await rowGeometry(primaryRow, parentRow, primaryParts)).toEqual(projectsAtLarge);
    await rest(page);
    await keyboardFocus(page, primary.locator("[data-checkout]"));
    expect(await rowGeometry(primaryRow, parentRow, primaryParts)).toEqual(projectsAtLarge);
    await rest(page);
    await screenshot(page, "projects-sidebar-large-font");
    await rest(page);
    const agentsAtLarge = await rowGeometry(agentRow, nextRow, agentParts);
    expect(await sidebarRowsFit(page)).toEqual([]);
    await agentRow.hover();
    expect(await rowGeometry(agentRow, nextRow, agentParts)).toEqual(agentsAtLarge);
    await rest(page);
    await screenshot(page, "agents-sidebar-large-font");

    // B3: on an input with no hover, the controls that wait for the pointer are always shown.
    const cdp = await page.context().newCDPSession(page);
    await cdp.send("Emulation.setTouchEmulationEnabled", { enabled: true, maxTouchPoints: 1 });
    await open(page, daemon);
    expect(await page.evaluate(() => matchMedia("(hover: none)").matches)).toBe(true);
    await rest(page);
    await expect(projectToggle).toHaveAttribute("aria-expanded", "true");
    await expect(projectToggle).toHaveCSS("opacity", "1");
    await expect(primaryToggle).toHaveCSS("opacity", "1");
    await cdp.send("Emulation.setTouchEmulationEnabled", { enabled: false });

    // B20: before the first snapshot the list says it is connecting, not that it is empty.
    await page.goto("about:blank");
    await page.goto(`${daemon.origin}/#token=refused`);
    await expect(page.locator("[data-sidebar-loading]")).toBeVisible({ timeout: 20_000 });
    await expect(page.locator("[data-projects-empty]")).toHaveCount(0);
  } finally {
    await daemon?.stop();
    await herdr.stop();
  }
});

test("Needs You and Unread are raised above Pinned and stay in their tree, whatever is folded", async ({ page }) => {
  await page.setViewportSize({ width: 1400, height: 900 });
  const herdr = await startHerdr();
  let daemon: Daemon | null = null;
  try {
    const repo = path.join(herdr.root, "raised");
    fs.mkdirSync(repo);
    git(repo, ["init"]);
    git(repo, ["commit", "--allow-empty", "-m", "initial"]);
    const asking = await workspaceAt(herdr, repo, "배포 전 확인 요청");
    const [finished] = herdr.panes;
    daemon = await startHided(herdr, "projects-raised");
    const sent = countSent(page);
    await open(page, daemon);

    const project = page.locator("[data-project]").filter({ has: page.locator("[data-project-row]", { hasText: /^raised/ }) });
    await project.locator("[data-project-row]").click({ button: "right" });
    await page.getByRole("menuitem", { name: "Pin", exact: true }).click();
    const pinned = page.locator('[data-section="pinned"]');
    await expect(pinned).toBeVisible();
    // Nothing needs the operator yet: no raised section is drawn.
    await expect(page.locator("[data-raised-group]")).toHaveCount(0);

    labelAgent(herdr, asking, { task: "배포 전 확인 요청", reply: "배포해도 될까요?", question: true });
    labelAgent(herdr, finished, { task: "Agent one", progress: "정리 끝" });
    await finishFixtureTurn(herdr, finished, elsewhereTab(herdr));
    const needsYou = page.locator('[data-raised-group="needs_you"]');
    const done = page.locator('[data-raised-group="done"]');
    await expect(needsYou.locator(`[data-pane="${asking}"]`)).toBeVisible({ timeout: 20_000 });
    await expect(needsYou.locator('[data-section="needs_you"]')).toHaveText("Needs You · 1");
    await expect(done.locator(`[data-pane="${finished}"]`)).toBeVisible();
    await expect(done.locator('[data-section="done"]')).toHaveText("Unread · 1");
    // Needs You, then Done, then Pinned, at the top of the list; neither is past its cap, so nothing folds.
    const top = async (locator: Locator) => (await locator.boundingBox())!.y;
    expect(await top(needsYou)).toBeLessThan(await top(done));
    expect(await top(done)).toBeLessThan(await top(pinned));
    await expect(page.locator("[data-raised-more]")).toHaveCount(0);
    await expect(page.locator("[data-raised-list] > li").first()).toHaveAttribute("data-raised-group", "needs_you");
    // The raised row names where the agent runs, as the Agents list does.
    await expect(needsYou.locator(`[data-pane="${asking}"] [data-agent-place]`)).toHaveText("raised › main");

    // The agent stays in its checkout tree too.
    const checkoutToggle = project.locator("[data-checkout-toggle]");
    await checkoutToggle.click();
    await expect(project.locator(`[data-checkout-agents-open] [data-pane="${asking}"]`)).toBeVisible();
    await screenshot(page, "projects-raised-open");

    // Folding the checkout and the project hides the tree rows, not the raised ones.
    await checkoutToggle.click();
    await project.locator("[data-project-toggle]").click();
    await expect(project.locator("[data-project-toggle]")).toHaveAttribute("aria-expanded", "false");
    await expect(project.locator(`[data-pane="${asking}"]`)).toHaveCount(0);
    await expect(needsYou.locator(`[data-pane="${asking}"]`)).toBeVisible();
    await screenshot(page, "projects-raised-folded");

    // A raised row opens its agent as the Agents row does: one focus_pane.
    const before = sent.get("focus_pane") ?? 0;
    await needsYou.locator(`[data-pane="${asking}"] [data-agent-open]`).click();
    await expect.poll(() => sent.get("focus_pane") ?? 0).toBe(before + 1);
    await expect(page.locator("[data-workspace-screen]")).toBeVisible();

    // Answered, the agent leaves Needs You and its empty section goes.
    await setFixtureLifecycle(herdr, asking, "working");
    await setFixtureLifecycle(herdr, asking, "idle");
    await expect(needsYou).toHaveCount(0, { timeout: 20_000 });
  } finally {
    await daemon?.stop();
    await herdr.stop();
  }
});

// docs/UI_BEHAVIOR.md, The sidebar list: Needs You and Done stand still above the projects.
test("Done stands still above the scrolled projects, and opening its first row moves the next one up into its place", async ({ page }) => {
  await page.setViewportSize({ width: 1400, height: 560 });
  const herdr = await startHerdr();
  let daemon: Daemon | null = null;
  try {
    const panes: string[] = [];
    for (const name of ["alpha-repo", "bravo-repo", "charlie-repo", "delta-repo", "echo-repo"]) panes.push(await workspaceAt(herdr, repoAt(herdr, name), `${name} 작업`));
    daemon = await startHided(herdr, "projects-raised-held");
    await open(page, daemon);
    const elsewhere = elsewhereTab(herdr);
    await finishFixtureTurn(herdr, panes[3]!, elsewhere);
    await finishFixtureTurn(herdr, panes[4]!, elsewhere);
    const done = page.locator('[data-raised-group="done"]');
    await expect(done.locator('[data-section="done"]')).toHaveText("Unread · 2", { timeout: 20_000 });
    const list = page.locator("[data-project-list]");
    expect(await list.evaluate((element) => element.scrollHeight > element.clientHeight)).toBe(true);

    // The projects scroll under Done, which stays where it stood.
    const top = async (locator: Locator) => Math.round((await locator.boundingBox())!.y);
    const held = await top(done);
    await list.evaluate((element) => element.scrollTo({ top: element.scrollHeight }));
    await expect.poll(() => list.evaluate((element) => element.scrollTop)).toBeGreaterThan(0);
    expect(await top(done)).toBe(held);
    await list.evaluate((element) => element.scrollTo({ top: 0 }));

    // Opening the first Done row reads it: it leaves Done, and the second row stands where the first stood.
    const rows = done.locator("li[data-pane]");
    const first = (await rows.first().getAttribute("data-pane"))!;
    const second = (await rows.nth(1).getAttribute("data-pane"))!;
    const slot = await top(rows.first());
    await rows.first().locator("[data-agent-open]").click();
    await expect(done.locator(`[data-pane="${first}"]`)).toHaveCount(0, { timeout: 20_000 });
    await expect(done.locator('[data-section="done"]')).toHaveText("Unread · 1");
    expect(await top(done)).toBe(held);
    expect(await top(done.locator(`li[data-pane="${second}"]`))).toBe(slot);
    await screenshot(page, "projects-raised-held");
  } finally {
    await daemon?.stop();
    await herdr.stop();
  }
});

/** A git repository at `herdr.root/<name>` with one commit, so the sidebar draws it as a project with its main checkout. */
function repoAt(herdr: HerdrFixture, name: string): string {
  const repo = path.join(herdr.root, name);
  fs.mkdirSync(repo);
  git(repo, ["init"]);
  git(repo, ["commit", "--allow-empty", "-m", "initial"]);
  return repo;
}

/** A project's list item, by the name its row shows. */
function projectItem(page: Page, name: string): Locator {
  return page.locator("nav[data-sidebar] [data-project]").filter({ has: page.locator("[data-row-name]", { hasText: new RegExp(`^${name}$`) }) });
}

// PRD sidebar-titlebar-refresh B1-B3: the fold and the weight are the core's
// decisions, which the list draws and a reload keeps.
test("projects without agents fold under No agents, and only a working agent brightens a project's name", async ({ page }) => {
  await page.setViewportSize({ width: 1400, height: 900 });
  const herdr = await startHerdr();
  let daemon: Daemon | null = null;
  try {
    const quietDir = path.join(herdr.root, "quiet-notes");
    fs.mkdirSync(quietDir);
    const quietPane = await workspaceAt(herdr, quietDir, null);
    const busyPane = await workspaceAt(herdr, repoAt(herdr, "busy-repo"), "빌드 고치기");
    daemon = await startHided(herdr, "projects-no-agents");
    await open(page, daemon);

    // B1: the agentless project is counted in the fold after the projects with agents, and not drawn.
    const busy = projectItem(page, "busy-repo");
    const quiet = projectItem(page, "quiet-notes");
    const fold = page.locator("[data-no-agent-projects]");
    await expect(busy).toBeVisible({ timeout: 20_000 });
    // The project in front is never folded, so the Workspace in front is busy-repo's.
    await busy.locator("[data-checkout]").first().click();
    await expect(page.locator("[data-workspace-screen]")).toBeVisible();
    await expect(fold).toHaveText(/^No agents 1$/);
    await expect(fold).toHaveAttribute("aria-expanded", "false");
    await expect(quiet).toHaveCount(0);
    expect((await fold.boundingBox())!.y).toBeGreaterThan((await busy.boundingBox())!.y);

    // B2: opened, it shows the project as an ordinary row, and the core keeps it open across a reload.
    await fold.click();
    await expect(fold).toHaveAttribute("aria-expanded", "true");
    await expect(quiet).toBeVisible();
    await screenshot(page, "projects-no-agents-open");
    await page.reload();
    await expect(fold).toHaveAttribute("aria-expanded", "true", { timeout: 20_000 });
    await expect(quiet).toBeVisible();

    // B3: an idle agent leaves the project's label muted; a working one
    // brightens it to the ordinary text colour while its one 600 weight stays,
    // because the label is what tells one project's block from the next
    // (group-tabs D-16).
    const toneOf = async (token: string) => page.evaluate((name) => {
      const probe = document.createElement("span");
      probe.style.color = `var(${name})`;
      document.body.append(probe);
      const color = getComputedStyle(probe).color;
      probe.remove();
      return color;
    }, token);
    const weight = busy.locator("[data-project-weight]");
    await expect(weight).toHaveAttribute("data-project-weight", "quiet");
    await expect(weight).toHaveCSS("font-weight", "600");
    await expect(weight).toHaveCSS("color", await toneOf("--muted-foreground"));
    await setFixtureLifecycle(herdr, busyPane, "working");
    await expect(weight).toHaveAttribute("data-project-weight", "working", { timeout: 20_000 });
    await expect(weight).toHaveCSS("color", await toneOf("--foreground"));
    await expect(quiet.locator("[data-project-weight]")).toHaveAttribute("data-project-weight", "quiet");

    // B2: an agent started in the folded project takes it out of the fold, which goes with its last project.
    herdr.run(["agent", "start", "agent-quiet", "--kind", "claude", "--pane", quietPane]);
    await expect(fold).toHaveCount(0, { timeout: 20_000 });
    await expect(quiet).toBeVisible();
  } finally {
    await daemon?.stop();
    await herdr.stop();
  }
});

// PRD sidebar-titlebar-refresh B4, B5: the fold of a sleeping project is the core's, kept across a reload.
test("a project whose agents all sleep folds, and stays open once opened", async ({ page }) => {
  await page.setViewportSize({ width: 1400, height: 900 });
  const herdr = await startHerdr();
  let daemon: Daemon | null = null;
  try {
    const napPane = await workspaceAt(herdr, repoAt(herdr, "napping-repo"), "잠들 작업");
    daemon = await startHided(herdr, "projects-asleep");
    await open(page, daemon);
    const project = projectItem(page, "napping-repo");
    const toggle = project.locator("[data-project-toggle]");
    await expect(project.locator(`[data-pane="${napPane}"]`)).toBeVisible({ timeout: 20_000 });

    await sendEvent(page, daemon, "agent_sleep", { pane_id: napPane });
    // B4: folded, with no agent row, and no moon or counts on the project row.
    await expect(toggle).toHaveAttribute("aria-expanded", "false", { timeout: 30_000 });
    await expect(project.locator("[data-project-row]").locator("xpath=..").locator("[data-badge-part]")).toHaveCount(0);
    await expect(project.locator(`[data-pane="${napPane}"]`)).toHaveCount(0);
    await expect(project.locator("[data-project-weight]")).toHaveAttribute("data-project-weight", "quiet");
    await screenshot(page, "projects-asleep-folded");

    // B4, B5: opened, the checkout and its sleeping row show, and the opening survives a reload.
    await toggle.click();
    await expect(toggle).toHaveAttribute("aria-expanded", "true");
    await expect(project.locator(`[data-pane="${napPane}"] [data-agent-status="sleeping"]`)).toBeVisible();
    await page.reload();
    await expect(project.locator("[data-project-toggle]")).toHaveAttribute("aria-expanded", "true", { timeout: 20_000 });
  } finally {
    await daemon?.stop();
    await herdr.stop();
  }
});

// PRD sidebar-titlebar-refresh B6-B8.
test("a project row's + and ⋯ appear on hover and focus, start in its main checkout and open its menu", async ({ page }) => {
  await page.setViewportSize({ width: 1400, height: 900 });
  const herdr = await startHerdr();
  let daemon: Daemon | null = null;
  try {
    await workspaceAt(herdr, repoAt(herdr, "tools-repo"), "도구 정리");
    daemon = await startHided(herdr, "projects-row-actions");
    await open(page, daemon);
    const project = projectItem(page, "tools-repo");
    const row = project.locator("[data-project-row]");
    const start = project.locator("[data-row-start]");
    const more = project.locator("[data-row-more]");
    const pair = project.locator("[data-row-actions]");
    await expect(row).toBeVisible({ timeout: 20_000 });
    await rest(page);
    const name = row.locator("[data-row-name]");
    const nameWidth = (await name.boundingBox())!.width;
    await expect(pair).toHaveCSS("opacity", "0");

    // B6: the pointer shows the pair and the name keeps its width.
    await row.hover();
    await expect(pair).toHaveCSS("opacity", "1");
    expect((await name.boundingBox())!.width).toBe(nameWidth);
    await screenshot(page, "projects-row-actions-hover");
    await rest(page);
    await expect(pair).toHaveCSS("opacity", "0");

    // B8: each has its own name, and the keyboard reaches and presses them.
    await expect(start).toHaveAccessibleName("Start agent in tools-repo");
    await expect(more).toHaveAccessibleName("More actions for tools-repo");
    await keyboardFocus(page, start);
    await expect(pair).toHaveCSS("opacity", "1");
    // B7: + aims the start panel at the project's main checkout.
    await page.keyboard.press("Enter");
    const panel = page.locator("[data-start-panel]");
    await expect(panel.locator("[data-start-target]")).toHaveAttribute("data-start-target", new RegExp(`^checkout:${daemon.node}:.*/tools-repo$`));
    await page.keyboard.press("Escape");
    await expect(panel).toHaveCount(0);

    // B8: ⋯ opens the right-click menu under itself.
    await row.click({ button: "right" });
    const contextItems = await page.locator('[role="menu"] [data-menu-item]').evaluateAll((items) => items.map((item) => item.getAttribute("data-menu-item")));
    await page.keyboard.press("Escape");
    await keyboardFocus(page, more);
    await page.keyboard.press("Space");
    const menu = page.locator('[role="menu"]');
    await expect(menu).toBeVisible();
    expect(await menu.locator("[data-menu-item]").evaluateAll((items) => items.map((item) => item.getAttribute("data-menu-item")))).toEqual(contextItems);
    const button = (await more.boundingBox())!;
    expect((await menu.boundingBox())!.y).toBeGreaterThanOrEqual(button.y + button.height - 1);
    await expect(pair).toHaveCSS("opacity", "1");
    await screenshot(page, "projects-row-actions-menu");
    await page.keyboard.press("Escape");
    await expect(menu).toHaveCount(0);
  } finally {
    await daemon?.stop();
    await herdr.stop();
  }
});

// PRD sidebar-titlebar-refresh B9, B10.
test("section headers stay at the top of the scrolled list, and a row the keyboard reaches is not under one", async ({ page }) => {
  await page.setViewportSize({ width: 1400, height: 560 });
  const herdr = await startHerdr();
  let daemon: Daemon | null = null;
  try {
    const names = ["alpha-repo", "bravo-repo", "charlie-repo", "delta-repo", "echo-repo"];
    for (const name of names) await workspaceAt(herdr, repoAt(herdr, name), `${name} 작업`);
    daemon = await startHided(herdr, "projects-sticky");
    await open(page, daemon);
    await expect(projectItem(page, "echo-repo")).toBeVisible({ timeout: 20_000 });
    await projectItem(page, "alpha-repo").locator("[data-project-row]").click({ button: "right" });
    await page.getByRole("menuitem", { name: "Pin", exact: true }).click();
    const pinned = page.locator('[data-section="pinned"]');
    const recent = page.locator('[data-section="recent"]');
    // B9: the Pinned header carries the pin, and the pinned row none.
    await expect(pinned.locator("[data-section-pin]")).toBeVisible();
    await expect(projectItem(page, "alpha-repo").locator("svg.lucide-pin")).toHaveCount(0);

    // B9: scrolled to the end, the Projects header is stuck at the list's top with its hairline, the Pinned one pushed off.
    const list = page.locator("[data-project-list]");
    await list.evaluate((element) => element.scrollTo({ top: element.scrollHeight }));
    await expect(recent).toHaveAttribute("data-stuck", "true");
    const listTop = (await list.boundingBox())!.y;
    const header = (await recent.boundingBox())!;
    expect(Math.round(header.y)).toBe(Math.round(listTop));
    await expect(recent).toHaveCSS("border-bottom-width", "1px");
    expect((await pinned.boundingBox())!.y + (await pinned.boundingBox())!.height).toBeLessThanOrEqual(listTop + 0.5);
    await screenshot(page, "projects-sticky-header");

    // B10: the keyboard brings the first project under the header into view below it, whole.
    const first = recent.locator("xpath=..").locator("[data-project-row]").first();
    await keyboardFocus(page, first);
    const row = (await first.boundingBox())!;
    const stuck = (await recent.boundingBox())!;
    expect(row.y).toBeGreaterThanOrEqual(stuck.y + stuck.height - 0.5);

    // Keep the cached paint marker stale while a real core focus change arrives.
    // The first agent fits in the viewport but is covered by the header.
    const target = recent.locator("xpath=..").locator("[data-agent-open]").first();
    await target.evaluate((element) => {
      const list = element.closest<HTMLElement>("[data-project-list]")!;
      list.scrollTop += element.getBoundingClientRect().top - list.getBoundingClientRect().top - 4;
      const headers = [...list.querySelectorAll<HTMLElement>("[data-section-sticky]")];
      const stale = () => { for (const header of headers) if (header.dataset.stuck !== "false") header.dataset.stuck = "false"; };
      stale();
      const observer = new MutationObserver(stale);
      for (const header of headers) observer.observe(header, { attributes: true, attributeFilter: ["data-stuck"] });
      // Fixture ends with this page; keep other animation frames and core deltas live.
      window.addEventListener("pagehide", () => observer.disconnect(), { once: true });
    });
    await expect(recent).toHaveAttribute("data-stuck", "false");
    await expect(target).not.toHaveAttribute("aria-current", "true");
    const covered = (await target.boundingBox())!;
    const covering = (await recent.boundingBox())!;
    const viewport = (await list.boundingBox())!;
    expect(covered.y).toBeGreaterThanOrEqual(viewport.y);
    expect(covered.y).toBeLessThan(covering.y + covering.height);
    expect(covered.y + covered.height).toBeLessThanOrEqual(viewport.y + viewport.height);
    await target.dispatchEvent("click");
    await expect(target).toHaveAttribute("aria-current", "true");
    expect((await target.boundingBox())!.y).toBeGreaterThanOrEqual((await recent.boundingBox())!.y + (await recent.boundingBox())!.height - 0.5);
    await expect(recent).toHaveAttribute("data-stuck", "false");
  } finally {
    await daemon?.stop();
    await herdr.stop();
  }
});
