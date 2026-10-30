// A Conductor's worker on an isolated pinned Herdr and hided (PRD
// mission-worker-sleep B1-B3, B5, B8, B12, B14): `hide agent sleep` ends the
// worker in its pane, a report letter to it waits while a request letter
// wakes it into the same conversation and then rings, the operator's pane
// keeps the focus throughout, and the worker's node finishing puts it to
// sleep again without the hours.

import { expect, test } from "@playwright/test";
import { spawnSync } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { agentPane, type HideAnswer } from "./agent-pane";
import { claudeProjects, herdrHasFocus, startHerdr, writeFixtureTranscript } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { enterWorkspace, screenshot } from "./wire";
import { chord } from "./chords";

test.describe.configure({ timeout: 300_000 });

type Hide = (args: string[]) => Promise<HideAnswer>;
type Answer = { ok: boolean; reason?: string; value?: Record<string, unknown>; result?: Record<string, unknown> };
type ProcessInfo = {
  result: { process_info: { shell_pid: number; foreground_process_group_id: number; foreground_processes: { argv?: string[] | null }[] } };
};

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
  const herdr = await startHerdr({ agents: false });
  let daemon: Daemon | null = null;
  try {
    const repoDir = path.join(herdr.root, "missions-repo");
    fs.mkdirSync(repoDir);
    const repo = fs.realpathSync(repoDir);
    for (const args of [["init", "-q", "-b", "main"], ["-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "-q", "--allow-empty", "-m", "baseline"]]) {
      const git = spawnSync("git", ["-C", repo, ...args], { encoding: "utf8" });
      expect(git.status, git.stderr).toBe(0);
    }
    const created = herdr.run(["workspace", "create", "--cwd", repo, "--label", "missions-repo", "--env", `PATH=${herdr.fixturePath}`, "--focus"]) as { result: { root_pane: { pane_id: string } } };
    daemon = await startHided(herdr, "mission-worker-sleep");
    const lead = created.result.root_pane.pane_id;
    const conductor = await agentPane(herdr, daemon.stateDir, lead);
    const snapshotAgents = () => (herdr.run(["api", "snapshot"]) as { result: { snapshot: { agents: { pane_id: string; terminal_id: string; agent_session?: { value: string } }[] } } }).result.snapshot.agents;
    const listed = snapshotAgents().find((agent) => agent.pane_id === lead)!;
    const self = await ok(conductor, ["agent", "register", "--host-scope", herdr.socket, "--session", listed.agent_session!.value, "--instance", listed.terminal_id, "--name", "conductor", "--pane", lead]);
    const conductorId = (self.value as { id: string }).id;

    // The worker is the Conductor's child, in a tab of its own.
    const spawned = await ok(conductor, ["agent", "spawn", "--parent", "here", "--name", "worker", "--intent", "worker", "--kind", "claude", "--repo", repo, "--branch", "worker-task"]);
    const worker = spawned.value as { id: string; pane: string; session: string };
    expect(worker.pane.split(":")[0]).not.toBe(lead.split(":")[0]);
    const nativeSession = () => snapshotAgents().find((agent) => agent.pane_id === worker.pane)?.agent_session?.value ?? null;
    await expect.poll(nativeSession, { timeout: 20_000 }).toBeTruthy();
    const session = nativeSession()!;
    // The conversation file a real Claude writes; the bell reads its turns before it rings.
    writeFixtureTranscript(claudeProjects(herdr), session, { task: "Review fix worker" });
    const processInfo = () => (herdr.run(["pane", "process-info", "--pane", worker.pane]) as ProcessInfo).result.process_info;
    const asleep = () => { const info = processInfo(); return info.foreground_process_group_id === info.shell_pid; };

    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "missions-repo");
    const leadView = page.locator(`[data-pane-view="${lead}"]`);
    await expect(leadView).toBeVisible({ timeout: 20_000 });

    // Control: an awake worker's request rings in this fixture.
    const control = await ok(conductor, ["request", "send", worker.id, "--intent", "control", "--kind", "request", "--body", "깨어 있는 동안의 확인"]);
    const controlId = ((control.value ?? control.result) as { id: string }).id;
    try {
      await expect.poll(async () => JSON.stringify(await answer(conductor, ["request", "show", controlId])), { message: "an awake worker's request rings", timeout: 40_000 })
        .toMatch(/"bell_sent":true|"state":"(delivered|acknowledged)"/);
      // The fixture runs no hook to confirm it; settle it so no pending request is left to wake for.
      await ok(conductor, ["request", "cancel", controlId]);
    } catch (error) {
      console.log("DEBUG control held", JSON.stringify(diagnostics(daemon!, "doorbell.held").slice(-6)));
      throw error;
    }

    // B14: the parent puts its worker to sleep from the CLI; again answers asleep.
    await expect.poll(async () => JSON.stringify(await answer(conductor, ["agent", "sleep", worker.id])), { timeout: 30_000 }).toMatch(/"state":"(ending|asleep)"/);
    await expect.poll(asleep, { message: "the worker's shell holds its pane", timeout: 30_000 }).toBe(true);
    await expect.poll(async () => JSON.stringify(await answer(conductor, ["agent", "sleep", worker.id])), { timeout: 20_000 }).toContain('"state":"asleep"');
    await screenshot(page, "mission-worker-sleep-cli-slept");

    // B1, B3: a report to the sleeper is accepted, waits, and wakes no one.
    const report = await ok(conductor, ["request", "send", worker.id, "--intent", "status-report", "--kind", "report", "--body", "참고: 리뷰가 시작됐어요"]);
    expect(JSON.stringify(report)).toContain('"state":"pending"');
    await page.waitForTimeout(8_000);
    if (!asleep()) console.log("DEBUG report woke", JSON.stringify(diagnostics(daemon!, "agent_sleep.").slice(-12)));
    expect(asleep(), "a report wakes no one").toBe(true);

    // B2, B5: a request wakes it into the same conversation; the operator's pane keeps the focus.
    const request = await ok(conductor, ["request", "send", worker.id, "--intent", "review-fix", "--kind", "request", "--body", "리뷰 지적 하나를 고쳐 주세요"]);
    const requestId = ((request.value ?? request.result) as { id: string }).id;
    await expect.poll(asleep, { message: "the request wakes the worker", timeout: 60_000 }).toBe(false);
    // The wake was the letter's, resuming the conversation rather than starting a new one.
    const wakes = () => diagnostics(daemon!, '"agent_sleep.waking"').filter((record) => JSON.stringify(record).includes(worker.pane));
    await expect.poll(() => JSON.stringify(wakes()), { timeout: 20_000 }).toContain('"trigger":"letter"');
    expect(wakes().every((record) => JSON.stringify(record).includes('"mode":"resume"')), JSON.stringify(wakes())).toBe(true);
    await expect(leadView).toHaveAttribute("data-focused", "true");
    expect(herdrHasFocus(herdr, lead)).toBe(true);
    let rang = false;
    try {
      await expect.poll(async () => JSON.stringify(await answer(conductor, ["request", "show", requestId])), { message: "the request rings once the worker is back", timeout: 70_000 })
        .toMatch(/"bell_sent":true|"state":"(delivered|acknowledged)"/);
      rang = true;
    } catch {
      const sha = (value: string) => crypto.createHash("sha256").update(value).digest("hex");
      const now = snapshotAgents().find((agent) => agent.pane_id === worker.pane)?.agent_session?.value ?? "";
      const shown = JSON.stringify(await answer(conductor, ["request", "show", requestId]));
      console.log("DEBUG sessions", JSON.stringify({ original: session, originalSha: sha(session), herdrNow: now, herdrNowSha: sha(now), recipient: /"recipient":\{[^}]*\}/.exec(shown)?.[0] }));
      console.log("DEBUG ps", JSON.stringify(await answer(conductor, ["ps", "--json"])).slice(0, 2500));
      console.log("DEBUG woken request never rang; last holds", JSON.stringify(diagnostics(daemon!, "doorbell.held").filter((record) => JSON.stringify(record).includes(requestId))));
    }
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
    await page.waitForTimeout(5_000);
    expect(asleep(), "a working node sleeps no one").toBe(false);
    console.log("DEBUG show", JSON.stringify(await answer(conductor, ["mission", "show"])).slice(0, 1500));
    const finished = await answer(conductor, ["mission", "node", "fix", "--status", "dropped"]);
    console.log("DEBUG dropped", JSON.stringify(finished));
    expect(finished.ok, JSON.stringify(finished)).toBe(true);
    await expect.poll(asleep, { message: "the finished node's worker sleeps within the minute pass", timeout: 150_000, intervals: [2_000] }).toBe(true)
      .catch((error) => { console.log("DEBUG mission sleep", JSON.stringify(diagnostics(daemon!, "agent_sleep.").slice(-8))); throw error; });
    // The Conductor itself never sleeps for its mission.
    expect(snapshotAgents().some((agent) => agent.pane_id === lead)).toBe(true);
    expect(conductorId).toBeTruthy();
    await expect(leadView).toHaveAttribute("data-focused", "true");
    await screenshot(page, "mission-worker-sleep-node-done");
    expect(rang, "the woken worker's request rang").toBe(true);
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});
