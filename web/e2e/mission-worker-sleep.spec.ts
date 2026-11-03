// A Conductor's worker on either engine and an isolated hided (PRD
// mission-worker-sleep B1-B3, B5, B8, B14): `hide agent sleep` ends the
// worker in its pane, a report letter to it waits while a request letter
// wakes it into the same conversation and then rings, the operator's pane
// keeps the focus throughout, the worker's node finishing puts it to sleep
// again without the hours, and the request still pending does not wake it a
// second time. B12 (no sleep again after a wake) is the runtime test's.

import { expect, test } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import type { HideAnswer } from "./agent-pane";
import { agentIn, closePane, corePaneFocus, tabOfPane, withControl } from "./daemon-actions";
import { startFixture } from "./engine-fixture";
import { spawnChild } from "./fixture-agents";
import { claudeProjects, writeFixtureTranscript } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { enterWorkspace, screenshot } from "./wire";
import { chord } from "./chords";
import { quietFor } from "./wait";

test.describe.configure({ timeout: 300_000 });

type Hide = (args: string[]) => Promise<HideAnswer>;
type Answer = { ok: boolean; reason?: string; value?: Record<string, unknown>; result?: Record<string, unknown> };

/** The last JSON line one `hide` command printed. */
async function answer(hide: Hide, args: string[]): Promise<Answer> {
  const ran = await hide(args);
  const line = ran.out.trim().split("\n").at(-1) ?? "";
  if (!line) return { ok: ran.status === 0, reason: ran.status === 0 ? undefined : `exit ${ran.status}` };
  return JSON.parse(line) as Answer;
}

/** Every record of `kind` the daemon's diagnostic logs hold, wherever its folders keep them. */
function diagnostics(daemon: Daemon, kind: string): Record<string, unknown>[] {
  const found: Record<string, unknown>[] = [];
  const walk = (dir: string) => {
    if (!fs.existsSync(dir)) return;
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      const file = path.join(dir, entry.name);
      if (entry.isDirectory() && !entry.isSymbolicLink()) walk(file);
      else if (entry.isFile() && entry.name.endsWith(".jsonl")) {
        for (const line of fs.readFileSync(file, "utf8").split("\n")) {
          if (!line.includes(kind)) continue;
          try { found.push(JSON.parse(line) as Record<string, unknown>); } catch { /* a line being written */ }
        }
      }
    }
  };
  walk(daemon.stateDir);
  walk(daemon.home);
  return found;
}

async function ok(hide: Hide, args: string[]): Promise<Answer> {
  const got = await answer(hide, args);
  expect(got.ok, `${args.join(" ")}: ${JSON.stringify(got)}`).toBe(true);
  return got;
}

