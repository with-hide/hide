import { expect, test } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { DatabaseSync } from "node:sqlite";
import { claudeProjects, continueFixtureTranscript, declareParent, labelAgent, labelMarker, setFixtureLifecycle, startHerdr, type FixtureLabel } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { FIRST_GOAL, MISSED, NEXT_GOAL, nextWork, seedFirstWork, workFixture, workRepositoryReady } from "./session-work-fixture";
import { enterWorkspace, showProjectRow, taskRow } from "./wire";
import { chord } from "./chords";
import { openOverview } from "./overview-entry";

test.describe.configure({ timeout: 180_000 });

/** A session's record in the lane's 보기 slot (PRD sidebar-task-lens B15). */
const RECORD = 'aside[data-lane="detail"][data-detail="record"] [data-session-record]';

async function connectRepository(page: import("@playwright/test").Page, repository: string) {
  // The project's gear is on its row in the sidebar's 프로젝트 tree.
  await page.locator('[data-sidebar-lens-option="projects"]').click();
  const gear = page.locator('[data-project]', { hasText: "fixture" }).locator('[data-row-settings]').first();
  await (await showProjectRow(page, gear)).click();
  const sheet = page.locator('[data-project-settings]');
  await sheet.getByRole("combobox", { name: "GitHub repository", exact: true }).click();
  await page.getByRole("option", { name: "Choose repository", exact: true }).click();
  await sheet.getByRole("textbox", { name: "owner/repository" }).fill(repository);
  await sheet.getByRole("button", { name: "Apply", exact: true }).click();
  await expect(sheet).toContainText("Saved");
  await page.keyboard.press("Escape");
}

function originalRepository(stateDir: string, session: string) {
  const db = new DatabaseSync(path.join(stateDir,"links.sqlite3"),{readOnly:true});
  try { return db.prepare("SELECT repository FROM ledger_session_origins WHERE id=?").get(session)?.repository; }
  finally { db.close(); }
}

/**
 * Show task list pins the focused pane's checklist and the chord again closes
 * it; a press on the chip pins it and a second press closes it under the
 * pointer (PRD task-split-checklist B12, D-06).
 */
async function pinAndClose(page: import("@playwright/test").Page, pane: string, names: string[]) {
  const chip = page.locator(`[data-pane-task-chip="${pane}"]`);
  const list = page.locator(`[data-pane-task-list="${pane}"]`);
  await page.locator(`[data-pane-view="${pane}"] .xterm-helper-textarea`).focus();
  await page.keyboard.press(chord("toggle_task_list"));
  await expect(list.locator("[data-task-list-name]")).toHaveText(names);
  await page.keyboard.press(chord("toggle_task_list"));
  await expect(list).toHaveCount(0);
  await chip.click();
  await expect(chip).toHaveAttribute("aria-pressed", "true");
  await chip.click();
  await expect(list).toHaveCount(0);
  await expect(chip).toHaveAttribute("aria-pressed", "false");
}

