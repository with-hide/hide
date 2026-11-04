import { skipOwnWindowsAgentStart } from "./own-windows-agent-start";
// Closing an agent that spawned others (PRD close-agent-subtree): the close
// sheet lists the descendants, Escape sends nothing, Enter closes the whole
// subtree deepest first with the target last, and Close only leaves the
// children running as the operator's own rows. Delete worktree asks the same
// question about the agents its checkout spawned outside it.

import { expect, test, type Locator } from "@playwright/test";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { startFixture, type AgentFixture } from "./engine-fixture";
import { connectControl } from "./hided-control.mjs";
import { closePane, focusTab, openProject, openWorktree, showTabOf, tabOfPane, viewPane, withControl } from "./daemon-actions";
import { agentReading, finishUnseenTurn, setAgentAsking, setAgentState, spawnChild, spawnRoot, startAgentInProject, takeOverSession } from "./fixture-agents";
import { claudeProjects, writeFixtureTranscript, type FixtureLabel } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { countSent, enterWorkspace, screenshot } from "./wire";
import { chord } from "./chords";
import { completedCloseOrder, type CloseRecord, type CloseTiming } from "./close-order";

test.describe.configure({ timeout: 180_000 });
test.use({ actionTimeout: 15_000 });

/** Waits until the daemon lists none of `panes` in any tab. */
async function gone(daemon: Daemon, panes: string[], timeout = 60_000): Promise<void> {
  await withControl(daemon, (control) => control.until((rest) => (panes.every((pane) => tabOfPane(rest, pane) === undefined) ? true : undefined), `${panes.join(", ")} closed`, timeout));
}

/** Whether the daemon still lists `pane` in a tab. */
async function listed(daemon: Daemon, pane: string): Promise<boolean> {
  return withControl(daemon, async (control) => (await control.until((rest) => ({ live: tabOfPane(rest, pane) !== undefined }), `the layouts naming ${pane}`)).live);
}

/** Existing diagnostics in FIFO order, in the daemon's two bounded files. */
function closeRecords(daemon: Daemon): CloseRecord[] {
  return ["core.previous.jsonl", "core.jsonl"].flatMap((name) => {
    const file = path.join(daemon.stateDir, "Logs", name);
    if (!fs.existsSync(file)) return [];
    // The writer may be appending the last record. Only complete lines count;
    // malformed complete records fail instead of silently losing evidence.
    return fs.readFileSync(file, "utf8").split("\n").slice(0, -1)
      .map((line) => JSON.parse(line));
  });
}

function closeTimings(records: CloseRecord[]): CloseTiming[] {
  return records.filter((record) => record.kind === "pane_op.timing" && record.op === "pane.close") as CloseTiming[];
}

/**
 * Snapshots coalesce changes, so two absences in a frame do not prove a tie.
 * Observe this request's actual completed closes in the existing diagnostics:
 * each child must be applied before the next close is requested, and every
 * watched pane must also leave the published layouts. The pre-click boundary
 * and operation IDs exclude old records; exact pane IDs exclude other closes.
 */
async function watchCloseOrder(daemon: Daemon, watched: string[]): Promise<{ order: Promise<string[]>; close: () => void }> {
  const before = new Set(closeTimings(closeRecords(daemon)).map((record) => record.op_id));
  const started = Date.now();
  const control = await connectControl(daemon);
  const current = (records: CloseRecord[]) => closeTimings(records).filter((record) => !before.has(record.op_id)
    && record.requested_at_unix_ms >= started
    && record.pane_ids.length === 1 && watched.includes(record.pane_ids[0]!));
  const done = (async () => {
    await control.until((rest) => watched.every((pane) => tabOfPane(rest, pane) === undefined) ? true : undefined, "every watched pane closed", 60_000);
    // The diagnostic writer is independent of snapshot publication.
    await expect.poll(() => current(closeRecords(daemon)).length, { timeout: 10_000, message: "every watched close has a timing record" }).toBe(watched.length);
    const records = closeRecords(daemon);
    return completedCloseOrder(records, current(records), watched);
  })();
  // A failure before the caller awaits `done` still reaches it; this only
  // keeps the runner from reporting the same rejection as unhandled.
  done.catch(() => undefined);
  return { order: done, close: () => control.close() };
}

