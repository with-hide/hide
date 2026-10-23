// Closing an agent that spawned others (PRD close-agent-subtree): the close
// sheet lists the descendants, Escape sends nothing, Enter closes the whole
// subtree deepest first with the target last, and Close only leaves the
// children running as the operator's own rows. Delete worktree asks the same
// question about the agents its checkout spawned outside it.

import { expect, test, type Locator } from "@playwright/test";
import { execFile, execFileSync } from "node:child_process";
import fs from "node:fs";
import net from "node:net";
import path from "node:path";
import { promisify } from "node:util";
import { finishFixtureTurn, labelAgent, setFixtureLifecycle, spawnAgent, startHerdr, type FixtureLabel, type HerdrFixture } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { localEndpoint } from "./platform-fixture";
import { countSent, enterWorkspace, screenshot } from "./wire";
import { chord } from "./chords";

test.describe.configure({ timeout: 180_000 });
test.use({ actionTimeout: 15_000 });

const run = promisify(execFile);

/** Every pane id the private Herdr lists right now. */
async function livePanes(herdr: HerdrFixture): Promise<Set<string>> {
  const { stdout } = await run(herdr.bin, ["pane", "list"], { env: herdr.env, encoding: "utf8", timeout: 10_000 });
  const ids = new Set<string>();
  const visit = (value: unknown): void => {
    if (Array.isArray(value)) value.forEach(visit);
    else if (value && typeof value === "object") {
      const row = value as Record<string, unknown>;
      if (typeof row.pane_id === "string") ids.add(row.pane_id);
      Object.values(row).forEach(visit);
    }
  };
  visit(JSON.parse(stdout));
  return ids;
}

/**
 * Records the order Herdr closes the panes in `watched`, from its own
 * `pane.closed` events. Polling `pane list` from a subprocess was slower than
 * the core's ~140 ms steps on a loaded runner and saw two panes go in one
 * look (CI, 2026-10-01); the events carry the order itself. Resolves once
 * the subscription is live, so nothing the caller does next is missed; the
 * caller closes it on every exit path.
 */
async function watchCloseOrder(herdr: HerdrFixture, watched: string[]): Promise<{ order: Promise<string[]>; close: () => void }> {
  const socket = net.createConnection(localEndpoint(herdr.socket));
  const order: string[] = [];
  let buffer = "";
  let ack: (error?: Error) => void = () => undefined;
  let settle: (error?: Error) => void = () => undefined;
  const live = new Promise<void>((resolve, reject) => {
    ack = (error) => (error ? reject(error) : resolve());
  });
  const done = new Promise<string[]>((resolve, reject) => {
    settle = (error) => {
      clearTimeout(timer);
      socket.destroy();
      if (error) reject(error);
      else resolve(order);
    };
  });
  // A failure before the caller awaits `done` still reaches it; this only
  // keeps the runner from reporting the same rejection as unhandled.
  done.catch(() => undefined);
  const fail = (error: Error) => {
    ack(error);
    settle(error);
  };
  const timer = setTimeout(() => fail(new Error(`closed so far: ${order.join(", ") || "none"}`)), 60_000);
  socket.on("error", fail);
  // Herdr ending the stream before every pane closed is a failure, not a wait.
  socket.on("close", () => fail(new Error(`Herdr closed the event stream; closed so far: ${order.join(", ") || "none"}`)));
  socket.on("data", (chunk) => {
    buffer += chunk.toString("utf8");
    for (let newline = buffer.indexOf("\n"); newline >= 0; newline = buffer.indexOf("\n")) {
      const text = buffer.slice(0, newline).trim();
      buffer = buffer.slice(newline + 1);
      if (!text) continue;
      const line = JSON.parse(text) as { id?: string; error?: unknown; data?: { type?: string; pane_id?: string } };
      if (line.id === "close-order") {
        ack(line.error ? new Error(`events.subscribe refused: ${JSON.stringify(line.error)}`) : undefined);
        continue;
      }
      const pane = line.data?.type === "pane_closed" ? line.data.pane_id : undefined;
      if (pane && watched.includes(pane) && !order.includes(pane)) order.push(pane);
      if (order.length === watched.length) settle();
    }
  });
  socket.write(`${JSON.stringify({ id: "close-order", method: "events.subscribe", params: { subscriptions: [{ type: "pane.closed" }] } })}\n`);
  await live;
  return { order: done, close: () => fail(new Error("closed by the test")) };
}

/** The tops of `buttons`, rounded: one value means they share one row. */
async function rowTops(buttons: Locator[]): Promise<number[]> {
  const tops = new Set<number>();
  for (const button of buttons) tops.add(Math.round((await button.boundingBox())!.y));
  return [...tops];
}