test("@platform a native session keeps its tasks, child outputs and missed work through search and summaries off", async ({ page }) => {
  const herdr = await startHerdr();
  let daemon: Daemon | undefined;
  try {
    const github = workFixture(herdr);
    daemon = await startHided(herdr, "session-work-ledger", undefined, github.env);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await workRepositoryReady(daemon.stateDir);
    await enterWorkspace(page, "fixture");
    const first = Date.now();
    github.clock(first, first);
    const { session, child } = seedFirstWork(herdr, first);
    const root = await taskRow(page, herdr.panes[0], FIRST_GOAL);
    // The pane head's chip counts the session's tasks and opens the same
    // checklist under the pointer, whose 기록 열기 opens the record (PRD
    // task-split-checklist B9, B11, B12).
    const chip = page.locator(`[data-pane-task-chip="${herdr.panes[0]}"]`);
    await expect(chip).toHaveText("1/1", { timeout: 45_000 });
    await chip.hover();
    const list = page.locator(`[data-pane-task-list="${herdr.panes[0]}"]`);
    await expect(list.locator("[data-task-list-name]")).toHaveText(["기록 화면 구현"]);
    // The pane's own child is one line under the tasks, never a bar (PRD record-card-readability B15).
    await expect(list.locator("[data-task-list-children]")).toHaveAttribute("data-task-list-children", "1");
    await list.locator("[data-task-list-record]").click();
    await expect(list).toHaveCount(0);
    const record = page.locator(RECORD);
    const tasks = record.locator('[data-record-work]').first().locator('[data-record-task]');
    await expect(tasks).toHaveCount(1, { timeout: 45_000 });
    await expect(tasks.first().locator('[data-task-state="done"]')).toBeVisible();
    await expect(record.locator('[data-record-output="pr"]')).toHaveCount(2, { timeout: 45_000 });
    await page.locator(`[data-pane-view="${herdr.panes[0]}"] .xterm-helper-textarea`).focus();
    await page.keyboard.type(NEXT_GOAL);
    await page.keyboard.press("Enter");
    const second = Date.now();
    github.clock(first, second);
    continueFixtureTranscript(herdr, session, nextWork(second));
    await setFixtureLifecycle(herdr, herdr.panes[0], "working");
    await expect(root).toContainText(NEXT_GOAL, { timeout: 45_000 });
    await setFixtureLifecycle(herdr, herdr.panes[0], "idle");
    // The record button shows the record whether or not typing in the
    // terminal put the unpinned lane away (PRD right-lane B15).
    await root.hover();
    await page.locator(`[data-task-record="${herdr.panes[0]}"]`).click();
    // The second request asked for two deliverables: two tasks of one group.
    await expect(tasks).toHaveCount(3, { timeout: 45_000 });
    await expect(record).toContainText(FIRST_GOAL);
    await expect(record).toContainText(NEXT_GOAL);
    await expect(record.locator('[data-record-output="pr"]')).toHaveCount(3, { timeout: 45_000 });
    // First asked first: the finished task, then the split request's two, which share the failed CI of its PR.
    expect(await tasks.evaluateAll((rows) => rows.map((row) => row.querySelector("[data-task-state]")?.getAttribute("data-task-state")))).toEqual(["done", "ci_failed", "ci_failed"]);
    await expect(chip).toHaveText("1/3");
    // The split request's PR mark hangs on its first task; the second says the PR's state in a small word (status-marks B34).
    await pinAndClose(page, herdr.panes[0], ["기록 화면 구현", "이슈와 검색 검증", "좁은 열 줄바꿈 확인"]);
    // A child is a row that opens its own record in the slot, and the head's ← comes back (PRD sidebar-task-lens B16).
    await record.locator(`[data-record-child="${child}"]`).click();
    // The slot swaps the parent's three tasks for the child's one, so wait for that whole list.
    await expect(record.locator("[data-record-task]")).toHaveText([/자식 연결 검증/]);
    await page.locator("[data-detail-back]").click();
    await expect(tasks).toHaveCount(3);
    // An open missed item stands in its own section, named by the task its turn made (PRD record-card-readability B12).
    const missed = record.locator("[data-record-missed] [data-missed-work]", { hasText: MISSED });
    await expect(missed.locator("[data-missed-note]")).toHaveText(" · 기록 화면 구현");
    // The device's open missed work also folds above the Overview backlog,
    // newest first, and 무시 there closes the same item (PRD sidebar-task-lens B52).
    await openOverview(page, "backlog");
    const group = page.locator("[data-open-missed]");
    await expect(group.locator("[data-missed-work]")).toHaveCount(0);
    await group.locator("[data-open-missed-toggle]").click();
    const rows = group.locator("[data-missed-work]");
    await expect(rows).toHaveCount(2);
    await expect(rows.last()).toContainText(MISSED);
    await rows.last().getByRole("button", { name: "Dismiss", exact: true }).click();
    await expect(rows).toHaveCount(1);
    // Closing the Overview shows the Workspace again, its slot still holding the record.
    await page.getByRole("button", { name: "Close Overview", exact: true }).click();
    await expect(page.locator("[data-overview-page]")).toHaveCount(0);
    // What is handled leaves the record (B13).
    await expect(missed).toHaveCount(0);
    await page.keyboard.press(chord("search"));
    await page.locator("[data-palette-input]").fill(NEXT_GOAL);
    const hits = page.locator('[data-palette-group="sessions"]');
    await expect(hits).toContainText(NEXT_GOAL, { timeout: 30_000 });
    await hits.locator('[data-palette-row]').first().click();
    await expect(record).toContainText(FIRST_GOAL);
    await page.keyboard.press(chord("settings"));
    await page.locator('[data-settings-tab="hideAi"]').click();
    await page.locator('[data-ai-agent-summary="true"]').click();
    await page.keyboard.press("Escape");
    await expect(record.locator('[data-record-task]')).toHaveCount(0);
    await expect(record.locator('[data-missed-work]')).toHaveCount(0);
    // The parent's two requests; the child's is in its own record, behind its row.
    await expect(record.locator('[data-record-request]')).toHaveCount(2);
    await expect(record.locator('[data-record-output="pr"]')).toHaveCount(3);
    await expect(record.getByRole('heading', { level: 2 })).toContainText('Claude');
    await expect(record.locator('[data-record-child]')).toContainText('Claude');
    // The child's pull request stands on the child's row; the session's own stand beside the title (B15).
    await expect(record.locator('[data-record-child] [data-record-output="pr"]')).toHaveCount(1);
    await expect(record.locator('h2 [data-record-output="pr"]')).toHaveCount(2);
    // With summaries off the backlog draws no missed line, though one is still open (B52).
    await openOverview(page, "backlog");
    await expect(page.locator("[data-backlog-section]").first()).toBeVisible();
    await expect(page.locator("[data-open-missed]")).toHaveCount(0);
  } finally { daemon?.stop(); herdr.stop(); }
});

