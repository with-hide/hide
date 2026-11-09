// A Codex plan waiting for approval on an isolated engine and hided (PRD
// codex-plan-approval-hold B1, B4, B12): the engine reads Codex's
// "Implement this plan?" menu as an ordinary stop, so the wait is read from
// the session file. A rollout whose last plan-mode turn proposed a plan puts
// the Codex row in Needs You with the approval mark, and the next turn's
// record takes it out. That the row stays after it is read (B2) is the
// core's rule, tested in `hide-core/src/runtime/tests/labels.rs`.
//
// The fixture Codex is the shim copied as `codex`. The engine reads its
// screen as working while the title carries Codex's spinner and, once a title
// without it is drawn (the `osc_title_idle` rule), as a finished turn. A real
// Codex back at its prompt reads the same. Each change moves the engine's
// state sequence, which is what makes the core read the session again, so the
// wait is read for the state the row is in.

import { expect, test } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { startFixture, type EngineServer } from "./engine-fixture";
import { agentIn, openProject, sendKeys, withControl } from "./daemon-actions";
import { setAgentSession } from "./fixture-agents";
import { fixtureExecutable } from "./platform-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { enterWorkspace, screenshot, showProjectRow } from "./wire";

test.describe.configure({ timeout: 120_000 });

const SESSION = "0199a000-0000-7000-8000-0000000000e2";

/** One Codex 0.160.1 rollout record. */
function record(type: string, payload: Record<string, unknown>): string {
  return `${JSON.stringify({ timestamp: "2026-10-07T01:00:00.000Z", type, payload })}\n`;
}

function event(kind: string, turn: string, extra: Record<string, unknown> = {}): string {
  return record("event_msg", { type: kind, turn_id: turn, ...extra });
}

function person(text: string): string {
  return record("response_item", { type: "message", role: "user", content: [{ type: "input_text", text }] });
}

/** A session whose last turn ran in plan mode and proposed a plan. */
function planWaiting(cwd: string): string {
  return record("session_meta", { id: SESSION, cwd, cli_version: "0.160.1" })
    + event("task_started", "turn-1", { collaboration_mode_kind: "plan" })
    + person("로그인 화면을 고칠 계획을 세워줘")
    + event("item_completed", "turn-1", { item: { type: "Plan", id: "item-1", text: "1. 입력 검증을 고친다" } })
    + record("response_item", { type: "message", role: "assistant", content: [{ type: "output_text", text: "<proposed_plan>\n1. 입력 검증을 고친다\n</proposed_plan>" }] })
    + event("task_complete", "turn-1", { last_agent_message: null });
}

/** The session read the daemon keeps for the pane: its engine state and the last turn it folded. */
function turnRead(daemon: Daemon, pane: string): { seq?: number; mode?: string } {
  type Stored = { turns_seq?: number; turns?: { last?: { mode?: string } } };
  try {
    const file = JSON.parse(fs.readFileSync(path.join(daemon.stateDir, "labels.json"), "utf8")) as { targets?: Record<string, Record<string, Stored>> };
    const record = Object.values(file.targets ?? {}).map((panes) => panes[pane]).find(Boolean);
    return { seq: record?.turns_seq, mode: record?.turns?.last?.mode };
  } catch {
    return {};
  }
}

/**
 * Draws the title a Codex screen shows while it works (the spinner) or once
 * its turn ended (a title without one). Only the working state is waited on:
 * a rest the core holds in Needs You reads as neither idle nor done.
 */
async function drawTitle(daemon: Daemon, pane: string, state: "working" | "ended"): Promise<void> {
  await withControl(daemon, async (control) => {
    sendKeys(control, pane, `\x1b]0;${state === "working" ? "⠋ Working" : "plan"}\x07`);
    if (state === "working") await control.until((rest) => agentIn(rest, pane)?.activity === "working", `${pane} working`, 20_000);
  });
}

/**
 * Starts the Codex stand-in in `pane`, whose shell is at its prompt, and
 * returns once its row names `SESSION`, as Codex's integration reports the
 * session at SessionStart: Hide's hook helper with Codex's payload, run by
 * the stand-in itself after the engine lists it. No background shell job
 * shares the stand-in's tty.
 */
