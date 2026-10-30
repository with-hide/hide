import { expect, test, type Locator, type Page } from "@playwright/test";
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

/** Overview's close button shows the Workspace again. */
async function closeOverview(page: import("@playwright/test").Page) {
  await page.getByRole("button", { name: "Close Overview", exact: true }).click();
  await expect(page.locator("[data-overview-page]")).toHaveCount(0);
}

test("@platform a native session's record groups its requests by goal flow and keeps its PRs, children and asks through search and summaries off", async ({ page }) => {
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
    // The pane head carries no task chip (PRD session-record-goal-flow B1).
    await expect(page.locator(`[data-pane-view="${herdr.panes[0]}"]`)).toBeVisible();
    await expect(page.locator("[data-pane-task-chip]")).toHaveCount(0);
    await openRecord(page, root, herdr.panes[0]);
    const record = page.locator(RECORD);
    // The parent's own flows; an expanded child draws its flows inside its row.
    const flows = record.locator("[data-record-flows]").first().locator(":scope > ol > [data-record-flow]");
    // One flow named by the goal, its request shown by the label's summary (B3, B5).
    await expect(flows).toHaveCount(1, { timeout: 45_000 });
    await expect(flows.first().locator("[data-record-flow-name]")).toHaveText(FIRST_GOAL);
    await expect(record.locator("[data-record-request]").first()).toContainText("기록 화면 구현 요청", { timeout: 45_000 });
    await expect(record.locator("[data-record-artifact]")).toHaveText(["세션 기록 화면 시안"]);
    // The record reads the session's one list, the one its row's mark draws (status-model, One list per session):
    // this session's PR, #32 its pane holds on the checkout's branch (the fixture opens every PR on main), and its
    // child's in its own group; the two merged ones fold (B18, B19).
    await expect(record.locator('[data-record-prs="3"]')).toBeVisible({ timeout: 45_000 });
    await expect(root.locator("[data-agent-pr]")).toHaveAttribute("aria-label", /^PR 3 · /);
    await record.locator('[data-record-settled="2"]').click();
    await expect(record.locator('[data-record-pr-group="own"] [data-record-pr-row="31"]')).toBeVisible();
    await expect(record.locator(`[data-record-pr-group="${child}"] [data-record-pr-row="34"]`)).toBeVisible();
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
    await openRecord(page, root, herdr.panes[0]);
    // A new goal opens a second flow; the first folds to its result and the current one is open (B3, B13, B14).
    // Its own and its child's pull requests merged after its last line, so once the link store holds their merge
    // times Hide's fact stands for its result (fact-status B1).
    await expect(flows).toHaveCount(2, { timeout: 45_000 });
    await expect(flows.last().locator("[data-record-flow-name]")).toHaveText(NEXT_GOAL);
    await expect.poll(() => mergedPullRequests(daemon!.stateDir), { message: "the link store holds both merge times", timeout: 45_000 }).toEqual([31, 34]);
    await expect(flows.first().locator('[data-record-flow-result="settled"] [data-record-end="merged"]')).toBeVisible({ timeout: 45_000 });
    await expect(flows.first().locator("[data-record-request]")).toHaveCount(0);
    await expect(flows.last().locator("[data-record-request]")).toContainText(["이슈와 검색 검증 요청"], { timeout: 45_000 });
    // The open PR with failed CI shows; the settled ones stay one fold.
    await expect(record.locator('[data-record-pr-group="own"] [data-record-pr-row="32"]')).toBeVisible({ timeout: 45_000 });
    // The agent's question waits in 나를 기다리는 것, and 무시 can be taken back (B11, B12).
    const ask = record.locator("[data-record-asks] [data-record-ask]");
    await expect(ask).toContainText(["좁은 열 줄바꿈 기준을 정해 주세요"], { timeout: 45_000 });
    await ask.locator("[data-record-ask-dismiss]").click();
    await expect(ask).toHaveAttribute("data-dismissed", "true");
    await ask.locator("[data-record-ask-undo]").click();
    await expect(ask).not.toHaveAttribute("data-dismissed", "true");
    // Expand the child in place, retaining the parent's flows and navigation.
    await record.locator(`[data-record-child="${child}"] > button`).click();
    const detail = record.locator(`[data-record-child-detail="${child}"]`);
    await expect(flows).toHaveCount(2);
    await expect(detail.locator("[data-record-flow-name]")).toHaveText([/자식 세션 결과물 연결 검증/]);
    // Explicit Open record navigates, and the slot's back action returns.
    await detail.getByRole("button", { name: "Open record", exact: false }).click();
    await expect(record.locator("[data-record-flow-name]")).toHaveText([/자식 세션 결과물 연결 검증/]);
    await page.locator("[data-detail-back]").click();
    await expect(flows).toHaveCount(2);
    // Problems and follow-ups leave the record for the Overview backlog, which
    // never counts an ask, and 무시 there closes the line (D-13, B31).
    await expect(record.locator("[data-missed-work]")).toHaveCount(0);
    await openOverview(page, "backlog");
    const group = page.locator("[data-open-missed]");
    await group.locator("[data-open-missed-toggle]").click();
    const rows = group.locator("[data-missed-work]");
    await expect(rows).toHaveCount(2);
    await rows.filter({ hasText: MISSED }).getByRole("button", { name: "Skip", exact: true }).click();
    await expect(rows).toHaveCount(1);
    await closeOverview(page);
    await page.keyboard.press(chord("search"));
    await page.locator("[data-palette-input]").fill(NEXT_GOAL);
    const hits = page.locator('[data-palette-group="sessions"]');
    await expect(hits).toContainText(NEXT_GOAL, { timeout: 30_000 });
    await hits.locator("[data-palette-row]").first().click();
    await expect(record).toContainText(FIRST_GOAL);
    await page.keyboard.press(chord("settings"));
    await page.locator('[data-settings-tab="hideAi"]').click();
    await page.locator('[data-ai-agent-summary="true"]').click();
    await page.keyboard.press("Escape");
    // Summaries off: every request as sent in time order, no flow, ask or artifact (B28).
    await expect(record.locator("[data-record-flow]")).toHaveCount(0);
    await expect(record.locator("[data-record-asks]")).toHaveCount(0);
    await expect(record.locator("[data-record-artifacts]")).toHaveCount(0);
    await expect(record.locator("[data-record-raw-requests] [data-record-request]")).toHaveCount(2);
    await expect(record.locator('[data-record-prs="3"]')).toBeVisible();
    await expect(record.getByRole("heading", { level: 2 })).toContainText("Claude");
    await expect(record.locator("[data-record-child] > button")).toContainText("Claude");
    await record.locator(`[data-record-child="${child}"] > button`).click();
    await expect(record.locator('[data-record-child-detail] [data-record-prs="1"]')).toBeVisible();
    // With summaries off the backlog draws no missed line, though one is still open (B52).
    await openOverview(page, "backlog");
    await expect(page.locator("[data-backlog-section]").first()).toBeVisible();
    await expect(page.locator("[data-open-missed]")).toHaveCount(0);
  } finally { daemon?.stop(); herdr.stop(); }
});