/**
 * A session that asks once and delegates finishes its tasks once the child's
 * PR merges, and the turn its child's report wakes keeps `!` for the question
 * it ends on, though the merge finished the task first; the child counts done
 * (PRD task-continuation B1, B3, B16, D-07).
 */
test("a report letter's turn keeps the question's mark on the task its sender's merged PR finished", async ({ page }) => {
  const herdr = await startHerdr();
  let daemon: Daemon | undefined;
  try {
    const github = workFixture(herdr);
    daemon = await startHided(herdr, "session-work-continuation", undefined, github.env);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await workRepositoryReady(daemon.stateDir);
    await enterWorkspace(page, "fixture");
    const at = Date.now();
    github.clock(at, at);
    const goal = "사이드바 두 화면 개선 작업";
    const session = labelAgent(herdr, herdr.panes[0], {
      task: goal, at, cwd: path.join(herdr.root, "fixture"), end: "waiting",
      work: { task: { kind: "new", names: ["쉬는 행 빈 원", "자식 PR 표시"] }, missed: [], resolved: [], output_task_id: null },
    });
    labelAgent(herdr, herdr.panes[1], {
      task: "자식 세션 결과물 연결 검증", at: at + 200, cwd: path.join(herdr.root, "fixture"), end: "waiting", toolOutputs: ["https://github.com/acme/repo/pull/34"],
    });
    declareParent(herdr, herdr.panes[1], herdr.panes[0]);
    const chip = page.locator(`[data-pane-task-chip="${herdr.panes[0]}"]`);
    const list = page.locator(`[data-pane-task-list="${herdr.panes[0]}"]`);
    // The child's merged PR finishes the task that spawned it.
    await expect(chip).toHaveText("2/2", { timeout: 45_000 });
    await chip.click();
    await expect(list.locator('[data-task-box="checked"]')).toHaveCount(2);
    await expect(list.locator("[data-task-asks]")).toHaveCount(0);
    // The child's report wakes the parent, which ends asking; the envelope
    // names the child by its agent name, as delivery writes it.
    const letter: FixtureLabel = { task: goal, at: at + 5_000, end: "question", reply: "앱을 다시 설치할지 정하세요" };
    fs.appendFileSync(path.join(claudeProjects(herdr), "e2e", `${session}.jsonl`), [
      { type: "user", sessionId: session, timestamp: new Date(at + 5_000).toISOString(), origin: { kind: "human" }, message: { role: "user", content: `Hide letter letter-1 from two (claude) [report]\n끝났어요\n${labelMarker(letter)}` } },
      { type: "assistant", sessionId: session, timestamp: new Date(at + 6_000).toISOString(), message: { role: "assistant", content: [{ type: "text", text: labelMarker(letter) }] } },
    ].map((record) => `${JSON.stringify(record)}\n`).join(""));
    await setFixtureLifecycle(herdr, herdr.panes[0], "working");
    await setFixtureLifecycle(herdr, herdr.panes[0], "idle");
    await expect(list.locator("[data-task-asks]")).toHaveCount(2, { timeout: 45_000 });
    await expect(chip).toHaveText("2/2");
    await expect(list.locator('[data-task-box="checked"]')).toHaveCount(2);
    await expect(list.locator("[data-task-list-children]")).toHaveText("Delegated1· Done 1");
  } finally { daemon?.stop(); herdr.stop(); }
});

