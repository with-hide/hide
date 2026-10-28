// The sidebar's row menus (PRD sidebar-context-menus D-11) on an isolated
// pinned Herdr and hided: a Git project Herdr shows without a registration,
// with an agent in its main checkout and one in a linked worktree. The
// project row's menu lists the board's items and Pin registers the row and
// pins it; New tab in main opens a terminal in the default checkout and brings
// it to the front, where an explicit start creates its terminal; Set as default
// checkout moves the home glyph and the first place to the worktree; the agent row's Copy session id puts Herdr's
// session id on the clipboard, and its Close tab… closes that agent's tab.
// Each contract is its own spec on its own fixture. Light and Dark captures of
// the menus land in HIDE_E2E_SCREENSHOT_DIR.

import { expect, test, type Locator, type Page } from "@playwright/test";
import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { claudeProjects, labelAgent, setFixtureSession, writeFixtureTranscript, startHerdr, type HerdrFixture } from "./herdr-fixture";
import { waitForTerminal } from "./group-start-helper";
import { herdrTabs } from "./failure-dump";
import { startHided } from "./hided-fixture";
import { rest, screenshot } from "./wire";
import { chord, commandLabel } from "./chords";

test.describe.configure({ timeout: 180_000 });

const BRANCH = "feature/menus";
const SESSION = "5b0c7e2a-menu-e2e-session";

function git(cwd: string, args: string[]): void {
  execFileSync("git", ["-c", "user.name=e2e", "-c", "user.email=e2e@example.invalid", "-c", "init.defaultBranch=main", "-c", "commit.gpgsign=false", ...args], { cwd, stdio: "ignore" });
}

async function prompt(herdr: HerdrFixture, pane: string): Promise<void> {
  await expect.poll(() => {
    const read = spawnSync(herdr.bin, ["pane", "read", pane, "--source", "visible", "--format", "text"], { env: herdr.env, encoding: "utf8", timeout: 10_000 });
    return read.status === 0 && read.stdout.includes("fixture %");
  }, { timeout: 15_000, message: `no prompt in pane ${pane}` }).toBe(true);
}

/** A Herdr workspace at `cwd` with a fake `claude` agent titled `task`; returns its pane. */
async function workspaceAt(herdr: HerdrFixture, cwd: string, task: string): Promise<string> {
  const created = herdr.run(["workspace", "create", "--cwd", cwd, "--label", path.basename(cwd), "--env", `PATH=${herdr.fixturePath}`, "--no-focus"]) as {
    result: { root_pane: { pane_id: string } };
  };
  const pane = created.result.root_pane.pane_id;
  await prompt(herdr, pane);
  herdr.run(["agent", "start", `agent-${path.basename(cwd)}`, "--kind", "claude", "--pane", pane]);
  labelAgent(herdr, pane, { task });
  return pane;
}

/** The open menu as it is drawn: a line per item (its label and chord), "─" per separator, "(disabled)" after a disabled item. */
async function menuLines(menu: Locator): Promise<string[]> {
  return menu.evaluate((element) =>
    Array.from(element.children).filter((child) => !child.hasAttribute("data-menu-title")).map((child) => {
      if (child.getAttribute("role") === "separator") return "─";
      const label = child.querySelector("[data-menu-label]")?.textContent ?? "";
      const chord = child.querySelector("[data-menu-shortcut]")?.textContent;
      return [label, chord, child.hasAttribute("data-disabled") ? "(disabled)" : null].filter(Boolean).join(" ");
    }),
  );
}

async function openMenu(page: Page, row: Locator, name: string): Promise<Locator> {
  await row.click({ button: "right" });
  const menu = page.getByRole("menu", { name });
  await expect(menu).toBeVisible();
  return menu;
}

async function chooseTheme(page: Page, theme: "light" | "dark"): Promise<void> {
  await page.keyboard.press(chord("settings"));
  await expect(page.locator('[data-settings="true"]')).toBeVisible();
  await page.locator('[data-settings-tab="general"]').click();
  await page.locator(`[data-theme-option="${theme}"]`).click();
  await expect(page.locator("html")).toHaveClass(new RegExp(`\\b${theme}\\b`));
  await page.keyboard.press("Escape");
  await expect(page.locator('[data-settings="true"]')).toHaveCount(0);
}