/**
 * Opens a task row's record from its record button, which the row shows
 * while it holds focus or the pointer. Focus, not a hover: a row the list
 * moves under a still pointer (a child nesting under it) loses the hover.
 */
async function openRecord(page: Page, row: Locator, pane: string): Promise<void> {
  await row.locator('[data-task-focus="row"]').focus();
  await page.locator(`[data-task-record="${pane}"]`).click();
}

/**
 * A child's report letter is a dim line under its sender in the parent's
 * record, never a request of its own, and the question the woken turn ends on
 * waits in 나를 기다리는 것 (PRD session-record-goal-flow D-18, B11, B24).
 */
test("a report letter is a dim line in the record and the question it wakes waits for the operator", async ({ page }) => {
  const herdr = await startHerdr();
  let daemon: Daemon | undefined;
  try {
    const github = workFixture(herdr);
    daemon = await startHided(herdr, "session-work-letter", undefined, github.env);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await workRepositoryReady(daemon.stateDir);
    await enterWorkspace(page, "fixture");
    const at = Date.now();
    github.clock(at, at);
    const goal = "사이드바 두 화면 개선 작업";
    const session = labelAgent(herdr, herdr.panes[0], { task: goal, at, cwd: path.join(herdr.root, "fixture"), end: "waiting", request: "사이드바 두 화면 개선" });
    labelAgent(herdr, herdr.panes[1], {
      task: "자식 세션 결과물 연결 검증", at: at + 200, cwd: path.join(herdr.root, "fixture"), end: "waiting", toolOutputs: ["https://github.com/acme/repo/pull/34"],
    });
    declareParent(herdr, herdr.panes[1], herdr.panes[0]);
    const root = await taskRow(page, herdr.panes[0], goal);
    await openRecord(page, root, herdr.panes[0]);
    const record = page.locator(RECORD);
    await expect(record.locator("[data-record-request]")).toHaveCount(1, { timeout: 45_000 });
    // The child's report wakes the parent, which ends asking; the envelope
    // names the child by its agent name, as delivery writes it.
    const question = "앱을 다시 설치할지 정하세요";
    const letter: FixtureLabel = { task: goal, at: at + 5_000, end: "question", reply: question, work: { missed: [{ kind: "ask", text: question }], resolved: [], artifacts: [] } };
    fs.appendFileSync(path.join(claudeProjects(herdr), "e2e", `${session}.jsonl`), [
      { type: "user", sessionId: session, timestamp: new Date(at + 5_000).toISOString(), origin: { kind: "human" }, message: { role: "user", content: `Hide letter letter-1 from two (claude) [report]\n끝났어요\n${labelMarker(letter)}` } },
      { type: "assistant", sessionId: session, timestamp: new Date(at + 6_000).toISOString(), message: { role: "assistant", content: [{ type: "text", text: labelMarker(letter) }] } },
    ].map((line) => `${JSON.stringify(line)}\n`).join(""));
    await setFixtureLifecycle(herdr, herdr.panes[0], "working");
    await setFixtureLifecycle(herdr, herdr.panes[0], "idle");
    await expect(record.locator("[data-record-asks] [data-record-ask]")).toContainText([question], { timeout: 45_000 });
    await expect(record.locator('[data-record-letter="two"]')).toHaveCount(1);
    await expect(record.locator("[data-record-request]")).toHaveCount(2);
    // The operator's request keeps its summary; the letter is not counted as one.
    await expect(record.locator("[data-record-request]:not([data-record-letter])")).toContainText(["사이드바 두 화면 개선"]);
  } finally { daemon?.stop(); herdr.stop(); }
});

