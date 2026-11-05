// Synthetic native-format writers on an isolated engine and hided.
// Installed CLI measurements remain a separate acceptance boundary.
import { expect, test, type Locator, type Page } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { startFixture, type AgentFixture } from "./engine-fixture";
import { agentIn, sendKeys, tabOfPane, withControl } from "./daemon-actions";
import type { Rest } from "./hided-control.mjs";
import { startHided, type Daemon } from "./hided-fixture";
import crypto from "node:crypto";
import {
  GROK_ID, GROK_PLAN, GROK_TITLE, OMP_ID, OMP_TITLE, OMP_UPDATED_TITLE, OPENCODE_CHILD_TITLE, OPENCODE_ID, OPENCODE_TITLE, PI_ID,
  PI_TITLE, type NativeFileReader, appendOmpQuestion, nativeSessionId, prepareEngineWriter, prepareOpenCodeWriter, reportOpenCodeState, setGrokPlanApproval, updateOmpTitle,
  writeOpenCodeQuestion,
} from "./session-reader-fixture";
import { enterWorkspace, openSleepingProject, screenshot } from "./wire";
import { afterCleanup } from "./worker-owned";
import { dumpOnFailure } from "./failure-dump";
import type { AgentRow } from "../src/snapshot";

type QuestionRow = AgentRow & { user_turn?: { kind: string; content: { text: string; choices: string[]; truncated: boolean } | null } };

test.describe.configure({ timeout: 150_000 });

/** The program a native writer runs as: Cursor's CLI is `cursor-agent`. */
const programOf = (kind: NativeFileReader) => (kind === "cursor" ? "cursor-agent" : kind);

/** The sidebar agent row of `kind` in `pane`, once the engine lists it. */
function nativeAgent(rest: Rest, pane: string, kind: string): true | undefined {
  return agentIn(rest, pane)?.agent_kind === kind ? true : undefined;
}

/** A fresh pane (not `except`) the sidebar lists a `kind` agent in. */
function freshAgent(rest: Rest, kind: string, except: string): string | undefined {
  return ((rest.navigator as { agents?: AgentRow[] } | undefined)?.agents ?? []).find((row) => row.agent_kind === kind && row.pane_id !== except)?.pane_id;
}

/**
 * Starts the stand-in `kind` at the shell prompt of `pane`, as the operator
 * starts one, and waits until its session file exists, the engine lists the
 * agent and the agent's row names its session. The session reaches Hide as
 * the agent's own report: the stand-in's, from inside its pane
 * (`prepareEngineWriter`).
 */
async function startNative(fixture: AgentFixture, daemon: Daemon, kind: NativeFileReader, pane: string, session: string): Promise<void> {
  await withControl(daemon, async (control) => sendKeys(control, pane, `${programOf(kind)}\r`));
  await expect.poll(() => fs.existsSync(session)).toBe(true);
  await withControl(daemon, (control) => control.until((rest) => nativeAgent(rest, pane, kind), `a ${kind} agent in ${pane}`, 30_000));
  await reportSession(daemon, kind, pane);
}

/**
 * Waits until the row of `pane` names the agent's session, which the
 * stand-in reported when it wrote or resumed its session.
 */
async function reportSession(daemon: Daemon, kind: NativeFileReader, pane: string): Promise<void> {
  await withControl(daemon, (control) => control.until((rest) => (agentIn(rest, pane)?.session_id === nativeSessionId(kind) ? true : undefined), `${pane} on its ${kind} session`, 30_000));
}

/** Waits until the daemon publishes no layout holding `pane`: its agent's pane was closed. */
async function paneGone(daemon: Daemon, pane: string): Promise<void> {
  await withControl(daemon, (control) => control.until((rest) => (tabOfPane(rest, pane) === undefined ? true : undefined), `${pane} closed`, 30_000));
}