// The fixture every contract below starts from: the project's menu is read
// from its row, the agent's from the Agents list.
async function startMenus(page: Page, options: { pinned?: boolean } = {}) {
  await page.setViewportSize({ width: 1400, height: 900 });
  const herdr = await startHerdr();
  try {
    const repo = path.join(herdr.root, "repo");
    fs.mkdirSync(repo);
    git(repo, ["init"]);
    fs.writeFileSync(path.join(repo, "README.md"), "# repo\n");
    git(repo, ["add", "README.md"]);
    git(repo, ["commit", "-m", "initial"]);
    const worktree = path.join(herdr.root, "repo-menus");
    git(repo, ["worktree", "add", "-b", BRANCH, worktree]);
    await workspaceAt(herdr, repo, "메인 체크아웃 정리");
    const worktreePane = await workspaceAt(herdr, worktree, "사이드바 메뉴 구현");
    // The session the menu copies carries the row's title too.
    writeFixtureTranscript(claudeProjects(herdr), SESSION, { task: "사이드바 메뉴 구현" });
    setFixtureSession(herdr, worktreePane, SESSION);

    // The fixture's folders are in the daemon's home, as an operator's projects
    // are, so a pin registers them and a project outliving its last pane stays
    // listed without asking about a folder outside home (PRD wsl-device B15).
    const daemon = await startHided(herdr, "sidebar-menus", herdr.root);
    await page.context().grantPermissions(["clipboard-read", "clipboard-write"], { origin: daemon.origin });
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await expect(page.locator("[data-main-screen]")).toBeVisible({ timeout: 20_000 });

    const project = page.locator("[data-project]").filter({ has: page.locator("[data-project-row]", { hasText: /^repo/ }) });
    const projectRow = project.locator("[data-project-menu]").first();
    const main = project.locator("[data-checkout-row]").filter({ has: page.locator('[data-checkout][aria-label^="main"]') });
    const feature = project.locator("[data-checkout-row]").filter({ has: page.locator(`[data-checkout][aria-label^="${BRANCH}"]`) });
    await expect(feature).toBeVisible({ timeout: 30_000 });
    const agentRow = page.locator(`nav[data-sidebar] li[data-pane="${worktreePane}"]`);
    if (options.pinned) {
      // Pin registers the row; a checkout row's menu and Set as default are
      // the registered project's, so these contracts start from it.
      const menu = await openMenu(page, projectRow, "repo actions");
      await menu.locator('[data-menu-item="pin"]').click();
      await expect(page.locator('[data-section="pinned"]')).toHaveText(/Pinned · 1/);
    }
    return { herdr, daemon, worktreePane, project, projectRow, main, feature, agentRow, stop: () => { daemon.stop(); herdr.stop(); } };
  } catch (error) {
    herdr.stop();
    throw error;
  }
}

test("the project row's menu lists the board's items, and Pin registers and pins an unregistered project", async ({ page }) => {
  const { projectRow, stop } = await startMenus(page);
  try {
    await expect(page.locator('[data-section="pinned"]')).toHaveCount(0);
    // B1, B3: in the board's order. A browser tab has no OS file manager, so
    // its reveal is absent; a row Herdr shows without a registration still
    // offers both Pin and removal. Project Settings B4 adds the settings
    // entry to this same project menu without removing its existing actions.
    let menu = await openMenu(page, projectRow, "repo actions");
    expect(await menuLines(menu)).toEqual(["Project settings", "New worktree…", `New tab in main ${commandLabel("new_tab")}`, "─", "Copy path", "Issue source", "─", "Pin", "Remove project…"]);
    await screenshot(page, "sidebar-menus-project-dark");
    await menu.locator('[data-menu-item="pin"]').click();
    await expect(menu).toHaveCount(0);
    // Pin registered the row and pinned it: it now stands under Pinned, once.
    await expect(page.locator('[data-section="pinned"]')).toHaveText(/Pinned · 1/);
    await expect(page.locator("[data-project-row]", { hasText: /^repo/ })).toHaveCount(1);
    menu = await openMenu(page, projectRow, "repo actions");
    await expect(menu.locator('[data-menu-item="unpin"]')).toHaveText(/Unpin/);
    await page.keyboard.press("Escape");
  } finally { stop(); }
});

