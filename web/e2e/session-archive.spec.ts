// 세션 보관 on a real isolated daemon (PRD session-archive): a
// session with a working child asks first, archives into 보관됨 with every
// pane closed, opens its record, resumes the tree, and deletes from the archive.
import { expect, test, type Page } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { corePaneFocus, tabOfPane, withControl } from "./daemon-actions";
import { startFixture, type AgentFixture } from "./engine-fixture";
import { setAgentState, spawnRoot } from "./fixture-agents";
import { CLAUDE_FULL_ACCESS, continueFixtureTranscript } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { openTaskFolds, screenshot, showTasks } from "./wire";

test.describe.configure({ timeout: 240_000 });
test.use({ actionTimeout: 15_000 });

/** Waits for the resume of `session` and returns the pane its agent now runs in. */
async function confirmResume(daemon: Daemon, session: string): Promise<string> {
  return withControl(daemon, (control) => control.until((rest) => {
    const rows = ((rest.navigator as { agents?: { pane_id: string; agent_kind?: string | null; session_id?: string | null }[] } | undefined)?.agents ?? []);
    return rows.find((row) => row.agent_kind && row.session_id === session)?.pane_id;
  }, `the resume of ${session}`, 60_000));
}

/** Hide starts a resume as `claude --resume <session>` with the default 전부 허용 option; `from` is a live agent whose pane reads the line. */
async function expectResumeLine(fixture: AgentFixture, from: string, pane: string, session: string): Promise<void> {
  const read = await fixture.runAs(from)(["pane", "read", pane]);
  // The shell echoes the line as typed, which may quote the settings argument, so each part is read on its own.
  for (const part of [`claude --resume ${session}`, CLAUDE_FULL_ACCESS[0]!, "skipDangerousModePermissionPrompt"]) expect(read.out).toContain(part);
}

const archive = (daemon: Daemon) => () => Object.keys(JSON.parse(fs.readFileSync(path.join(daemon.stateDir, "core-state.json"), "utf8")).session_archive ?? {}).length;

async function openArchivedFold(page: Page) {
  const fold = page.locator('[data-task-fold="archived"] > button[data-task-focus="fold"]');
  await expect(fold).toBeVisible({ timeout: 30_000 });
  if ((await fold.getAttribute("aria-expanded")) === "false") await fold.click();
}

test("세션 보관 asks over running work, closes the tree into 보관됨, resumes it and deletes it from the archive", async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 1000 });
  // The family is the archived tree: the fixture's parent and its delegated child.
  const fixture = await startFixture({ family: true });
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(fixture, "session-archive");
    const [root, child] = fixture.panes;
    const rootSession = "fixture-one";
    const childSession = "fixture-two";
    // The session kept apart from the tree: an independent root the parent handed work to.
    const other = await spawnRoot(fixture, daemon, root, { name: "archive-other", repo: path.join(fixture.root, "fixture") });
    continueFixtureTranscript(fixture, rootSession, { task: "보관할 세션의 루트 작업" });
    await setAgentState(daemon, root, "working");
    await setAgentState(daemon, root, "idle");
    continueFixtureTranscript(fixture, childSession, { task: "루트가 맡긴 하위 작업" });
    await setAgentState(daemon, child, "working");
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
    await withControl(daemon, (control) => control.until((rest) => [root, child].every((pane) => tabOfPane(rest, pane) === undefined) || undefined, `${root} and ${child} closed`, 60_000));
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
    const resumedRoot = await confirmResume(daemon, rootSession);
    const resumedChild = await confirmResume(daemon, childSession);
    expect(resumedChild).not.toBe(resumedRoot);
    // Only the root takes the screen; its child opens behind it.
    await withControl(daemon, (control) => control.until((rest) => corePaneFocus(rest) === resumedRoot || undefined, `${resumedRoot} focused`, 30_000));
    // Reading a pane types into the pane it runs in, so the lines are read once the screen is settled.
    for (const [pane, session] of [[resumedRoot, rootSession], [resumedChild, childSession]] as const) await expectResumeLine(fixture, other, pane, session);
    await expect.poll(archive(daemon), { timeout: 60_000 }).toBe(0);
    await expect(tasks.locator("[data-archived-row]")).toHaveCount(0);
    await openTaskFolds(page);
    await expect(tasks.locator(`[data-task-row="${resumedRoot}"]`)).toBeVisible({ timeout: 30_000 });

    // B28, B29: 보관함에서 지우기… asks first, neither button holding the keyboard, and the record goes.
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
    fixture.stop();
  }
});