let delegated = 0;

/**
 * Has the agent in `parent` delegate a child in a tab of its own, as a real
 * agent does. With `named`, the child runs a session whose transcript is
 * labelled so from the start, since a later session is no longer that
 * child's. The child's pane is sized for the keys a spec types into it.
 */
async function delegate(fixture: AgentFixture, daemon: Daemon, parent: string, name: string, named?: FixtureLabel, repo = path.join(fixture.root, "fixture")): Promise<string> {
  let session: string | undefined;
  if (named) {
    session = `close-${name}-${process.pid}-${(delegated += 1)}`;
    writeFixtureTranscript(claudeProjects(fixture), session, named);
  }
  const pane = await spawnChild(fixture, daemon, parent, { name, repo, session });
  await withControl(daemon, async (control) => viewPane(control, pane));
  // Its stand-in drops keys typed before it reads its terminal.
  await agentReading(fixture, pane);
  return pane;
}

/**
 * The fixture's second agent was spawned beside the first as its child: a
 * new session in its pane ends that relationship, so the pair are two
 * operator-owned agents, as two panes of one tab are.
 */
async function releaseChild(fixture: AgentFixture, daemon: Daemon): Promise<void> {
  await takeOverSession(fixture, daemon, fixture.panes[1], "fixture-released");
}

/** Makes `dir` a Git repository on `main` with one commit, the folder a spawned child opens in. */
function repository(dir: string): void {
  fs.mkdirSync(dir, { recursive: true });
  git(dir, ["init", "-q", "-b", "main"]);
  git(dir, ["commit", "-q", "--allow-empty", "-m", "baseline"]);
}

/** The tops of `buttons`, rounded: one value means they share one row. */
async function rowTops(buttons: Locator[]): Promise<number[]> {
  const tops = new Set<number>();
  for (const button of buttons) tops.add(Math.round((await button.boundingBox())!.y));
  return [...tops];
}