test("the close sheet counts and brightens what needs the operator, stays live while open, and closes what it shows", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const herdr = await startHerdr();
  let daemon: Daemon | null = null;
  try {
    const target = herdr.panes[1];
    const spawn = (name: string, task: FixtureLabel) => spawnAgent(herdr, name, target, undefined, task);
    const working = await spawn("working", { task: "계보 투영 구현" });
    const asking = await spawn("asking", { task: "병합 전 검증 실행", reply: "병합 전에 검증을 다시 돌릴까요?", question: true });
    const finished = await spawn("finished", { task: "릴리스 노트 초안" });
    const quiet = await spawn("quiet", { task: "로그 파일 정리 작업" });
    await setFixtureLifecycle(herdr, working, "working");
    // Its own workspace is not the one Herdr's clients show.
    await finishFixtureTurn(herdr, finished, herdr.tab);

    daemon = await startHided(herdr, "close-subtree-states");
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
    const late = await spawnAgent(herdr, "late", target);
    await expect(sheet.locator(`[data-subtree-row="${late}"]`)).toBeVisible({ timeout: 30_000 });
    await expect(sheet.getByRole("heading")).toHaveText("Close this agent and 5 children?");
    herdr.run(["pane", "close", quiet]);
    await expect(sheet.locator(`[data-subtree-row="${quiet}"]`)).toHaveCount(0, { timeout: 30_000 });
    await expect(sheet.getByRole("heading")).toHaveText("Close this agent and 4 children?");
    expect(sent.get("close_tree") ?? 0).toBe(0);

    // Enter closes exactly what the sheet shows at the press.
    await expect(sheet.locator("[data-subtree-close-all]")).toBeFocused();
    await page.keyboard.press("Enter");
    expect(sent.get("close_tree")).toBe(1);
    expect(new Set(last.get("close_tree")?.pane_ids as string[])).toEqual(new Set([working, asking, finished, late]));
    await expect.poll(async () => {
      const live = await livePanes(herdr);
      return [target, working, asking, finished, late].filter((pane) => live.has(pane));
    }, { timeout: 60_000 }).toEqual([]);
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});

test("the close sheet closes the whole subtree deepest first, and Close only keeps the children", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const herdr = await startHerdr();
  let daemon: Daemon | null = null;
  let closeWatch: { order: Promise<string[]>; close: () => void } | null = null;
  try {
    const [keeper, target] = herdr.panes;
    // target spawned child, which spawned grandchild; keeper spawned kept.
    const child = await spawnAgent(herdr, "child", target);
    const grandchild = await spawnAgent(herdr, "grandchild", child);
    const kept = await spawnAgent(herdr, "kept", keeper);

    daemon = await startHided(herdr, "close-subtree");
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
    const before = await livePanes(herdr);
    for (const pane of [target, child, grandchild]) expect(before.has(pane)).toBe(true);

    // Enter closes the subtree as one event, deepest first, target last.
    await page.locator(`[data-terminal-host="${target}"]`).click();
    await page.keyboard.press(chord("close_tab"));
    await expect(sheet.locator("[data-subtree-close-all]")).toBeFocused();
    closeWatch = await watchCloseOrder(herdr, [target, child, grandchild]);
    await page.keyboard.press("Enter");
    expect(await closeWatch.order).toEqual([grandchild, child, target]);
    expect(sent.get("close_tree")).toBe(1);
    expect(last.get("close_tree")).toMatchObject({ target: { kind: "pane", pane_id: target }, pane_ids: [grandchild, child], confirmed: true });
    for (const pane of [target, child, grandchild]) await expect(page.locator(`[data-pane="${pane}"]`)).toHaveCount(0, { timeout: 20_000 });
    await screenshot(page, "close-subtree-all-closed");

    // Close only closes just the target; its child becomes the operator's.
    await page.locator(`[data-terminal-host="${keeper}"]`).click();
    await page.keyboard.press(chord("close_tab"));
    await expect(sheet.getByRole("heading")).toHaveText("Close this agent and 1 child?");
    await sheet.locator("[data-subtree-close-only]").click();
    await expect.poll(async () => (await livePanes(herdr)).has(keeper), { timeout: 20_000 }).toBe(false);
    expect((await livePanes(herdr)).has(kept)).toBe(true);
    expect(sent.get("close_tree")).toBe(1);
    await expect(page.locator(`[data-pane="${kept}"]`)).toHaveAttribute("data-depth", "0", { timeout: 20_000 });
    await screenshot(page, "close-subtree-close-only");
  } finally {
    closeWatch?.close();
    daemon?.stop();
    herdr.stop();
  }
});

