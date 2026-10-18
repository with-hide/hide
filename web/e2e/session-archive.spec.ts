// 세션 보관 on a real isolated Herdr and daemon (PRD session-archive): a
// session with a working child asks first, archives into 보관됨 with every
// pane closed, opens its record, resumes the tree, and deletes from the archive.
import { expect, test, type Page } from "@playwright/test";
import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { declareParent, herdrHasFocus, labelAgent, setFixtureLifecycle, setFixtureSession, spawnAgent, startHerdr, type HerdrFixture } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { openTaskFolds, screenshot, showTasks } from "./wire";

test.describe.configure({ timeout: 240_000 });
test.use({ actionTimeout: 15_000 });

type ProcessInfo = { result: { process_info: { foreground_processes: { argv?: string[] | null }[] } } };

function git(cwd: string, args: string[]): void {
  execFileSync("git", ["-c", "user.name=e2e", "-c", "user.email=e2e@example.invalid", "-c", "init.defaultBranch=main", "-c", "commit.gpgsign=false", ...args], { cwd, stdio: "ignore" });
}

const panes = (herdr: HerdrFixture) => (herdr.run(["pane", "list"]) as { result: { panes: { pane_id: string }[] } }).result.panes.map((pane) => pane.pane_id);

/** The pane whose agent Herdr started with `claude --resume <session>`, or null. */
function resumedIn(herdr: HerdrFixture, session: string): string | null {
  for (const pane of panes(herdr)) {
    // A pane listed a moment ago can be gone now: opening a checkout replaces its first shell.
    const read = spawnSync(herdr.bin, ["pane", "process-info", "--pane", pane], { env: herdr.env, encoding: "utf8", timeout: 10_000 });
    if (read.status !== 0) continue;
    const info = JSON.parse(read.stdout) as ProcessInfo;
    if (info.result.process_info.foreground_processes.some((process) => process.argv?.join(" ") === `claude --resume ${session}`)) return pane;
  }
  return null;
}

/** Waits for the resume of `session`, then reports it from that pane as Claude Code's session hook does. */
async function confirmResume(herdr: HerdrFixture, session: string): Promise<string> {
  let pane: string | null = null;
  await expect.poll(() => (pane = resumedIn(herdr, session)), { timeout: 60_000 }).not.toBeNull();
  setFixtureSession(herdr, pane!, session);
  return pane!;
}

const archive = (daemon: Daemon) => () => Object.keys(JSON.parse(fs.readFileSync(path.join(daemon.stateDir, "core-state.json"), "utf8")).session_archive ?? {}).length;

async function openArchivedFold(page: Page) {
  const fold = page.locator('[data-task-fold="archived"] > button[data-task-focus="fold"]');
  await expect(fold).toBeVisible({ timeout: 30_000 });
  if ((await fold.getAttribute("aria-expanded")) === "false") await fold.click();
}

test("세션 보관 asks over running work, closes the tree into 보관됨, resumes it and deletes it from the archive", async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 1000 });
  const herdr = await startHerdr();
  let daemon: Daemon | null = null;
  try {
    const repo = path.join(herdr.root, "repo");
    fs.mkdirSync(repo);
    git(repo, ["init"]);
    git(repo, ["commit", "--allow-empty", "-m", "initial"]);
    const root = await spawnAgent(herdr, "archive-root", null, repo);
    const rootSession = labelAgent(herdr, root, { task: "보관할 세션의 루트 작업" });
    const child = await spawnAgent(herdr, "archive-child", null, path.join(herdr.root, "child"));
    const childSession = labelAgent(herdr, child, { task: "루트가 맡긴 하위 작업" });
    declareParent(herdr, child, root);
    await setFixtureLifecycle(herdr, child, "working");
    daemon = await startHided(herdr, "session-archive");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    const tasks = await showTasks(page);
    const row = tasks.locator(`[data-task-row="${root}"]`);
    await expect(row).toContainText("보관할 세션의 루트 작업", { timeout: 30_000 });

    // B1, B4: a right-click on the task offers 세션 보관; the child's pane menu draws it disabled with why.
    await row.click({ button: "right" });
    const item = page.locator(`[data-task-context="${root}"] [data-menu-item="archive"]`);
    await expect(item).toBeEnabled({ timeout: 20_000 });
    await screenshot(page, "session-archive-menu");
    await item.click();

    // B6: work running asks first, listing what works, neither button holding the keyboard.
    const sheet = page.locator(`[data-confirm-archive="${root}"]`);
    await expect(sheet.locator(`[data-archive-working-row="${child}"]`)).toBeVisible();
    await expect(sheet.locator("[data-archive-confirm]")).not.toBeFocused();
    await screenshot(page, "session-archive-confirm");
    await sheet.locator("[data-archive-confirm]").click();

    // B8, B14: every pane closes, the task leaves the open groups for 보관됨, and the archive is saved.
    await expect.poll(() => panes(herdr).filter((pane) => pane === root || pane === child), { timeout: 60_000 }).toEqual([]);
    await expect(row).toHaveCount(0);
    await expect.poll(archive(daemon)).toBe(1);
    await openArchivedFold(page);
    const archived = tasks.locator("[data-archived-row]");
    await expect(archived).toHaveAttribute("data-archived-phase", "archived");
    await expect(archived).toContainText("보관할 세션의 루트 작업");
    await screenshot(page, "session-archive-fold");

    // B19: the archived record reads 보관됨 with 이어가기 as its one button.
    await archived.locator('[data-task-focus="row"]').click();
    const now = page.locator('[data-record-now="archived"]');
    await expect(now).toBeVisible({ timeout: 30_000 });
    await expect(page.locator(`[data-record-child="${childSession}"] [data-record-child-state]`)).toHaveText("Archived");
    await screenshot(page, "session-archive-record");

    // B21, B22: 이어가기 resumes the root, then its child, and the row leaves 보관됨 once both are back.
    await now.locator('[data-record-now-button="resume"]').click();
    const resumedRoot = await confirmResume(herdr, rootSession);
    const resumedChild = await confirmResume(herdr, childSession);
    expect(resumedChild).not.toBe(resumedRoot);
    // Only the root takes the screen; its child opens behind it.
    expect(herdrHasFocus(herdr, resumedRoot)).toBe(true);
    await expect.poll(archive(daemon), { timeout: 60_000 }).toBe(0);
    await expect(tasks.locator("[data-archived-row]")).toHaveCount(0);
    await openTaskFolds(page);
    await expect(tasks.locator(`[data-task-row="${resumedRoot}"]`)).toBeVisible({ timeout: 30_000 });

    // B28, B29: 보관함에서 지우기… asks first, neither button holding the keyboard, and the record goes.
    const [other] = herdr.panes;
    await tasks.locator(`[data-task-row="${other}"]`).click({ button: "right" });
    await page.locator(`[data-task-context="${other}"] [data-menu-item="archive"]`).click();
    await expect.poll(archive(daemon), { timeout: 60_000 }).toBe(1);
    await openArchivedFold(page);
    await tasks.locator("[data-archived-row]").click({ button: "right" });
    await page.locator('[data-archived-context] [data-menu-item="delete"]').click();
    const confirm = page.locator("[data-confirm-archive-delete]");
    await expect(confirm.locator("[data-archive-delete]")).not.toBeFocused();
    await screenshot(page, "session-archive-delete");
    await confirm.locator("[data-archive-delete]").click();
    await expect.poll(archive(daemon), { timeout: 30_000 }).toBe(0);
    await expect(tasks.locator("[data-archived-row]")).toHaveCount(0);
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});