async function startCodex(fixture: EngineServer, daemon: Daemon, pane: string): Promise<void> {
  await withControl(daemon, async (control) => {
    sendKeys(control, pane, "codex\r");
    await control.until((rest) => agentIn(rest, pane), "the Codex agent in the plan project", 30_000);
  });
  await setAgentSession(fixture, daemon, pane, SESSION);
  await withControl(daemon, (control) => control.until((rest) => agentIn(rest, pane)?.session_id === SESSION, `${pane} on session ${SESSION}`, 20_000));
}

test("a Codex plan waiting for approval holds its row in Needs You until the next turn", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const fixture = await startFixture();
  let daemon: Daemon | null = null;
  try {
    const cwd = path.join(fixture.root, "plan");
    fs.mkdirSync(cwd, { recursive: true });
    fs.copyFileSync(path.join(fixture.root, "bin", fixtureExecutable("claude")), path.join(fixture.root, "bin", fixtureExecutable("codex")));
    daemon = await startHided(fixture, "codex-plan-approval");
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    // A project whose shell runs the Codex stand-in, typed at its prompt once
    // the page draws it: a pane takes the operator's keys only while a screen shows it.
    const plan = await withControl(daemon, (control) => openProject(control, cwd, "plan"));
    const pane = plan.pane;
    await (await showProjectRow(page, page.locator("[data-project]", { hasText: "plan" }).locator("[data-checkout]").first())).click({ position: { x: 12, y: 8 } });
    await expect(page.locator("[data-canvas]").first()).toHaveAttribute("data-canvas", plan.tab);
    await expect.poll(() => page.evaluate((id) => window.__hideProbe?.paneText(id) ?? "", pane), { timeout: 20_000, message: "the prompt in the plan project" }).toContain("fixture %");
    await startCodex(fixture, daemon, pane);

    const sessions = path.join(daemon.home, ".codex", "sessions", "2026", "10", "07");
    fs.mkdirSync(sessions, { recursive: true });
    const rollout = path.join(sessions, `rollout-2026-10-07T01-00-00-${SESSION}.jsonl`);
    fs.writeFileSync(rollout, planWaiting(cwd));

    const row = page.locator(`nav[data-sidebar] [data-project] [data-pane="${pane}"]`);
    await expect(row).toBeVisible({ timeout: 20_000 });

    // B1: the turn that proposed the plan has ended, as the engine's next state
    // reads it; the row is the operator's to answer, with the approval mark.
    await drawTitle(daemon, pane, "working");
    await drawTitle(daemon, pane, "ended");
    const waiting = page.locator(`[data-raised-group="needs_you"] [data-pane="${pane}"]`);
    await expect(waiting).toBeVisible({ timeout: 20_000 });
    await expect(waiting.locator('[data-mark="turn"]')).toBeVisible();
    // The state the plan was read for, which the approving turn's read must pass.
    await expect.poll(() => turnRead(daemon!, pane).mode, { message: "the plan turn is read", timeout: 20_000 }).toBe("plan");
    const planSeq = turnRead(daemon, pane).seq;
    await screenshot(page, "codex-plan-approval-waiting");

    // B4: approving starts the next turn; once the engine's state moves, the row
    // leaves Needs You.
    // A project of one agent that no longer waits folds to its project row, so the operator opens it to see the agent's row.
    const toggle = page.locator("nav[data-sidebar] [data-project]", { hasText: "plan" }).locator("[data-checkout-toggle]");
    fs.appendFileSync(rollout, event("task_started", "turn-2", { collaboration_mode_kind: "default" }) + person("Implement the plan."));
    await drawTitle(daemon, pane, "working");
    await expect(row.or(toggle)).toBeVisible();
    if (!(await row.isVisible())) await toggle.click();
    await expect(row.locator('[data-mark="going"]')).toBeVisible({ timeout: 20_000 });
    await drawTitle(daemon, pane, "ended");
    // A state not read yet shows no wait either, so the row's absence counts
    // only once the read for a later state has folded the approving turn.
    await expect.poll(() => {
      const read = turnRead(daemon!, pane);
      return { mode: read.mode, later: (read.seq ?? 0) > planSeq! };
    }, { message: "the session is read for a later state", timeout: 20_000 }).toEqual({ mode: "other", later: true });
    await expect(row).toBeVisible();
    await expect(page.locator(`[data-raised-group="needs_you"] [data-pane="${pane}"]`)).toHaveCount(0);
    await screenshot(page, "codex-plan-approval-approved");
  } finally {
    daemon?.stop();
    fixture.stop();
  }
});