test("@platform 백로그로 saves a local memo without publishing and 되돌리기 restores the missed work", async ({ page }) => {
  const herdr = await startHerdr();
  let daemon: Daemon | undefined;
  try {
    const github = workFixture(herdr);
    daemon = await startHided(herdr, "session-backlog-local", undefined, github.env);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await workRepositoryReady(daemon.stateDir);
    await enterWorkspace(page, "fixture");
    const first = Date.now();
    github.clock(first, first);
    seedFirstWork(herdr, first);
    const missed = await openMissed(page, herdr.panes[0]!, FIRST_GOAL);
    // B16: one press retains the missed text in Hide. A private record is
    // never automatically published to its session's GitHub repository.
    await missed.locator("[data-missed-backlog-add]").click();
    await expect(missed).toHaveCount(0, { timeout: 30_000 });
    expect(createdIn(github.contexts)).toEqual([]);
    await openOverview(page, "backlog");
    const row = page.locator('[data-backlog-row^="local:"]', { hasText: MISSED });
    await expect(row).toBeVisible({ timeout: 30_000 });
    await row.getByRole("button").first().click();
    const item = page.locator("[data-overview-panel] [data-backlog-item]");
    await expect(item).toContainText(MISSED);
    await expect(item.locator("[data-item-github]")).toHaveCount(0);
    // Existing GitHub issues still open with full body/activity and retain
    // their external ownership: Hide offers no undo for someone else's issue.
    await item.locator("[data-item-close]").click();
    await page.locator('[data-backlog-row="github:acme/repo#41"]').getByRole("button").first().click();
    await expect(item.locator("[data-item-activity]")).toContainText("실제 fixture 댓글", { timeout: 30_000 });
    await expect(item.locator("[data-item-undo]")).toHaveCount(0);
    await item.locator("[data-item-close]").click();
    await row.getByRole("button").first().click();
    await item.locator("[data-item-undo]").click();
    await expect(row).toHaveCount(0, { timeout: 30_000 });
    // The missed line comes back to the backlog's missed group.
    await expect(missed).toBeVisible({ timeout: 30_000 });
    expect(createdIn(github.contexts)).toEqual([]);
  } finally { daemon?.stop(); herdr.stop(); }
});