test("@platform 백로그로 makes one title-only issue, a lost create response converges on retry, and 되돌리기 closes it", async ({ page }) => {
  const herdr = await startHerdr();
  let daemon: Daemon | undefined;
  try {
    const github = workFixture(herdr, true);
    daemon = await startHided(herdr, "session-backlog-retry", undefined, github.env);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await workRepositoryReady(daemon.stateDir);
    await enterWorkspace(page, "fixture");
    const first = Date.now();
    github.clock(first, first);
    const { session } = seedFirstWork(herdr, first);
    const root = await taskRow(page, herdr.panes[0], FIRST_GOAL);
    // 백로그로 creates only in the repository the ledger proved the session
    // started in, so wait for that proof as an operator's later press would.
    await expect.poll(() => originalRepository(daemon!.stateDir, session), { timeout: 30_000 }).toBe("acme/repo");
    await root.hover();
    await page.locator(`[data-task-record="${herdr.panes[0]}"]`).click();
    const missed = page.locator(RECORD).locator("[data-missed-work]", { hasText: MISSED });
    // PRD overview-device-tasks B43-B45: one press, no form; GitHub made the
    // issue but its answer was lost, so the line fails, and the retry joins
    // the issue GitHub already has instead of making a second one.
    await missed.locator("[data-missed-backlog-add]").click({ timeout: 45_000 });
    await expect(missed.locator("[data-missed-failed]")).toBeVisible({ timeout: 30_000 });
    await missed.locator("[data-missed-failed]").getByRole("button", { name: "Retry", exact: true }).click();
    // The item is made, so the line leaves the record (PRD record-card-readability B13).
    await expect(missed).toHaveCount(0, { timeout: 30_000 });
    const calls = () => fs.readFileSync(github.calls, "utf8").trim().split("\n").map((line) => JSON.parse(line) as string[]);
    const creates = calls().filter((args) => args[0] === "issue" && args[1] === "create");
    expect(creates).toHaveLength(1);
    // Title only: nothing of the conversation goes to GitHub (D-10).
    expect(creates[0]![creates[0]!.indexOf("--title") + 1]).toBe(MISSED);
    expect(creates[0]![creates[0]!.indexOf("--body") + 1]).toBe("");

    // The item is in the Overview's backlog, whose 되돌리기 closes the issue
    // and opens the missed line again (B32).
    await openOverview(page, "backlog");
    await page.locator('[data-backlog-row="github:acme/repo#77"]').getByRole("button").first().click();
    const item = page.locator("[data-overview-panel] [data-backlog-item]");
    await expect(item).toHaveAttribute("data-backlog-item", "github:acme/repo#77");
    // D-21: #41 is in the same backlog but Hide did not make it, so its
    // panel has no 되돌리기 and Hide never closes someone else's issue.
    await page.locator('[data-backlog-row="github:acme/repo#41"]').getByRole("button").first().click();
    await expect(item).toHaveAttribute("data-backlog-item", "github:acme/repo#41");
    await expect(item.locator("[data-item-start]")).toBeVisible();
    await expect(item.locator("[data-item-undo]")).toHaveCount(0);
    await page.locator('[data-backlog-row="github:acme/repo#77"]').getByRole("button").first().click();
    await expect(item).toHaveAttribute("data-backlog-item", "github:acme/repo#77");
    await item.locator("[data-item-undo]").click();
    await expect.poll(() => calls().some((args) => args[0] === "issue" && args[1] === "close" && args[2] === "77"), { timeout: 30_000 }).toBe(true);
    await page.keyboard.press("Escape");
    await page.keyboard.press("Escape");
    await expect(missed).toBeVisible({ timeout: 30_000 });
  } finally { daemon?.stop(); herdr.stop(); }
});