test("the close sheet counts and brightens what needs the operator, stays live while open, and closes what it shows", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const fixture = await startFixture({ family: true });
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(fixture, "close-subtree-states");
    const live = daemon;
    const [target] = fixture.panes;
    await releaseChild(fixture, live);
    const working = await delegate(fixture, live, target, "working", { task: "계보 투영 구현" });
    const asking = await delegate(fixture, live, target, "asking", { task: "병합 전 검증 실행", reply: "병합 전에 검증을 다시 돌릴까요?", question: true });
    const finished = await delegate(fixture, live, target, "finished", { task: "릴리스 노트 초안" });
    const quiet = await delegate(fixture, live, target, "quiet", { task: "로그 파일 정리 작업" });
    await setAgentState(live, working, "working");
    await setAgentAsking(live, asking);
    // The operator's own tab is the one shown, so the finished turn is not seen.
    await finishUnseenTurn(fixture, live, finished, fixture.tab);
    await withControl(live, (control) => focusTab(control, fixture, fixture.tab));

    const last = new Map<string, Record<string, unknown>>();
    const sent = countSent(page, last);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    await expect(page.locator(`nav[data-sidebar] [data-pane="${target}"] [data-tree-chevron="${target}"]`).first()).toBeVisible({ timeout: 30_000 });

    // The sheet chooses where the keyboard starts from the states it opens
    // on, so it opens once the sidebar shows every child's: a row whose
    // state is not in yet reads Unknown, which is the core's close guard.
    // The root's chevron opens its children in place (agent-hierarchy-screens B10), each with its own mark.
    const chevron = page.locator(`nav[data-sidebar] [data-pane="${target}"] [data-tree-chevron="${target}"]`).first();
    await chevron.click();
    // A delegated child draws only in progress or quiet (status-marks B24); its state word is on the mark.
    for (const [pane, status] of [[working, "working"], [asking, "question"]]) {
      await expect(page.locator(`nav[data-sidebar] [data-pane="${pane}"] [data-agent-status]`).first()).toHaveAttribute("data-agent-status", status, { timeout: 30_000 });
    }
    await expect(page.locator(`nav[data-sidebar] [data-pane="${working}"] [data-mark]`).first()).toHaveAttribute("data-mark", "going");
    await expect(page.locator(`nav[data-sidebar] [data-pane="${asking}"] [data-mark]`).first()).toHaveAttribute("data-mark", "quiet");
    for (const pane of [finished, quiet]) {
      await expect(page.locator(`nav[data-sidebar] [data-pane="${pane}"] [data-agent-status]`).first()).not.toHaveAttribute("data-agent-status", "unknown", { timeout: 30_000 });
    }
    await expect(page.locator(`[data-agent-open="${target}"]`).first()).not.toHaveAccessibleName(/\bUnknown\b/);
    await chevron.click();

    const sheet = page.locator("[data-confirm-subtree]");
    const summary = sheet.locator("[data-subtree-summary]");
    await page.locator(`[data-terminal-host="${target}"]`).click();
    await page.keyboard.press(chord("close_tab"));
    await expect(summary).toHaveAttribute("data-subtree-summary", "waiting for you 1 · working 1");
    await expect(sheet.getByRole("heading")).toHaveText("Close this agent and 4 children?");
    await expect(sheet.locator(`[data-subtree-row="${working}"]`)).toHaveAttribute("data-subtree-state", "working");
    await expect(sheet.locator(`[data-subtree-row="${asking}"]`)).toHaveAttribute("data-subtree-state", "waiting");
    // A delegated child's finished turn is its parent's news, not a dot of its own (status-marks B24).
    for (const pane of [finished, quiet]) await expect(sheet.locator(`[data-subtree-row="${pane}"]`)).toHaveAttribute("data-subtree-state", "quiet");
    // Bright rows keep full opacity; the quiet ones are dimmed.
    const opacity = (pane: string) => sheet.locator(`[data-subtree-row="${pane}"]`).evaluate((row) => Number(getComputedStyle(row).opacity));
    for (const pane of [working, asking]) expect(await opacity(pane)).toBe(1);
    for (const pane of [finished, quiet]) expect(await opacity(pane)).toBeLessThan(1);
    // Each row's accessible name says its state; only the rows that need the
    // operator show the word, and a quiet row's mark says it on hover.
    await expect(sheet.locator(`[data-subtree-row="${asking}"]`)).toHaveAttribute("aria-label", /병합 전 검증/);
    for (const pane of [working, asking]) await expect(sheet.locator(`[data-subtree-row="${pane}"] [data-subtree-status]`)).toHaveCount(1);
    for (const pane of [finished, quiet]) {
      await expect(sheet.locator(`[data-subtree-row="${pane}"] [data-subtree-status]`)).toHaveCount(0);
      await expect(sheet.locator(`[data-subtree-row="${pane}"] [data-row-mark-hint]`)).toHaveCount(1);
    }
    await screenshot(page, "close-subtree-states");

    // The list is live (D-20, B13): a child spawned while the sheet is open
    // shows up, one that closes elsewhere drops out, and the title follows.
    const late = await delegate(fixture, live, target, "late");
    await expect(sheet.locator(`[data-subtree-row="${late}"]`)).toBeVisible({ timeout: 30_000 });
    await expect(sheet.getByRole("heading")).toHaveText("Close this agent and 5 children?");
    await withControl(live, (control) => closePane(control, quiet));
    await expect(sheet.locator(`[data-subtree-row="${quiet}"]`)).toHaveCount(0, { timeout: 30_000 });
    await expect(sheet.getByRole("heading")).toHaveText("Close this agent and 4 children?");
    expect(sent.get("close_tree") ?? 0).toBe(0);

    // Enter closes exactly what the sheet shows at the press.
    await expect(sheet.locator("[data-subtree-close-all]")).toBeFocused();
    await page.keyboard.press("Enter");
    expect(sent.get("close_tree")).toBe(1);
    expect(new Set(last.get("close_tree")?.pane_ids as string[])).toEqual(new Set([working, asking, finished, late]));
    await gone(live, [target, working, asking, finished, late]);
  } finally {
    daemon?.stop();
    fixture.stop();
  }
});