test("a finished worker sleeps, a request letter wakes it into the same conversation, and its node's finish sleeps it again", async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 1000 });
  const fixture = await startFixture({ family: true });
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(fixture, "mission-worker-sleep");
    const [lead, beside] = fixture.panes;
    const live = daemon;
    // The Conductor is alone in its project, as a mission's lead is: its one worker is spawned below.
    await withControl(live, (control) => closePane(control, beside));
    const conductor = fixture.runAs(lead);
    const repo = fs.realpathSync(path.join(fixture.root, "fixture"));
    const conductorId = ((await ok(conductor, ["agent", "show", "here"])).value as { id: string }).id;

    // The conversation file a real Claude writes; the bell reads its turns before it rings.
    // Written now, so turning sleep on later does not find it hours idle.
    const session = `worker-session-${process.pid}`;
    writeFixtureTranscript(claudeProjects(fixture), session, { task: "Review fix worker", at: Date.now() });
    // The worker is the Conductor's child, in a tab of its own.
    const workerPane = await spawnChild(fixture, live, lead, { name: "worker", repo, branch: "worker-task", session });
    const listed = (await ok(conductor, ["agent", "list"])).value as { items: { id: string; pane: string }[] };
    const worker = { id: listed.items.find((agent) => agent.pane === workerPane)!.id, pane: workerPane };
    await withControl(live, async (control) => {
      const tabs = await control.until((rest) => {
        const [leadTab, workerTab] = [tabOfPane(rest, lead), tabOfPane(rest, worker.pane)];
        return leadTab && workerTab ? { leadTab, workerTab } : undefined;
      }, "the Conductor's and the worker's tabs", 20_000);
      expect(tabs.workerTab).not.toBe(tabs.leadTab);
    });
    // Whether Hide holds the worker asleep, as its registration reads: an asleep agent no longer runs, so its pane belongs to its shell.
    const lifecycle = async () => ((await answer(conductor, ["agent", "show", worker.id])).value as { state?: { lifecycle?: string } } | undefined)?.state?.lifecycle;
    const asleep = async () => (await lifecycle()) === "asleep";
    const sessionNow = () => withControl(live, (control) => control.until((rest) => agentIn(rest, worker.pane)?.session_id ?? undefined, `the session of ${worker.pane}`, 20_000));

    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    const leadView = page.locator(`[data-pane-view="${lead}"]`);
    await expect(leadView).toBeVisible({ timeout: 20_000 });

    // Control: an awake worker's request rings in this fixture.
    const show = async (id: string) => JSON.stringify(await answer(conductor, ["request", "show", id]));
    const holds = (id: string) => JSON.stringify(diagnostics(live, "doorbell.held").filter((record) => record.letter_id === id).slice(-6));
    const control = await ok(conductor, ["request", "send", worker.id, "--intent", "control", "--kind", "request", "--body", "깨어 있는 동안의 확인"]);
    const controlId = ((control.value ?? control.result) as { id: string }).id;
    await expect.poll(() => show(controlId), { message: `an awake worker's request rings; holds ${holds(controlId)}`, timeout: 40_000 })
      .toMatch(/"bell_sent":true|"state":"(delivered|acknowledged)"/);
    // The fixture runs no hook to confirm it; settle it so no pending request is left to wake for.
    await ok(conductor, ["request", "cancel", controlId]);

    // B14: the parent puts its worker to sleep from the CLI; again answers asleep.
    await expect.poll(async () => JSON.stringify(await answer(conductor, ["agent", "sleep", worker.id])), { timeout: 30_000 }).toMatch(/"state":"(ending|asleep)"/);
    await expect.poll(asleep, { message: "the worker's shell holds its pane", timeout: 30_000 }).toBe(true);
    await expect.poll(async () => JSON.stringify(await answer(conductor, ["agent", "sleep", worker.id])), { timeout: 20_000 }).toContain('"state":"asleep"');
    await screenshot(page, "mission-worker-sleep-cli-slept");

    // B1, B3: a report to the sleeper is accepted, waits, and wakes no one.
    const report = await ok(conductor, ["request", "send", worker.id, "--intent", "status-report", "--kind", "report", "--body", "참고: 리뷰가 시작됐어요"]);
    expect(JSON.stringify(report)).toContain('"state":"pending"');
    const reportId = ((report.value ?? report.result) as { id: string }).id;
    const sleepRecords = () => JSON.stringify(diagnostics(live, "agent_sleep.").slice(-12));
    await quietFor(page, 8_000, "a report letter starts no wake; a request starts one within a tick");
    expect(await asleep(), `a report wakes no one; ${sleepRecords()}`).toBe(true);

    // B2, B5: a request wakes it into the same conversation; the operator's pane keeps the focus.
    const request = await ok(conductor, ["request", "send", worker.id, "--intent", "review-fix", "--kind", "request", "--body", "리뷰 지적 하나를 고쳐 주세요"]);
    const requestId = ((request.value ?? request.result) as { id: string }).id;
    await expect.poll(asleep, { message: "the request wakes the worker", timeout: 60_000 }).toBe(false);
    // The wake was the letter's, resuming the conversation rather than starting a new one.
    const wakes = () => diagnostics(live, '"agent_sleep.waking"').filter((record) => record.pane_id === worker.pane);
    const letterWakes = () => wakes().filter((record) => record.trigger === "letter").length;
    await expect.poll(letterWakes, { timeout: 20_000 }).toBe(1);
    expect(wakes().every((record) => record.mode === "resume"), JSON.stringify(wakes())).toBe(true);
    expect(await sessionNow(), "the worker resumed its own conversation").toBe(session);
    await expect(leadView).toHaveAttribute("data-focused", "true");
    await withControl(live, (control) => control.until((rest) => corePaneFocus(rest) === lead || undefined, "the Conductor's pane holding the focus", 20_000));
    // One bell rings for the worker's turn: the oldest letter's, which counts the request with it.
    await expect.poll(async () => {
      const [reportShown, requestShown] = [await show(reportId), await show(requestId)];
      return /"bell_sent":true/.test(requestShown) || (/"bell_sent":true/.test(reportShown) && /and 1 more/.test(reportShown));
    }, { message: `the woken worker's bell carries the request; holds ${holds(requestId)}`, timeout: 70_000 }).toBe(true);
    await screenshot(page, "mission-worker-sleep-woken");

    // B8, B12: with sleep turned on, the worker's node finishing sleeps it without the hours.
    await page.keyboard.press(chord("settings"));
    await page.locator('[data-settings-tab="agents"]').click();
    const choice = page.locator("[data-agent-sleep-after]");
    await choice.click();
    await page.locator('[data-agent-sleep-option="72"]').click();
    await expect(choice).toHaveAttribute("data-agent-sleep-after", "72", { timeout: 10_000 });
    await page.keyboard.press("Escape");
    await ok(conductor, ["mission", "create", "--title", "워커 재우기 확인"]);
    await ok(conductor, ["mission", "plan", "--json", JSON.stringify({ scope: "pr", nodes: [{ key: "fix", title: "리뷰 수정", kind: "change", delivery: "pr", criteria: ["지적이 고쳐졌다"] }] })]);
    await ok(conductor, ["mission", "start"]);
    await ok(conductor, ["mission", "node", "fix", "--status", "working", "--agent-id", worker.id, "--line", "고치는 중"]);
    await quietFor(page, 5_000, "a working node marks no one; a finish marks its worker at once");
    expect(await asleep(), `a working node sleeps no one; ${sleepRecords()}`).toBe(false);
    await ok(conductor, ["mission", "node", "fix", "--status", "dropped"]);
    await expect.poll(asleep, { message: `the finished node's worker sleeps within the minute pass; ${sleepRecords()}`, timeout: 150_000, intervals: [2_000] }).toBe(true);
    const missionSleeps = diagnostics(live, '"agent_sleep.ending"').filter((record) => record.pane_id === worker.pane && record.trigger === "mission");
    expect(missionSleeps.length, `the node's finish put it to sleep; ${sleepRecords()}`).toBeGreaterThan(0);
    // The request still pending woke it once already, so it does not wake it again.
    expect(await show(requestId), "the request is still pending, so only the once rule keeps the worker asleep").toContain('"state":"pending"');
    await quietFor(page, 10_000, "a pending letter's wake starts within a tick of the sleep landing");
    expect(await asleep(), `a letter wakes its recipient once; ${sleepRecords()}`).toBe(true);
    expect(letterWakes()).toBe(1);
    // The Conductor itself never sleeps for its mission.
    await withControl(live, (control) => control.until((rest) => agentIn(rest, lead) || undefined, "the Conductor's row", 20_000));
    expect(conductorId).toBeTruthy();
    await expect(leadView).toHaveAttribute("data-focused", "true");
    await screenshot(page, "mission-worker-sleep-node-done");
  } finally {
    daemon?.stop();
    fixture.stop();
  }
});