test("@platform ⌘⇧M adds a backlog issue from the terminal and hands the keyboard back", async ({ page }) => {
  const herdr = await startHerdr();
  let daemon: Daemon | undefined;
  try {
    const github = workFixture(herdr);
    daemon = await startHided(herdr, "quick-add", undefined, github.env);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await workRepositoryReady(daemon.stateDir);
    await enterWorkspace(page, "fixture");
    const terminal = page.locator(`[data-pane-view="${herdr.panes[0]}"] .xterm-helper-textarea`);
    await terminal.focus();
    // B37-B39: the input opens over the Workspace on the front checkout's
    // project, says why, and says where the item goes.
    await page.keyboard.press(chord("add_backlog"));
    const quick = page.locator("[data-quick-add]");
    await expect(quick).toBeVisible();
    await expect(quick.locator("[data-quick-add-input]")).toBeFocused();
    await expect(quick.locator("[data-quick-add-reason]")).toHaveAttribute("data-quick-add-reason", "front");
    await expect(quick.locator("[data-quick-add-destination]")).toBeVisible({ timeout: 30_000 });
    // B42: Esc makes nothing and the terminal holds the keyboard again.
    await page.keyboard.press("Escape");
    await expect(quick).toHaveCount(0);
    await expect(terminal).toBeFocused();
    // B42: a pasted multi-line text is one line.
    await page.keyboard.press(chord("add_backlog"));
    await quick.locator("[data-quick-add-input]").fill("빠른 추가\n두 번째 줄");
    await expect(quick.locator("[data-quick-add-input]")).toHaveValue("빠른 추가 두 번째 줄");
    // B40: Enter makes it, the made line names it, and the input closes on its own.
    await page.keyboard.press("Enter");
    await expect(quick.locator('[data-quick-add-made="github:acme/repo#77"]')).toBeVisible({ timeout: 30_000 });
    await expect(quick).toHaveCount(0, { timeout: 10_000 });
    await expect(terminal).toBeFocused();
    const creates = fs.readFileSync(github.calls, "utf8").trim().split("\n").map((line) => JSON.parse(line) as string[]).filter((args) => args[0] === "issue" && args[1] === "create");
    expect(creates).toHaveLength(1);
    expect(creates[0]![creates[0]!.indexOf("--title") + 1]).toBe("빠른 추가 두 번째 줄");
  } finally { daemon?.stop(); herdr.stop(); }
});

/** What the fixture `gh` was asked to create, by the repository each create named. */
function createdIn(contexts: string): (string | null)[] {
  if (!fs.existsSync(contexts)) return [];
  return fs.readFileSync(contexts, "utf8").trim().split("\n").map((line) => JSON.parse(line) as { args: string[]; repository: string | null })
    .filter((call) => call.args[0] === "issue" && call.args[1] === "create").map((call) => call.repository);
}

/** The record of the 태스크 list row of `pane`, once the row says `goal`, opened in the 보기 slot; returns its missed line. */
async function openRecord(page: import("@playwright/test").Page, pane: string, goal: string) {
  const root = await taskRow(page, pane, goal);
  await root.hover();
  await page.locator(`[data-task-record="${pane}"]`).click();
  const missed = page.locator(RECORD).locator("[data-missed-work]", { hasText: MISSED });
  await expect(missed).toBeVisible({ timeout: 45_000 });
  return missed;
}

test("@platform 백로그로 keeps a session's original repository after a connection edit, and a session started after it creates in the new one", async ({ page }) => {
  const herdr = await startHerdr();
  let daemon: Daemon | undefined;
  try {
    const github = workFixture(herdr);
    daemon = await startHided(herdr, "backlog-repository-edit", undefined, github.env);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await workRepositoryReady(daemon.stateDir);
    await enterWorkspace(page, "fixture");
    const original = seedFirstWork(herdr, Date.now()).session;
    let missed = await openRecord(page, herdr.panes[0]!, FIRST_GOAL);
    await expect.poll(() => originalRepository(daemon!.stateDir, original), { timeout: 30_000 }).toBe("acme/repo");
    await connectRepository(page, "acme/override");
    await workRepositoryReady(daemon.stateDir, "acme/override");
    // The session started in acme/repo; its missed line makes nothing in acme/override.
    await missed.locator("[data-missed-backlog-add]").click();
    await expect(missed.locator('[data-missed-failed="repository_changed"]')).toBeVisible({ timeout: 30_000 });
    expect(createdIn(github.contexts)).toEqual([]);
    expect(originalRepository(daemon.stateDir, original)).toBe("acme/repo");
    // A session that starts after the edit belongs to acme/override.
    const fresh = seedFirstWork(herdr, Date.now(), NEXT_GOAL).session;
    await expect.poll(() => originalRepository(daemon!.stateDir, fresh), { timeout: 45_000 }).toBe("acme/override");
    missed = await openRecord(page, herdr.panes[0]!, NEXT_GOAL);
    await missed.locator("[data-missed-backlog-add]").click();
    await expect(missed).toHaveCount(0, { timeout: 30_000 });
    expect(createdIn(github.contexts)).toEqual(["acme/override"]);
    expect(originalRepository(daemon.stateDir, original)).toBe("acme/repo");
  } finally { daemon?.stop(); herdr.stop(); }
});