test("the close sheet closes the whole subtree deepest first, and Close only keeps the children", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const fixture = await startFixture({ family: true });
  let daemon: Daemon | null = null;
  let closeWatch: { order: Promise<string[]>; close: () => void } | null = null;
  try {
    daemon = await startHided(fixture, "close-subtree");
    const live = daemon;
    // target spawned child, which spawned grandchild; keeper spawned kept.
    // The fixture's first agent is the target and its own child, beside it, is the
    // child; the second root is the keeper, in a tab of its own.
    const [target, child] = fixture.panes;
    const grandchild = await delegate(fixture, live, child, "grandchild");
    const keeper = await spawnRoot(fixture, live, target, { name: "keeper", repo: path.join(fixture.root, "fixture") });
    await withControl(live, async (control) => viewPane(control, keeper));
    await agentReading(fixture, keeper);
    const kept = await delegate(fixture, live, keeper, "kept");
    await withControl(live, (control) => focusTab(control, fixture, fixture.tab));

    const last = new Map<string, Record<string, unknown>>();
    const sent = countSent(page, last);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    // The sheet reads the core's close lists, so wait until the lineage is in.
    await expect(page.locator(`nav[data-sidebar] [data-pane="${target}"] [data-tree-chevron="${target}"]`).first()).toBeVisible({ timeout: 30_000 });
    await expect(page.locator(`nav[data-sidebar] [data-pane="${keeper}"] [data-tree-chevron="${keeper}"]`).first()).toBeVisible({ timeout: 30_000 });

    const sheet = page.locator("[data-confirm-subtree]");
    await page.locator(`[data-terminal-host="${target}"]`).click();
    await page.keyboard.press(chord("close_tab"));
    await expect(sheet).toBeVisible();
    await expect(sheet.getByRole("heading")).toHaveText("Close this agent and 2 children?");
    await expect(sheet.locator("[data-subtree-row]")).toHaveCount(3);
    await expect(sheet.locator("[data-subtree-row]").first()).toHaveAttribute("data-subtree-row", target);
    await expect(sheet.locator("[data-subtree-row]").nth(1)).toHaveAttribute("data-subtree-row", child);
    await expect(sheet.locator("[data-subtree-row]").nth(2)).toHaveAttribute("data-subtree-row", grandchild);
    // No sentence under the title: what Close only leaves is its button's tooltip and description.
    await expect(sheet.locator("[data-subtree-close-only]")).toHaveAccessibleDescription("Children keep running and move into your list.");
    // Enter's default is Close all (D-18).
    await expect(sheet.locator("[data-subtree-close-all]")).toBeFocused();
    await screenshot(page, "close-subtree-sheet");

    // Escape sends nothing and leaves every pane as it was.
    await page.keyboard.press("Escape");
    await expect(sheet).toHaveCount(0);
    expect(sent.get("close_tree") ?? 0).toBe(0);
    expect(sent.get("close_pane") ?? 0).toBe(0);
    for (const pane of [target, child, grandchild]) expect(await listed(live, pane)).toBe(true);

    // Enter closes the subtree as one event, deepest first, target last.
    await page.locator(`[data-terminal-host="${target}"]`).click();
    await page.keyboard.press(chord("close_tab"));
    await expect(sheet.locator("[data-subtree-close-all]")).toBeFocused();
    closeWatch = await watchCloseOrder(live, [target, child, grandchild]);
    await page.keyboard.press("Enter");
    expect(await closeWatch.order).toEqual([grandchild, child, target]);
    expect(sent.get("close_tree")).toBe(1);
    expect(last.get("close_tree")).toMatchObject({ target: { kind: "pane", pane_id: target }, pane_ids: [grandchild, child], confirmed: true });
    for (const pane of [target, child, grandchild]) await expect(page.locator(`[data-pane="${pane}"]`)).toHaveCount(0, { timeout: 20_000 });
    await screenshot(page, "close-subtree-all-closed");

    // Close only closes just the target; its child becomes the operator's.
    // The keeper's tab is not the one the page shows, so it is shown first.
    await withControl(live, (control) => showTabOf(control, keeper));
    await page.locator(`[data-terminal-host="${keeper}"]`).click();
    await page.keyboard.press(chord("close_tab"));
    await expect(sheet.getByRole("heading")).toHaveText("Close this agent and 1 child?");
    await sheet.locator("[data-subtree-close-only]").click();
    await expect.poll(() => listed(live, keeper), { timeout: 20_000 }).toBe(false);
    expect(await listed(live, kept)).toBe(true);
    expect(sent.get("close_tree")).toBe(1);
    await expect(page.locator(`[data-pane="${kept}"]`)).toHaveAttribute("data-depth", "0", { timeout: 20_000 });
    await screenshot(page, "close-subtree-close-only");
  } finally {
    closeWatch?.close();
    daemon?.stop();
    fixture.stop();
  }
});