// B7, D-04: the project's issue source is a submenu of the row's menu; the
// choice is the one the old Settings tab sent and the stored value comes back
// as the checked item, by pointer and by keyboard.
test("the project row's Issue source submenu shows the stored choice and changes it", async ({ page }) => {
  // Hold real daemon frames across the reopen: the stored choice must update
  // an already-open menu, regardless of when the acknowledgement arrives.
  const held: (string | Buffer)[] = [];
  let holding = false;
  let deliver: (frame: string | Buffer) => void = () => { throw new Error("socket not connected"); };
  await page.routeWebSocket(/\/ws$/, (socket) => {
    const server = socket.connectToServer();
    deliver = (frame) => socket.send(frame);
    socket.onMessage((frame) => server.send(frame));
    server.onMessage((frame) => { if (holding) held.push(frame); else deliver(frame); });
  });
  const { projectRow, stop } = await startMenus(page);
  try {
    let menu = await openMenu(page, projectRow, "repo actions");
    await menu.locator('[data-menu-item="issue_source"]').click();
    const choices = page.getByRole("menu", { name: "Issue source" });
    await expect(choices).toBeVisible();
    // Automatic is in force until the operator chooses; it names what it resolved to.
    await expect(choices.getByRole("menuitemradio")).toHaveText([/^Automatic \(/, /^GitHub/, /^Local$/]);
    await expect(choices.getByRole("menuitemradio", { name: /^Automatic/ })).toHaveAttribute("aria-checked", "true");
    await screenshot(page, "sidebar-menus-issue-source-dark");
    await choices.getByRole("menuitemradio", { name: /^Local$/ }).click();
    await expect(page.getByRole("menu")).toHaveCount(0);
    // The stored choice comes back checked the next time the menu opens.
    menu = await openMenu(page, projectRow, "repo actions");
    await menu.locator('[data-menu-item="issue_source"]').click();
    await expect(page.getByRole("menu", { name: "Issue source" }).getByRole("menuitemradio", { name: /^Local$/ })).toHaveAttribute("aria-checked", "true");
    await page.keyboard.press("Escape");
    await page.keyboard.press("Escape");
    // The keyboard reaches it too: menu key on the row, ArrowRight into the submenu, Enter on a choice.
    await projectRow.locator("[data-project-row]").focus();
    await page.keyboard.press("Shift+F10");
    const keyed = page.getByRole("menu", { name: "repo actions" });
    await expect(keyed).toBeVisible();
    await keyed.locator('[data-menu-item="issue_source"]').focus();
    await page.keyboard.press("ArrowRight");
    const submenu = page.getByRole("menu", { name: "Issue source" });
    await expect(submenu).toBeVisible();
    await submenu.getByRole("menuitemradio", { name: /^Automatic/ }).focus();
    holding = true;
    await page.keyboard.press("Enter");
    await expect(page.getByRole("menu")).toHaveCount(0);
    menu = await openMenu(page, projectRow, "repo actions");
    await menu.locator('[data-menu-item="issue_source"]').click();
    await expect(page.getByRole("menu", { name: "Issue source" }).getByRole("menuitemradio", { name: /^Local$/ })).toHaveAttribute("aria-checked", "true");
    holding = false;
    for (const frame of held.splice(0)) deliver(frame);
    await expect(page.getByRole("menu", { name: "Issue source" }).getByRole("menuitemradio", { name: /^Automatic/ })).toHaveAttribute("aria-checked", "true");
    await page.keyboard.press("Escape");
  } finally { stop(); }
});

// B7: a plain folder is its own checkout and has one row, built from the
// project's menu and the checkout's; that row's Issue source reads the stored
// choice too, so a choice made there is still checked when the menu reopens.
test("a plain folder's row shows the stored Issue source, not Automatic", async ({ page }) => {
  await page.setViewportSize({ width: 1400, height: 900 });
  const herdr = await startHerdr();
  let daemon: Awaited<ReturnType<typeof startHided>> | null = null;
  try {
    const folder = path.join(herdr.root, "notes");
    fs.mkdirSync(folder);
    fs.writeFileSync(path.join(folder, "todo.txt"), "write\n");
    await workspaceAt(herdr, folder, "메모 정리");
    daemon = await startHided(herdr, "sidebar-menus-folder");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await expect(page.locator("[data-main-screen]")).toBeVisible({ timeout: 20_000 });
    const row = page.locator("[data-project]").filter({ has: page.locator("[data-project-menu]") }).locator("[data-project-menu]").first();
    await expect(row).toBeVisible({ timeout: 30_000 });
    let menu = await openMenu(page, row, "notes actions");
    await menu.locator('[data-menu-item="issue_source"]').click();
    const choices = page.getByRole("menu", { name: "Issue source" });
    await expect(choices.getByRole("menuitemradio")).toHaveText([/^Automatic/, /^Local$/]);
    await expect(choices.getByRole("menuitemradio", { name: /^Automatic/ })).toHaveAttribute("aria-checked", "true");
    await choices.getByRole("menuitemradio", { name: /^Local$/ }).click();
    await expect(page.getByRole("menu")).toHaveCount(0);
    menu = await openMenu(page, row, "notes actions");
    await menu.locator('[data-menu-item="issue_source"]').click();
    await expect(page.getByRole("menu", { name: "Issue source" }).getByRole("menuitemradio", { name: /^Local$/ })).toHaveAttribute("aria-checked", "true");
    await page.keyboard.press("Escape");
    await page.keyboard.press("Escape");
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});

test("a checkout row's menu offers worktree actions, and the default checkout cannot be set again", async ({ page }) => {
  const { main, feature, stop } = await startMenus(page, { pinned: true });
  try {
    // B4: the worktree is not the default yet. The row's kind is known only
    // once the worktree reader has read it, a few seconds after launch, so
    // the menu is read again until it has; reading a menu changes nothing.
    const checkoutLines = async (): Promise<string[]> => {
      const open = await openMenu(page, feature.locator("[data-checkout-menu]"), `${BRANCH} actions`);
      const lines = await menuLines(open);
      await page.keyboard.press("Escape");
      await expect(open).toHaveCount(0);
      return lines;
    };
    await expect
      .poll(checkoutLines, { timeout: 30_000 })
      .toEqual(["Open", `New tab here ${commandLabel("new_tab")}`, "New group…", "─", "Set purpose…", "Set as default checkout", "Copy branch name", "Copy path", "─", "Delete worktree…"]);
    await openMenu(page, feature.locator("[data-checkout-menu]"), `${BRANCH} actions`);
    await screenshot(page, "sidebar-menus-checkout-dark");
    await page.keyboard.press("Escape");
    const menu = await openMenu(page, main.locator("[data-checkout-menu]"), "main actions");
    await expect(menu.locator('[data-menu-item="set_primary"]')).toHaveAttribute("data-disabled", "");
    await expect(menu.locator('[data-menu-item="set_primary"]')).toContainText("Already the default checkout.");
    await page.keyboard.press("Escape");
  } finally { stop(); }
});

test("New tab in main opens a terminal in the default checkout", async ({ page }) => {
  const { herdr, projectRow, main, stop } = await startMenus(page);
  try {
    const nativeTabs = () => (herdrTabs(herdr) as { tabs: string[] }[]).flatMap((workspace) => workspace.tabs).sort();
    const before = nativeTabs();
    // Drafts share the Agent tab kind; native IDs distinguish the live tabs.
    const liveTabs = page.locator('[data-tab-bar] [data-tab-kind="herdr"]:not([data-tab^="draft:"])');
    const menu = await openMenu(page, projectRow, "repo actions");
    await menu.locator('[data-menu-item="new_tab_primary"]').click();
    await expect(page.locator("[data-workspace-screen]")).toBeVisible({ timeout: 20_000 });
    await expect(liveTabs).toHaveCount(2);
    await expect(main.locator("[data-checkout]")).toHaveAttribute("aria-current", "true");
    const started = await waitForTerminal(page);
    await expect(liveTabs).toHaveCount(2, { timeout: 20_000 });
    await expect.poll(nativeTabs).toEqual([...before, started].sort());
    await expect(main.locator("[data-checkout]")).toHaveAttribute("aria-current", "true");
  } finally { stop(); }
});

test("Set as default checkout moves the home glyph and the first place to the worktree", async ({ page }) => {
  const { project, main, feature, stop } = await startMenus(page, { pinned: true });
  try {
    // B6
    await expect(main.locator("[data-checkout]")).toHaveAttribute("data-checkout-kind", "primary");
    const menu = await openMenu(page, feature.locator("[data-checkout-menu]"), `${BRANCH} actions`);
    await menu.locator('[data-menu-item="set_primary"]').click();
    await expect(feature.locator("[data-checkout]")).toHaveAttribute("data-checkout-kind", "primary");
    await expect(main.locator("[data-checkout]")).toHaveAttribute("data-checkout-kind", "branch");
    await expect(project.locator("[data-checkout-row]").first()).toHaveAttribute("data-checkout-row", await feature.getAttribute("data-checkout-row") ?? "");
  } finally { stop(); }
});

test("the agent row's menu copies Herdr's session id and the agent's reference with the pane's Herdr id", async ({ page }) => {
  const { agentRow, worktreePane, stop } = await startMenus(page);
  try {
    // B7, B8
    await expect(agentRow).toBeVisible();
    let menu = await openMenu(page, agentRow, "사이드바 메뉴 구현 actions");
    expect(await menuLines(menu)).toEqual(["Show", "─", "Copy name and ID", "Copy session id", "─", "Close tab…"]);
    await screenshot(page, "sidebar-menus-agent-dark");
    await page.locator('[data-menu-item="copy_session_id"]').click();
    await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe(SESSION);
    // The id `herdr pane list` names this pane by, the one ⌘K finds it by, inside the reference an agent reads.
    menu = await openMenu(page, agentRow, "사이드바 메뉴 구현 actions");
    await page.locator('[data-menu-item="copy_reference"]').click();
    await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toMatch(new RegExp(`^@.+ \\(pane ${worktreePane} · 사이드바 메뉴 구현\\)$`));
  } finally { stop(); }
});

test("the agent row's Close tab… closes that agent's tab without asking when it is idle", async ({ page }) => {
  const { agentRow, stop } = await startMenus(page);
  try {
    // B8: through the tab close flow, in a checkout that is not the one in
    // front. An idle agent's tab closes without asking (close.ts asks only
    // for working or attention panes), and the row leaves with its pane.
    const mainTabs = page.locator(`[data-tab-bar] [data-tab-kind="herdr"]`);
    await expect(agentRow).toBeVisible();
    const menu = await openMenu(page, agentRow, "사이드바 메뉴 구현 actions");
    await menu.locator('[data-menu-item="close_tab"]').click();
    await expect(agentRow).toHaveCount(0, { timeout: 20_000 });
    await expect(page.locator('[data-confirm-close="tab"]')).toHaveCount(0);
    await expect(mainTabs).toHaveCount(await mainTabs.count());
  } finally { stop(); }
});

test("the project, checkout and agent menus draw in the light theme", async ({ page }) => {
  const { projectRow, main, agentRow, stop } = await startMenus(page);
  try {
    await expect(agentRow).toBeVisible();
    await chooseTheme(page, "light");
    await rest(page);
    await openMenu(page, agentRow, "사이드바 메뉴 구현 actions");
    await screenshot(page, "sidebar-menus-agent-light");
    await page.keyboard.press("Escape");
    await openMenu(page, projectRow, "repo actions");
    await screenshot(page, "sidebar-menus-project-light");
    await page.keyboard.press("Escape");
    await openMenu(page, main.locator("[data-checkout-menu]"), "main actions");
    await screenshot(page, "sidebar-menus-checkout-light");
    await page.keyboard.press("Escape");
  } finally { stop(); }
});