test("an open close sheet follows the snapshot: Stop-work tracks its pane, and a subtree sheet whose last child leaves turns back into it", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const herdr = await startHerdr();
  let daemon: Daemon | null = null;
  try {
    const [keeper, target] = herdr.panes;
    labelAgent(herdr, target, { task: "계보 투영 구현" });
    await setFixtureLifecycle(herdr, target, "working");
    daemon = await startHided(herdr, "close-stop-work-live");
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
    await setFixtureLifecycle(herdr, target, "idle");
    await expect(row).toHaveAttribute("data-stop-work-state", "quiet", { timeout: 30_000 });
    expect(await opacity()).toBeLessThan(1);
    // It starts again: bright again, in place.
    await setFixtureLifecycle(herdr, target, "working");
    await expect(row).toHaveAttribute("data-stop-work-state", "active", { timeout: 30_000 });
    expect(await opacity()).toBe(1);
    await screenshot(page, "close-stop-work-live");

    // A child spawned while it is open turns it into the subtree sheet in place.
    const child = await spawnAgent(herdr, "child", target);
    await expect(subtree.locator(`[data-subtree-row="${child}"]`)).toBeVisible({ timeout: 30_000 });
    await expect(subtree.getByRole("heading")).toHaveText("Close this agent and 1 child?");
    // The keyboard stays on the sheet itself, not on the Enter default of the new one.
    await expect(subtree).toBeFocused();

    // The target settles, then its last child closes elsewhere: the sheet
    // turns back into the target's Stop-work sheet with the quiet target dimmed.
    await setFixtureLifecycle(herdr, target, "idle");
    herdr.run(["pane", "close", child]);
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
    await expect.poll(async () => (await livePanes(herdr)).has(target), { timeout: 20_000 }).toBe(false);
    expect((await livePanes(herdr)).has(keeper)).toBe(true);
    await expect(confirm).toHaveCount(0);
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});

function git(cwd: string, args: string[]): string {
  return execFileSync("git", ["-c", "user.name=e2e", "-c", "user.email=e2e@example.invalid", "-c", "init.defaultBranch=main", "-c", "commit.gpgsign=false", ...args], { cwd, encoding: "utf8" });
}

// @platform: Process ownership: which processes belong to a checkout, read through libproc on macOS and /proc on Linux.
test("Delete worktree closes the agents its checkout spawned outside it before the folder goes", { tag: "@platform" }, async ({ page }) => {
  await page.setViewportSize({ width: 1400, height: 900 });
  const herdr = await startHerdr();
  let daemon: Daemon | null = null;
  try {
    const branch = "feature/spawner";
    const repo = path.join(herdr.root, "repo");
    fs.mkdirSync(repo);
    git(repo, ["init"]);
    fs.writeFileSync(path.join(repo, "README.md"), "# repo\n");
    git(repo, ["add", "README.md"]);
    git(repo, ["commit", "-m", "initial"]);
    git(repo, ["update-ref", "refs/remotes/origin/main", "HEAD"]);
    git(repo, ["symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/main"]);
    const worktree = path.join(herdr.root, "repo-spawner");
    git(repo, ["worktree", "add", "-b", branch, worktree]);
    // The worktree's agent spawned one in another folder.
    const spawner = await spawnAgent(herdr, "spawner", null, worktree);
    const outside = await spawnAgent(herdr, "outside", spawner);

    daemon = await startHided(herdr, "close-subtree-worktree");
    const last = new Map<string, Record<string, unknown>>();
    countSent(page, last);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await expect(page.locator("[data-main-screen]")).toBeVisible({ timeout: 20_000 });
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
    const live = await livePanes(herdr);
    expect(live.has(outside)).toBe(false);
    expect(live.has(spawner)).toBe(false);
    expect(fs.existsSync(worktree)).toBe(false);
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});

test("Remove project closes the agents its panes spawned outside it before the project goes", async ({ page }) => {
  await page.setViewportSize({ width: 1400, height: 900 });
  const herdr = await startHerdr();
  let daemon: Daemon | null = null;
  try {
    // The fixture project's second agent spawned one in a folder of its own.
    const outside = await spawnAgent(herdr, "outside", herdr.panes[1]);

    daemon = await startHided(herdr, "close-subtree-project");
    const last = new Map<string, Record<string, unknown>>();
    countSent(page, last);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await expect(page.locator("[data-main-screen]")).toBeVisible({ timeout: 20_000 });
    await expect(page.locator(`nav[data-sidebar] [data-pane="${herdr.panes[1]}"] [data-tree-chevron="${herdr.panes[1]}"]`).first()).toBeVisible({ timeout: 30_000 });

    // A plain folder's row: its menu target is the row itself.
    await page.locator("[data-project-menu]").filter({ has: page.getByRole("button", { name: /^fixture,/ }) }).first().click({ button: "right" });
    await page.getByRole("menu", { name: "fixture actions" }).locator('[data-menu-item="remove_project"]').click();

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
    await expect.poll(async () => {
      const live = await livePanes(herdr);
      return [outside, ...herdr.panes].filter((pane) => live.has(pane));
    }, { timeout: 60_000 }).toEqual([]);
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});