/** Quiet fixture agents can fold before the page first reads their session. */
async function showNativeRow(page: Page, pane: string): Promise<Locator> {
  const row = page.locator(`nav[data-sidebar] [data-checkout-agents-open] [data-pane="${pane}"]`);
  const folded = page.locator('nav[data-sidebar] [data-checkout-toggle][aria-expanded="false"]');
  await expect(row.or(folded)).toBeVisible();
  if (!(await row.isVisible())) await folded.click();
  await expect(row).toBeVisible();
  return row;
}

async function disableAgentSummaries(page: Page): Promise<void> {
  await page.locator("[data-open-settings]").click();
  await page.locator('[data-settings-tab="hideAi"]').click();
  const summaries = page.locator("[data-ai-agent-summary]");
  await expect(summaries).toHaveAttribute("data-ai-agent-summary", "true");
  await summaries.click();
  await expect(summaries).toHaveAttribute("data-ai-agent-summary", "false");
  await page.locator("[data-settings-close]").click();
  await expect(page.locator("[data-settings]")).toHaveCount(0);
}

// The last column is the option the default 전부 허용 adds to every restart; Pi never asks and has none.
for (const [kind, id, initialTitle, resumeFlag, fullAccess] of [["pi", PI_ID, PI_TITLE, "--session", []], ["omp", OMP_ID, OMP_TITLE, "--resume", ["--auto-approve"]], ["grok", GROK_ID, GROK_TITLE, "--resume", ["--always-approve"]]] as const) {
test(`${kind} native title and durable sleep wake the exact conversation in a fresh pane`, async ({ page }) => {
  let agents: QuestionRow[] = [];
  page.on("websocket", (socket) => socket.on("framereceived", (frame) => {
    if (typeof frame.payload !== "string") return;
    const incoming = JSON.parse(frame.payload) as { type: string; payload?: { rest?: { navigator?: { agents?: QuestionRow[] } } } };
    if (["snapshot", "delta"].includes(incoming.type) && incoming.payload?.rest?.navigator?.agents) agents = incoming.payload.rest.navigator.agents;
  }));
  const fixture = await startFixture({ agents: false });
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(fixture, `${kind}-reader`, path.join(fixture.root, "home"));
    const sourcePane = fixture.panes[0];
    const session = prepareEngineWriter(fixture, kind);
    expect(fs.existsSync(session)).toBe(false);
    await startNative(fixture, daemon, kind, sourcePane, session);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page);
    const row = await showNativeRow(page, sourcePane);
    // status-model.md, Task identity: an automatic/source-less name follows the goal.
    const selected = kind === "pi" ? "Generated Pi goal" : kind === "grok" ? "Generated Grok goal" : initialTitle;
    await expect(row).toContainText(selected, { timeout: 30_000 });
    if (kind === "pi" || kind === "grok") await disableAgentSummaries(page);
    await expect(row).toContainText(initialTitle, { timeout: 30_000 });
    await screenshot(page, `${kind}-native-title`);
    let title: string = initialTitle;
    if (kind === "omp") {
      const size = fs.statSync(session).size;
      updateOmpTitle(session, OMP_UPDATED_TITLE);
      expect(fs.statSync(session).size).toBe(size);
      title = OMP_UPDATED_TITLE;
      await expect(row).toContainText(title, { timeout: 30_000 });
      await expect(row).not.toContainText("Old audit title");
      const observed = () => agents.find((agent) => agent.pane_id === sourcePane);
      const question = { kind: "question", content: { text: "배포 대상을 골라주세요", choices: ["미리보기", "운영"], truncated: false } };
      appendOmpQuestion(session);
      await expect.poll(() => observed()?.user_turn, { timeout: 30_000 }).toEqual(question);
      await expect(row.locator('[data-agent-status="question"]')).toBeVisible();
      await row.click();
      await expect(page.locator(`[data-pane-view="${sourcePane}"] [data-pane-header-band]`)).toHaveAttribute("data-pane-header-band", "answer");
      await screenshot(page, "omp-current-title-question");
      appendOmpQuestion(session, true);
      await expect.poll(() => observed()?.user_turn, { timeout: 30_000 }).toBeUndefined();
      await expect(row.locator('[data-agent-status="question"]')).toHaveCount(0);
    }
    if (kind === "grok") {
      // Herdr reads Grok's plan approval as working; Grok's own state is the wait.
      const observed = () => agents.find((agent) => agent.pane_id === sourcePane);
      setGrokPlanApproval(session, true);
      await expect.poll(() => observed()?.user_turn, { timeout: 30_000 }).toEqual({ kind: "plan_approval", content: { text: GROK_PLAN, choices: [], truncated: false } });
      await expect(row.locator('[data-agent-status="approval"]')).toBeVisible();
      await screenshot(page, "grok-plan-approval");
      setGrokPlanApproval(session, false);
      await expect.poll(() => observed()?.user_turn, { timeout: 30_000 }).toBeUndefined();
    }
    await row.click();
    await page.locator(`[data-pane-menu="${sourcePane}"]`).click();
    await expect(page.locator('[data-menu-item="sleep_agent"]')).toBeEnabled({ timeout: 20_000 });
    await page.locator('[data-menu-item="sleep_agent"]').click();
    // Every agent of the project sleeps now, so it folds (PRD sidebar-titlebar-refresh B4).
    await openSleepingProject(page);
    const sleeping = page.locator('[data-sleeping-session]');
    await expect(sleeping).toContainText(title, { timeout: 30_000 });
    await expect(row).toHaveCount(0);
    await paneGone(daemon, sourcePane);
    await screenshot(page, `${kind}-durable-sleep`);
    const prior = fs.readFileSync(session);
    await sleeping.getByRole("button", { name: "Wake agent" }).click();
    const fresh = await withControl(daemon, (control) => control.until((rest) => freshAgent(rest, kind, sourcePane), `a fresh ${kind} agent`, 30_000));
    await expect.poll(() => fs.readFileSync(path.join(fixture.root, `${kind}-launches.jsonl`), "utf8").trim().split("\n").map(line => JSON.parse(line) as string[])).toContainEqual([resumeFlag, id, ...fullAccess]);
    await reportSession(daemon, kind, fresh);
    await expect(sleeping).toHaveCount(0, { timeout: 30_000 });
    await expect(await showNativeRow(page, fresh)).toContainText(title);
    expect(fs.readFileSync(session)).toEqual(prior);
    await screenshot(page, `${kind}-exact-wake`);
  } catch (error) {
    throw afterCleanup(afterCleanup(error, () => daemon?.stop()), () => fixture.stop());
  }
  try { daemon?.stop(); } catch (error) { throw afterCleanup(error, () => fixture.stop()); }
  fixture.stop();
});
}