test("@platform a 백로그로 create in flight across an A-to-B-to-A connection edit makes nothing, and pressing it again makes one", async ({ page }) => {
  const herdr = await startHerdr();
  let daemon: Daemon | undefined;
  try {
    const github = workFixture(herdr);
    daemon = await startHided(herdr, "backlog-repository-epoch", undefined, github.env);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await workRepositoryReady(daemon.stateDir);
    await enterWorkspace(page, "fixture");
    const { session } = seedFirstWork(herdr, Date.now());
    const missed = await openRecord(page, herdr.panes[0]!, FIRST_GOAL);
    await expect.poll(() => originalRepository(daemon!.stateDir, session), { timeout: 30_000 }).toBe("acme/repo");
    const callsBefore = fs.readFileSync(github.calls, "utf8").trim().split("\n").length;
    fs.writeFileSync(github.repositoryGate, "hold the external repository read");
    await missed.locator("[data-missed-backlog-add]").click();
    await expect.poll(() => fs.readFileSync(github.calls, "utf8").trim().split("\n").slice(callsBefore)
      .some((line) => { const args = JSON.parse(line) as string[]; return args[0] === "repo" && args[1] === "view"; }), { timeout: 30_000 }).toBe(true);
    try {
      await connectRepository(page, "acme/temporary");
      await connectRepository(page, "acme/repo");
    } finally { fs.rmSync(github.repositoryGate, { force: true }); }
    // The connection came back to the same name, but the create was asked under the first one.
    const failed = missed.locator('[data-missed-failed="repository_changed"]');
    await expect(failed).toBeVisible({ timeout: 30_000 });
    expect(createdIn(github.contexts)).toEqual([]);
    // A moved repository is no failure a retry can change: 백로그로 again reads the repository anew.
    await expect(failed.getByRole("button", { name: "Retry", exact: true })).toHaveCount(0);
    await failed.locator("[data-missed-backlog-add]").click();
    await expect(missed).toHaveCount(0, { timeout: 30_000 });
    expect(createdIn(github.contexts)).toEqual(["acme/repo"]);
  } finally { daemon?.stop(); herdr.stop(); }
});

test("@platform a migrated session's unknown original repository refuses 백로그로 in today's repository", async ({ page }) => {
  const herdr = await startHerdr();
  let daemon: Daemon | undefined;
  try {
    const github = workFixture(herdr);
    daemon = await startHided(herdr, "backlog-repository-legacy", undefined, github.env);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await workRepositoryReady(daemon.stateDir);
    await enterWorkspace(page, "fixture");
    const identity = seedFirstWork(herdr, Date.now()).session;
    await openRecord(page, herdr.panes[0]!, FIRST_GOAL);
    // This migration starts from completed, durable native work. Windows
    // fixture shutdown is forced; a rendered goal is not a saved label.
    const savedLabels = path.join(daemon.stateDir, "labels.json");
    const node = daemon.node;
    await expect.poll(() => {
      if (!fs.existsSync(savedLabels)) return null;
      const saved = JSON.parse(fs.readFileSync(savedLabels, "utf8")) as {
        targets?: Record<string, Record<string, { native_session_id?: string; goal?: string; end?: string }>>;
      };
      const label = saved.targets?.[node]?.[herdr.panes[0]!];
      return { session: label?.native_session_id, goal: label?.goal, end: label?.end };
    }, { message: "completed native label reached disk before migration restart", timeout: 30_000 })
      .toEqual({ session: identity, goal: FIRST_GOAL, end: "done" });
    // Emulate the version 2 store while this owned daemon is stopped.
    daemon = await daemon.restart((stateDir) => {
      const db = new DatabaseSync(path.join(stateDir, "links.sqlite3"));
      try { db.exec("DROP TABLE ledger_session_origins; DROP TABLE ledger_project_connections; DROP TABLE ledger_backlog; DROP TABLE retention_aliases; ALTER TABLE ledger_missed DROP COLUMN backlog_key; PRAGMA user_version=2;"); }
      finally { db.close(); }
    });
    await page.goto("about:blank");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    const missed = await openRecord(page, herdr.panes[0]!, FIRST_GOAL);
    await missed.locator("[data-missed-backlog-add]").click();
    await expect(missed.locator('[data-missed-failed="repository_changed"]')).toBeVisible({ timeout: 30_000 });
    expect(originalRepository(daemon.stateDir, identity)).toBeNull();
    expect(createdIn(github.contexts)).toEqual([]);
  } finally { daemon?.stop(); herdr.stop(); }
});