test("@platform ⌘⇧M adds a backlog issue from the terminal and hands the keyboard back", async ({ page }) => {
  const herdr = await startHerdr();
  let daemon: Daemon | undefined;
  try {
    const github = workFixture(herdr, true);
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
    await expect(quick.locator("[data-backlog-destination]")).toHaveAttribute("data-backlog-destination", "local");
    // Publishing requires choosing the named repository before typing.
    await quick.getByRole("combobox", { name: "Save to", exact: true }).click();
    await page.getByRole("option", { name: /acme\/repo/ }).click();
    await quick.locator("[data-quick-add-input]").fill("빠른 추가\n두 번째 줄");
    await expect(quick.locator("[data-quick-add-input]")).toHaveValue("빠른 추가 두 번째 줄");
    // B40: Enter makes it, the made line names it, and the input closes on its own.
    await quick.locator("[data-quick-add-input]").press("Enter");
    // The response is lost after GitHub creates it. Enter retries the same
    // intent and joins #77 rather than making another external issue.
    await expect(quick.locator("[data-quick-add-failure]")).toBeVisible({ timeout: 30_000 });
    await quick.locator("[data-quick-add-input]").press("Enter");
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

/** The fixture pull requests whose merge time the link store holds. */
function mergedPullRequests(stateDir: string): number[] {
  const db = new DatabaseSync(path.join(stateDir, "links.sqlite3"), { readOnly: true });
  try {
    return db.prepare("SELECT number FROM prs WHERE merged_at IS NOT NULL ORDER BY number").all().map((row) => Number(row.number));
  } finally { db.close(); }
}

/**
 * The open missed line of `pane`'s session, once its 태스크 row says `goal`, in
 * the Overview backlog's missed group, where problems and follow-ups live
 * (PRD session-record-goal-flow D-13).
 */
async function openMissed(page: import("@playwright/test").Page, pane: string, goal: string) {
  await taskRow(page, pane, goal);
  await openOverview(page, "backlog");
  const group = page.locator("[data-open-missed]");
  const toggle = group.locator('[data-open-missed-toggle][aria-expanded="false"]');
  await expect(group).toBeVisible({ timeout: 45_000 });
  if (await toggle.count()) await toggle.click();
  const missed = group.locator("[data-missed-work]", { hasText: MISSED });
  await expect(missed).toBeVisible({ timeout: 45_000 });
  return missed;
}

test("@platform local missed-work adds survive a repository edit and keep the session origin", async ({ page }) => {
  const herdr = await startHerdr();
  let daemon: Daemon | undefined;
  try {
    const github = workFixture(herdr);
    daemon = await startHided(herdr, "backlog-repository-edit", undefined, github.env);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await workRepositoryReady(daemon.stateDir);
    await enterWorkspace(page, "fixture");
    const original = seedFirstWork(herdr, Date.now()).session;
    await openMissed(page, herdr.panes[0]!, FIRST_GOAL);
    await expect.poll(() => originalRepository(daemon!.stateDir, original), { timeout: 30_000 }).toBe("acme/repo");
    await closeOverview(page);
    await connectRepository(page, "acme/override");
    await workRepositoryReady(daemon.stateDir, "acme/override");
    let missed = await openMissed(page, herdr.panes[0]!, FIRST_GOAL);
    // Local default does not depend on today's remote connection.
    await missed.locator("[data-missed-backlog-add]").click();
    await expect(missed).toHaveCount(0, { timeout: 30_000 });
    expect(createdIn(github.contexts)).toEqual([]);
    expect(originalRepository(daemon.stateDir, original)).toBe("acme/repo");
    // A session that starts after the edit belongs to acme/override.
    await closeOverview(page);
    const fresh = seedFirstWork(herdr, Date.now(), NEXT_GOAL).session;
    await expect.poll(() => originalRepository(daemon!.stateDir, fresh), { timeout: 45_000 }).toBe("acme/override");
    missed = await openMissed(page, herdr.panes[0]!, NEXT_GOAL);
    await missed.locator("[data-missed-backlog-add]").click();
    await expect(missed).toHaveCount(0, { timeout: 30_000 });
    expect(createdIn(github.contexts)).toEqual([]);
    expect(originalRepository(daemon.stateDir, original)).toBe("acme/repo");
  } finally { daemon?.stop(); herdr.stop(); }
});

test("@platform a local missed-work add finishes while GitHub is unavailable and survives A-to-B-to-A edits", async ({ page }) => {
  const herdr = await startHerdr();
  let daemon: Daemon | undefined;
  try {
    const github = workFixture(herdr);
    daemon = await startHided(herdr, "backlog-local-offline", undefined, github.env);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await workRepositoryReady(daemon.stateDir);
    await enterWorkspace(page, "fixture");
    seedFirstWork(herdr, Date.now());
    const missed = await openMissed(page, herdr.panes[0]!, FIRST_GOAL);
    fs.writeFileSync(github.repositoryUnavailable, "GitHub is unavailable");
    await missed.locator("[data-missed-backlog-add]").click();
    await expect(missed).toHaveCount(0, { timeout: 30_000 });
    expect(createdIn(github.contexts)).toEqual([]);
    fs.rmSync(github.repositoryUnavailable);
    await closeOverview(page);
    await connectRepository(page, "acme/temporary");
    await connectRepository(page, "acme/repo");
    await openOverview(page, "backlog");
    await expect(page.locator('[data-backlog-row^="local:"]', { hasText: MISSED })).toBeVisible({ timeout: 30_000 });
    expect(createdIn(github.contexts)).toEqual([]);
  } finally { daemon?.stop(); herdr.stop(); }
});

test("@platform a migrated session with an unknown original repository can retain its missed work locally", async ({ page }) => {
  const herdr = await startHerdr();
  let daemon: Daemon | undefined;
  try {
    const github = workFixture(herdr);
    daemon = await startHided(herdr, "backlog-repository-legacy", undefined, github.env);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await workRepositoryReady(daemon.stateDir);
    await enterWorkspace(page, "fixture");
    const identity = seedFirstWork(herdr, Date.now()).session;
    await openMissed(page, herdr.panes[0]!, FIRST_GOAL);
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
    // Emulate the version 2 store while this owned daemon is stopped,
    // removing every later table and column, including display metadata
    // (version 6), the goal flows (version 7, whose task table and
    // columns come back first) and the closed flow's line (version 8). A v2
    // version number with later columns is not a v2 store.
    daemon = await daemon.restart((stateDir) => {
      const db = new DatabaseSync(path.join(stateDir, "links.sqlite3"));
      try {
        db.exec(`
          ALTER TABLE ledger_chapters DROP COLUMN closed_result;
          CREATE TABLE ledger_tasks(device TEXT NOT NULL, agent TEXT NOT NULL, id TEXT NOT NULL,
           task TEXT NOT NULL, chapter TEXT NOT NULL, name TEXT NOT NULL, at INTEGER NOT NULL,
           last_at INTEGER NOT NULL, end TEXT NOT NULL, PRIMARY KEY(device,agent,id,task));
          ALTER TABLE ledger_requests ADD COLUMN chapter TEXT;
          ALTER TABLE ledger_requests ADD COLUMN task_ids TEXT NOT NULL DEFAULT '[]';
          ALTER TABLE ledger_requests DROP COLUMN summary;
          ALTER TABLE ledger_requests DROP COLUMN answers;
          ALTER TABLE ledger_requests DROP COLUMN letter_from;
          ALTER TABLE ledger_requests DROP COLUMN images;
          ALTER TABLE ledger_turns ADD COLUMN task_ids TEXT NOT NULL DEFAULT '[]';
          ALTER TABLE ledger_turns ADD COLUMN output_task TEXT;
          ALTER TABLE ledger_turns DROP COLUMN line;
          ALTER TABLE ledger_turns DROP COLUMN artifacts;
          ALTER TABLE ledger_outputs ADD COLUMN task TEXT;
          ALTER TABLE ledger_titles DROP COLUMN omitted_requests;
          DELETE FROM meta WHERE key='summaries_owed_since';
          DROP TABLE ledger_session_origins;
          DROP TABLE ledger_project_connections;
          DROP TABLE ledger_backlog;
          DROP TABLE retention_aliases;
          DROP TABLE direct_observations;
          ALTER TABLE ledger_missed DROP COLUMN backlog_key;
          ALTER TABLE ledger_titles DROP COLUMN goal;
          ALTER TABLE ledger_titles DROP COLUMN summary;
          ALTER TABLE ledger_titles DROP COLUMN label_at;
          ALTER TABLE ledger_titles DROP COLUMN analysed_start;
          ALTER TABLE ledger_titles DROP COLUMN analysed_end;
          PRAGMA user_version=2;
        `);
        expect(db.prepare("PRAGMA table_info(ledger_titles)").all().map(column => column.name))
          .toEqual(["device", "agent", "id", "native_title", "manual_title", "state"]);
      }
      finally { db.close(); }
    });
    await page.goto("about:blank");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    const missed = await openMissed(page, herdr.panes[0]!, FIRST_GOAL);
    await missed.locator("[data-missed-backlog-add]").click();
    await expect(missed).toHaveCount(0, { timeout: 30_000 });
    expect(originalRepository(daemon.stateDir, identity)).toBeNull();
    expect(createdIn(github.contexts)).toEqual([]);
  } finally { daemon?.stop(); herdr.stop(); }
});