test("OpenCode's root session titles its row, waits on its question and wakes by id in a fresh pane", async ({ page }) => {
  let agents: QuestionRow[] = [];
  page.on("websocket", (socket) => socket.on("framereceived", (frame) => {
    if (typeof frame.payload !== "string") return;
    const incoming = JSON.parse(frame.payload) as { type: string; payload?: { rest?: { navigator?: { agents?: QuestionRow[] } } } };
    if (["snapshot", "delta"].includes(incoming.type) && incoming.payload?.rest?.navigator?.agents) agents = incoming.payload.rest.navigator.agents;
  }));
  const fixture = await startFixture({ agents: false });
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(fixture, "opencode-reader", path.join(fixture.root, "home"));
    const sourcePane = fixture.panes[0];
    const database = prepareOpenCodeWriter(fixture);
    const launches = () => fs.readFileSync(path.join(fixture.root, "opencode-launches.jsonl"), "utf8").trim().split("\n").map(line => JSON.parse(line) as string[]);
    await withControl(daemon, async (control) => sendKeys(control, sourcePane, "opencode\r"));
    await expect.poll(() => fs.existsSync(database)).toBe(true);
    // The double reports its own session: Herdr's integration report on Herdr, Hide's plugin on the own engine.
    await withControl(daemon, (control) => control.until((rest) => (agentIn(rest, sourcePane)?.session_id === OPENCODE_ID ? true : undefined), `${sourcePane} on OpenCode's session`, 30_000));
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page);
    await showNativeRow(page, sourcePane);
    const row = page.locator(`nav[data-sidebar] [data-pane="${sourcePane}"]`);
    // OpenCode records no manual source: goal first, native title with summaries off.
    await expect(row).toContainText("Generated OpenCode goal", { timeout: 30_000 });
    await disableAgentSummaries(page);
    await expect(row).toContainText(OPENCODE_TITLE, { timeout: 30_000 });
    await expect(page.locator("nav[data-sidebar]")).not.toContainText(OPENCODE_CHILD_TITLE);
    await screenshot(page, "opencode-native-title");

    // A question keeps OpenCode's answer unfinished; its answer completes it.
    const question = () => agents.find((agent) => agent.pane_id === sourcePane)?.user_turn;
    const nativeReadState = () => ({
      row: agents.find((agent) => agent.pane_id === sourcePane),
      files: [database, `${database}-wal`, `${database}-shm`].map(file => ({ file, size: fs.existsSync(file) ? fs.statSync(file).size : null })),
      diagnostic: fs.readFileSync(path.join(daemon!.stateDir, "Logs", "core.jsonl"), "utf8").trim().split("\n").slice(-40),
    });
    await reportOpenCodeState(fixture, daemon, sourcePane, "working");
    writeOpenCodeQuestion(database, "running");
    await reportOpenCodeState(fixture, daemon, sourcePane, "idle");
    await dumpOnFailure("OpenCode question native read", nativeReadState, () =>
      expect.poll(question, { message: "the native question reached the row", timeout: 30_000 })
        .toEqual({ kind: "question", content: { text: "어느 브랜치에 올릴까요?", choices: ["main", "release"], truncated: false } }));
    await expect(page.locator(`[data-pane="${sourcePane}"] [data-agent-status="question"]`).first()).toBeVisible();
    await screenshot(page, "opencode-native-question");
    await reportOpenCodeState(fixture, daemon, sourcePane, "working");
    writeOpenCodeQuestion(database, "completed");
    await reportOpenCodeState(fixture, daemon, sourcePane, "idle");
    await expect.poll(question, { message: "the answer cleared the question", timeout: 30_000 }).toBeUndefined();

    // The answered turn is an unread completion, so the row is raised under Done as well.
    await row.first().click();
    await page.locator(`[data-pane-menu="${sourcePane}"]`).click();
    await dumpOnFailure("OpenCode sleep native proof", nativeReadState, () =>
      expect(page.locator('[data-menu-item="sleep_agent"]')).toBeEnabled({ timeout: 20_000 }));
    await page.locator('[data-menu-item="sleep_agent"]').click();
    await openSleepingProject(page);
    const sleeping = page.locator("[data-sleeping-session]");
    await expect(sleeping).toContainText(OPENCODE_TITLE, { timeout: 30_000 });
    await paneGone(daemon, sourcePane);
    // OpenCode's database and its write-ahead log: a write lands in either.
    const digest = () => {
      const hash = crypto.createHash("sha256").update(fs.readFileSync(database));
      if (fs.existsSync(`${database}-wal`)) hash.update(fs.readFileSync(`${database}-wal`));
      return hash.digest("hex");
    };
    const prior = digest();
    await sleeping.getByRole("button", { name: "Wake agent" }).click();
    await expect.poll(launches, { timeout: 30_000 }).toContainEqual(["-s", OPENCODE_ID, "--auto"]);
    const fresh = await withControl(daemon, (control) => control.until((rest) => freshAgent(rest, "opencode", sourcePane), "a fresh opencode agent", 30_000));
    await expect(sleeping).toHaveCount(0, { timeout: 30_000 });
    await expect(page.locator(`nav[data-sidebar] [data-pane="${fresh}"]`)).toContainText(OPENCODE_TITLE);
    expect(digest()).toBe(prior);
    await screenshot(page, "opencode-exact-wake");
  } finally {
    daemon?.stop();
    fixture.stop();
  }
});