test("an open close sheet follows the snapshot: Stop-work tracks its pane, and a subtree sheet whose last child leaves turns back into it", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const fixture = await startFixture({ family: true });
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(fixture, "close-stop-work-live");
    const live = daemon;
    // The target and the keeper are two operator-owned agents in one tab.
    const [target, keeper] = fixture.panes;
    await releaseChild(fixture, live);
    await setAgentState(live, target, "working");
    const last = new Map<string, Record<string, unknown>>();
    const sent = countSent(page, last);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");

    const confirm = page.locator('[data-confirm-close="pane"]');
    const subtree = page.locator("[data-confirm-subtree]");
    const row = confirm.locator(`[data-stop-work-row="${target}"]`);
    const opacity = () => row.evaluate((node) => Number(getComputedStyle(node).opacity));
    // The working state has to be in before the close asks: a quiet pane would close at once.
    await expect(page.locator(`[data-pane="${target}"] [data-agent-status="working"]`)).toBeVisible({ timeout: 30_000 });
    await page.locator(`[data-terminal-host="${target}"]`).click();
    await page.keyboard.press(chord("close_tab"));
    await expect(row).toHaveAttribute("data-stop-work-state", "active");
    await expect(confirm.getByRole("heading")).toHaveText("Stop the active pane?");
    await expect(subtree).toHaveCount(0);
    expect(await opacity()).toBe(1);

    // The pane settles while the sheet is open: its row dims and says so, and
    // the sheet stays until the operator answers (B28, D-40).
    await setAgentState(live, target, "idle");
    await expect(row).toHaveAttribute("data-stop-work-state", "quiet", { timeout: 30_000 });
    expect(await opacity()).toBeLessThan(1);
    // It starts again: bright again, in place.
    await setAgentState(live, target, "working");
    await expect(row).toHaveAttribute("data-stop-work-state", "active", { timeout: 30_000 });
    expect(await opacity()).toBe(1);
    await screenshot(page, "close-stop-work-live");

    // A child spawned while it is open turns it into the subtree sheet in place.
    const child = await delegate(fixture, live, target, "child");
    await expect(subtree.locator(`[data-subtree-row="${child}"]`)).toBeVisible({ timeout: 30_000 });
    await expect(subtree.getByRole("heading")).toHaveText("Close this agent and 1 child?");
    // The keyboard stays on the sheet itself, not on the Enter default of the new one.
    await expect(subtree).toBeFocused();

    // The target settles, then its last child closes elsewhere: the sheet
    // turns back into the target's Stop-work sheet with the quiet target dimmed.
    await setAgentState(live, target, "idle");
    await withControl(live, (control) => closePane(control, child));
    await expect(subtree).toHaveCount(0, { timeout: 30_000 });
    await expect(confirm.getByRole("heading")).toHaveText("Stop the active pane?");
    await expect(row).toHaveAttribute("data-stop-work-state", "quiet", { timeout: 30_000 });
    expect(await opacity()).toBeLessThan(1);
    await expect(confirm).toBeFocused();
    await screenshot(page, "close-subtree-back-to-stop-work");
    expect(sent.get("close_pane") ?? 0).toBe(0);
    expect(sent.get("close_tree") ?? 0).toBe(0);

    // Stop work and close closes the target alone.
    await confirm.getByRole("button", { name: "Stop work and close" }).click();
    expect(sent.get("close_pane")).toBe(1);
    expect(last.get("close_pane")).toMatchObject({ pane_id: target, confirmed: true });
    await expect.poll(() => listed(live, target), { timeout: 20_000 }).toBe(false);
    expect(await listed(live, keeper)).toBe(true);
    await expect(confirm).toHaveCount(0);
  } finally {
    daemon?.stop();
    fixture.stop();
  }
});

function git(cwd: string, args: string[]): string {
  return execFileSync("git", ["-c", "user.name=e2e", "-c", "user.email=e2e@example.invalid", "-c", "init.defaultBranch=main", "-c", "commit.gpgsign=false", ...args], { cwd, encoding: "utf8" });
}

// @platform: Process ownership: which processes belong to a checkout, read through libproc on macOS and /proc on Linux.
test("Delete worktree closes the agents its checkout spawned outside it before the folder goes", { tag: "@platform" }, async ({ page }) => {
  skipOwnWindowsAgentStart();
  await page.setViewportSize({ width: 1400, height: 900 });
  const fixture = await startFixture({ family: true });
  let daemon: Daemon | null = null;
  try {
    const branch = "feature/spawner";
    const repo = path.join(fixture.root, "fixture");
    const worktree = path.join(fixture.root, "repo-spawner");
    // The folder the worktree's agent spawns one in.
    const outsideRepo = path.join(fixture.root, "outside");
    repository(outsideRepo);

    daemon = await startHided(fixture, "close-subtree-worktree");
    // The fixture folder is a Git repository once its daemon made the fixture.
    fs.writeFileSync(path.join(repo, "README.md"), "# repo\n");
    git(repo, ["add", "README.md"]);
    git(repo, ["commit", "-m", "initial"]);
    git(repo, ["update-ref", "refs/remotes/origin/main", "HEAD"]);
    git(repo, ["symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/main"]);
    git(repo, ["worktree", "add", "-b", branch, worktree]);
    const live = daemon;
    const last = new Map<string, Record<string, unknown>>();
    countSent(page, last);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    // The core lists a linked worktree once a page is connected and it has read the repository.
    const { spawner, outside } = await withControl(live, async (control) => {
      const opened = await openWorktree(control, worktree);
      await openProject(control, outsideRepo, "outside");
      const agent = await startAgentInProject(fixture, live, opened, { task: "워크트리 스포너" });
      await showTabOf(control, agent.pane);
      await agentReading(fixture, agent.pane);
      // The worktree's agent spawned one in another folder.
      return { spawner: agent.pane, outside: await delegate(fixture, live, agent.pane, "outside", undefined, outsideRepo) };
    });
    await expect(page.locator(`nav[data-sidebar] [data-pane="${spawner}"] [data-tree-chevron="${spawner}"]`).first()).toBeVisible({ timeout: 30_000 });

    const feature = page.locator("[data-checkout-row]").filter({ has: page.locator(`[data-checkout][aria-label^="${branch}"]`) });
    // The fixture branch has nothing ahead of main, yet no work of its own
    // has landed, so its row stays among the active checkouts rather than
    // folding under Inactive. Its commit age is drawn once the core has read
    // the worktree's Git facts, which the deletion dialog is built from.
    await expect(feature.locator("[data-checkout-age]")).toBeVisible({ timeout: 30_000 });
    await expect(page.locator("[data-inactive-checkouts]")).toHaveCount(0);
    await feature.locator("[data-checkout-menu]").click({ button: "right" });
    await page.getByRole("menu", { name: `${branch} actions` }).locator('[data-menu-item="delete_worktree"]').click();

    const dialog = page.locator("[data-delete-worktree]");
    await expect(dialog.locator("[data-removal-subtree]")).toContainText("1 agent outside this worktree", { timeout: 30_000 });
    // The core's warnings are badges, the path shows once.
    await expect(dialog.locator('[data-delete-warning="not_pushed"]')).toBeVisible();
    await expect(dialog.getByText(/repo-spawner/)).toHaveCount(1);
    await expect(dialog.locator(`[data-subtree-row="${outside}"]`)).toBeVisible();
    const withOutside = dialog.locator('[data-delete-confirm="with-outside"]');
    await expect(withOutside).toHaveText("Close 1 agent and delete");
    await expect(dialog.locator('[data-delete-confirm="only"]')).toHaveText("Delete only");
    // Neither action holds the keyboard when the dialog opens (D-35).
    await expect(withOutside).not.toBeFocused();
    await expect(dialog.locator('[data-delete-confirm="only"]')).not.toBeFocused();
    // The three result-named buttons stand on one row.
    expect(await rowTops([dialog.locator("[data-delete-cancel]"), dialog.locator('[data-delete-confirm="only"]'), withOutside])).toHaveLength(1);
    await screenshot(page, "close-subtree-delete-worktree");

    await withOutside.click();
    expect(last.get("remove_worktree")).toMatchObject({ close_descendant_pane_ids: [outside] });
    await expect(dialog.locator('[data-delete-result="finished"]')).toBeVisible({ timeout: 60_000 });
    expect(await listed(live, outside)).toBe(false);
    expect(await listed(live, spawner)).toBe(false);
    expect(fs.existsSync(worktree)).toBe(false);
  } finally {
    daemon?.stop();
    fixture.stop();
  }
});

test("Remove project closes the agents its panes spawned outside it before the project goes", async ({ page }) => {
  await page.setViewportSize({ width: 1400, height: 900 });
  const fixture = await startFixture({ family: true });
  let daemon: Daemon | null = null;
  try {
    // The fixture project's agent spawned one in a project of its own.
    const outsideRepo = path.join(fixture.root, "outside");
    repository(outsideRepo);

    daemon = await startHided(fixture, "close-subtree-project");
    const live = daemon;
    const [parent] = fixture.panes;
    const outside = await withControl(live, async (control) => {
      await openProject(control, outsideRepo, "outside");
      return delegate(fixture, live, parent, "outside", undefined, outsideRepo);
    });
    const last = new Map<string, Record<string, unknown>>();
    countSent(page, last);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    await expect(page.locator(`nav[data-sidebar] [data-pane="${parent}"] [data-tree-chevron="${parent}"]`).first()).toBeVisible({ timeout: 30_000 });

    // The project row's own menu removes the whole project.
    await page.getByRole("button", { name: "More actions for fixture", exact: true }).click();
    await page.getByRole("menu", { name: "More actions for fixture" }).locator('[data-menu-item="remove_project"]').click();

    const dialog = page.locator("[data-remove-project]");
    await expect(dialog.locator("[data-removal-subtree]")).toContainText("1 agent outside this project", { timeout: 30_000 });
    await expect(dialog.locator(`[data-subtree-row="${outside}"]`)).toBeVisible();
    const withOutside = dialog.locator('[data-remove-confirm="with-outside"]');
    await expect(withOutside).toHaveText("Close 1 agent and remove");
    await expect(dialog.locator('[data-remove-confirm="only"]')).toHaveText("Remove only");
    await expect(withOutside).not.toBeFocused();
    expect(await rowTops([dialog.locator("[data-remove-cancel]"), dialog.locator('[data-remove-confirm="only"]'), withOutside])).toHaveLength(1);
    await screenshot(page, "close-subtree-remove-project");

    await withOutside.click();
    expect(last.get("remove_workspace")).toMatchObject({ close_descendant_pane_ids: [outside] });
    await gone(live, [outside, ...fixture.panes]);
  } finally {
    daemon?.stop();
    fixture.stop();
  }
});
