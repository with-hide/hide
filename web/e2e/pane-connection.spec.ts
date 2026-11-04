// The integration status control on an isolated engine and hided (PRD settings-cleanup B26, B28, B29, B31):
// a Claude Code pane that was running when Hide's hook was installed wears the chip, its popover offers
// Reopen and Not now, Reopen ends the agent and starts the same conversation in the same pane, and the
// pane's own SessionStart hook (the real helper, run the way the agent would run it) turns the chip off.
// A refused Reopen leaves the pane as it was and says why in the popover. A pane the hook connected stays
// connected across a Herdr live handoff, which empties every pane token, with no further hook event (issue 799).
// The Codex shared-server reasons
// need a Codex pane Herdr detects and a machine kit that read it, which this fixture has no program for:
// `PaneConnection.test.tsx` and the core's `runtime::tests::agent_connection` own them.

import { expect, test, type Page } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { agentRunner, reportedMarker, type AgentRun, type HideAnswer } from "./agent-pane";
import { agentIn, waitForInputGate, withControl } from "./daemon-actions";
import { startFixture, type AgentFixture } from "./engine-fixture";
import { setAgentState } from "./fixture-agents";
import { CLAUDE_FULL_ACCESS_LINE, claudeProjects, liveHandoff, setFixtureSession, startHerdr, writeFixtureTranscript, type HerdrFixture } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { fixtureExecutable } from "./platform-fixture";
import { snapshotWorking } from "./subagent-counts";
import { countSent, screenshot } from "./wire";

test.describe.configure({ timeout: 150_000 });

// The session the fixture's second agent runs, and the one the Herdr-only handoff test gives its pane.
const FIXTURE_SESSION = "fixture-two";
const SESSION = "22222222-3333-4444-5555-666666666666";
const HELPER = path.resolve("..", "target", "debug", "hide-agent-hooks");

type ProcessInfo = { result: { process_info: { foreground_processes?: { argv?: string[] | null }[] } } };

/** Hide's hook entries as the kit writes them (`hide-agent-hooks/src/install.rs`), so this machine's files say the hook is installed. */
function installHook(home: string): void {
  const entry = (event: string) => [
    {
      // The opener guard's entry selects the shell tool and asks for no Memory.
      ...(event === "PreToolUse" ? { matcher: "Bash" } : {}),
      hooks: [
        {
          type: "command",
          command: `if [ -x '${HELPER}' ]; then exec '${HELPER}' hook --runtime claude-code --event ${event}${event === "PreToolUse" ? "" : " --memory-injection"} --source hide-subagents@6; fi`,
          timeout: 8,
        },
      ],
    },
  ];
  const hooks = Object.fromEntries(["SessionStart", "UserPromptSubmit", "SubagentStart", "SubagentStop", "Stop", "PreToolUse"].map((event) => [event, entry(event)]));
  fs.mkdirSync(path.join(home, ".claude"), { recursive: true });
  fs.writeFileSync(path.join(home, ".claude", "settings.json"), JSON.stringify({ hooks }));
}

/** The reopen continues the conversation with the default 전부 허용 option, which every restart Hide makes carries. */
const resumeLine = (session: string) => `--resume ${session} ${CLAUDE_FULL_ACCESS_LINE}`;
const RESUME = resumeLine(SESSION);

/** Runs Hide's hook for `event` as the agent in the pane does: from inside its process, which hided proves is the pane's. */
async function runHook(asAgent: (args: string[], run?: AgentRun) => Promise<HideAnswer>, event: "SessionStart" | "SubagentStart", session = SESSION): Promise<void> {
  const ran = await asAgent(["hook", "--runtime", "claude-code", "--event", event, "--memory-injection", "--source", "hide-subagents@6"], {
    program: path.resolve("..", "target", "debug", fixtureExecutable("hide-agent-hooks")),
    input: JSON.stringify({ session_id: session, ...(event === "SubagentStart" ? { agent_id: "sub-1", agent_type: "general-purpose" } : {}) }),
  });
  expect(ran.status, `the ${event} hook`).toBe(0);
}

/** What the pane draws, as the page's probe reads it, with the terminal's line breaks and spaces taken out so a wrapped command line reads as one. */
const drawn = (page: Page, pane: string) => page.evaluate((id) => (window.__hideProbe?.paneText(id) ?? "").replace(/\s+/g, ""), pane);

/** How many times the pane's screen shows the command line that resumes `session` with the full-access option Hide adds. */
async function resumeLines(page: Page, pane: string, session: string): Promise<number> {
  const screen = await drawn(page, pane);
  // The shell's quoting of the settings argument differs between shells, so a line is read by its words.
  const words = ["--dangerously-skip-permissions", "--settings", "skipDangerousModePermissionPrompt"];
  return screen.split(`--resume${session}`).slice(1).filter((rest) => words.every((word) => rest.includes(word))).length;
}

async function start(page: Page, label: string): Promise<{ fixture: AgentFixture; daemon: Daemon; pane: string; sent: Map<string, number> }> {
  await page.setViewportSize({ width: 1440, height: 900 });
  const fixture = await startFixture();
  // A pane's shell reads the files of the daemon's HOME on the core's own engine.
  const home = fixture.home ?? path.join(fixture.root, "hided-home");
  installHook(home);
  const daemon = await startHided(fixture, label, home);
  const [, pane] = fixture.panes;
  // A started agent can read as working until its screen says it rests, and a Reopen waits for a rest.
  await setAgentState(daemon, pane, "idle");
  const sent = countSent(page);
  await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
  await page.locator("[data-checkout]").first().click();
  await expect(page.locator(`[data-pane-view="${pane}"]`)).toBeVisible({ timeout: 20_000 });
  return { fixture, daemon, pane, sent };
}

test("a session started before the hook wears the chip, Not now keeps it, and Reopen continues the conversation and connects it", async ({ page }) => {
  const { fixture, daemon, pane, sent } = await start(page, "pane-connection");
  const readyGate = path.join(fixture.root, "resumed-terminal.gate");
  const held = `${readyGate}.arrived`;
  try {
    const view = page.locator(`[data-pane-view="${pane}"]`);
    const chip = view.locator("[data-pane-connection]");
    // B26: both fixture agents were running before the hook was installed, so both panes wear the chip.
    const other = page.locator(`[data-pane-view="${fixture.panes[0]}"] [data-pane-connection]`);
    await expect(chip).toHaveAttribute("data-pane-connection", "started_before_hide", { timeout: 30_000 });
    await expect(other).toHaveAttribute("data-pane-connection", "started_before_hide");

    // B28, B31: the popover says why and offers Reopen and Not now; Not now closes it and the chip stays.
    await chip.click();
    const popover = page.locator("[data-pane-connection-popover]");
    await expect(chip).toHaveAccessibleName("Hide integration limited");
    await expect(chip).toHaveText("");
    await chip.press("Escape");
    await expect(chip).toBeFocused();
    await chip.press("Enter");
    await expect(popover).toContainText("Reconnect this session to Hide");
    await chip.press("Escape");
    await chip.press("Space");
    await expect(popover).toBeVisible();
    await screenshot(page, "pane-connection-popover");
    await popover.locator("[data-pane-connection-dismiss]").click();
    await expect(popover).toHaveCount(0);
    await expect(chip).toBeVisible();

    // B29: Reopen sends one event and the same conversation starts again in the same pane, as an agent whose hooks
    // run from inside its own process.
    const ready = path.join(fixture.root, `terminal-ready-${pane.replaceAll(":", "-")}`);
    await expect.poll(() => fs.existsSync(ready), { message: "the old program left its input-ready mark" }).toBe(true);
    const asAgent = agentRunner(fixture, daemon.stateDir, pane);
    // Hold the real reporter after its session report, before the native program
    // configures its terminal. An echoed resume line must not admit a hook key.
    fs.appendFileSync(path.join(fixture.root, "agent-pane-session.cjs"), `
fs.writeFileSync(${JSON.stringify(held)}, "");
${waitForInputGate(readyGate, 15_000)}
`);
    const startLines = await resumeLines(page, pane, FIXTURE_SESSION);
    await chip.click();
    await popover.locator(`[data-pane-reopen="${pane}"]`).dblclick();
    await expect.poll(() => sent.get("pane_reopen")).toBe(1);
    // The line that started the agent is on the screen already: the reopen types a second one, which the resumed agent answers.
    await expect.poll(() => resumeLines(page, pane, FIXTURE_SESSION), { timeout: 40_000 }).toBe(startLines + 1);
    await expect.poll(() => fs.existsSync(held), { message: "the resumed program held before terminal setup", timeout: 20_000 }).toBe(true);
    expect(fs.existsSync(ready), "the old program cannot make the resumed one input-ready").toBe(false);

    // The resumed agent runs its own SessionStart hook, which is what tells Hide it is listening.
    const hook = runHook(asAgent, "SessionStart", FIXTURE_SESSION);
    try {
      // Let the command attempt enter its readiness wait while the program is held.
      await new Promise<void>((resolve) => setImmediate(resolve));
      expect(fs.existsSync(path.join(fixture.root, "agent-pane-command.json")), "no command is submitted before fresh readiness").toBe(false);
    } finally {
      fs.writeFileSync(readyGate, "");
      await hook;
    }
    expect(fs.existsSync(ready), "the resumed program wrote its own input-ready mark").toBe(true);
    await expect(chip).toHaveCount(0, { timeout: 30_000 });
    await expect(page.locator("[data-pane-connection-popover]")).toHaveCount(0);
    // Only the pane that connected lost its chip.
    await expect(other).toBeVisible();
    expect(sent.get("pane_reopen")).toBe(1);
  } finally {
    fs.writeFileSync(readyGate, "");
    daemon.stop();
    fixture.stop();
  }
});

test("a Reopen the core refuses leaves the pane as it was and the popover says why", async ({ page }) => {
  const { fixture, daemon, pane, sent } = await start(page, "pane-connection-busy");
  try {
    await setAgentState(daemon, pane, "working");
    const view = page.locator(`[data-pane-view="${pane}"]`);
    await expect(view.locator("[data-pane-connection]")).toBeVisible({ timeout: 30_000 });
    // What the sidebar says of the agent: its session and what it is doing.
    const row = () => withControl(daemon, (control) => control.until((rest) => {
      const agent = agentIn(rest, pane);
      return agent && { session: agent.session_id, status: agent.status_code, activity: agent.activity, blocked: agent.blocked };
    }, `the agent in ${pane}`, 20_000));
    const before = { row: await row(), screen: await drawn(page, pane) };

    await view.locator("[data-pane-connection]").click();
    await page.locator(`[data-pane-reopen="${pane}"]`).click();
    await expect.poll(() => sent.get("pane_reopen")).toBe(1);
    const failure = page.locator("[data-pane-reopen-failed]");
    await expect(failure).toHaveAttribute("data-pane-reopen-failed", "agent_busy", { timeout: 20_000 });
    await expect(failure).toContainText("working or waiting");
    await screenshot(page, "pane-connection-refused");
    // Everything knowable first was refused first: the agent was not touched.
    expect({ row: await row(), screen: await drawn(page, pane) }).toEqual(before);
    expect(fs.existsSync(reportedMarker(fixture.root, pane))).toBe(false);
    await expect(page.locator(`[data-pane-reopen="${pane}"]`)).toBeEnabled();
  } finally {
    daemon.stop();
    fixture.stop();
  }
});

/** The Herdr-only handoff test's fixture: the pinned Herdr on a short root, with its agents. */
async function startHandoff(page: Page, label: string): Promise<{ herdr: HerdrFixture; daemon: Daemon; pane: string; working: () => number | null | undefined }> {
  await page.setViewportSize({ width: 1440, height: 900 });
  const herdr = await startHerdr({ shortRoot: true });
  const [, pane] = herdr.panes;
  writeFixtureTranscript(claudeProjects(herdr), SESSION, { task: "Agent two" });
  setFixtureSession(herdr, pane, SESSION);
  const home = path.join(herdr.root, "hided-home");
  installHook(home);
  const daemon = await startHided(herdr, label, home);
  // Listening before the page loads: the snapshot frames of its first connection are the ones a later read is compared with.
  const working = snapshotWorking(page, pane);
  await page.goto(`${daemon.origin}/#token=${daemon.token}`);
  await page.locator("[data-checkout]").first().click();
  await expect(page.locator(`[data-pane-view="${pane}"]`)).toBeVisible({ timeout: 20_000 });
  return { herdr, daemon, pane, working };
}

// Herdr leaves `foreground_processes` out of the answer while the pane's shell has not started a program yet.
const processArgv = (herdr: HerdrFixture, pane: string) =>
  ((herdr.run(["pane", "process-info", "--pane", pane]) as ProcessInfo).result.process_info.foreground_processes ?? []).map((process) => process.argv?.join(" "));

/** Whether the pane's foreground now runs the same conversation again; the agent runner execs as `.../claude`. */
const resumedOnHerdr = (herdr: HerdrFixture, pane: string): boolean =>
  processArgv(herdr, pane).some((argv) => argv === `claude ${RESUME}` || argv.endsWith(`/claude ${RESUME}`) || argv.endsWith(`\\claude.exe ${RESUME}`));

type PaneRow = { result: { pane: { terminal_id: string } } };
const paneRow = (herdr: HerdrFixture, pane: string) => (herdr.run(["pane", "get", pane]) as PaneRow).result.pane;

// Herdr's live handoff is what an update runs (`scripts/install-local-runtime.sh`) and it starts the new server with
// no pane tokens. The hook reports to the core, so nothing of its connection is Herdr's to lose (PRD engine-bundle-1
// D-8). Windows has no handoff.
// Herdr only: it asks the pinned Herdr server for its live handoff and compares the pane's terminal before and after.
test("a pane the hook connected stays connected across a Herdr live handoff with no further hook event @herdr", async ({ page }) => {
  test.skip(process.platform === "win32", "Herdr's live handoff is a Unix server feature");
  const { herdr, daemon, pane, working } = await startHandoff(page, "pane-connection-handoff");
  try {
    const chip = page.locator(`[data-pane-view="${pane}"] [data-pane-connection]`);
    await expect(chip).toHaveAttribute("data-pane-connection", "started_before_hide", { timeout: 30_000 });

    // Reopened, the agent's own hooks connect the pane and count one running subagent.
    const asAgent = agentRunner(herdr, daemon.stateDir, pane);
    await chip.click();
    await page.locator(`[data-pane-reopen="${pane}"]`).click();
    await expect.poll(() => resumedOnHerdr(herdr, pane), { timeout: 40_000 }).toBe(true);
    await runHook(asAgent, "SessionStart");
    await runHook(asAgent, "SubagentStart");
    await expect(chip).toHaveCount(0, { timeout: 30_000 });
    const before = paneRow(herdr, pane);
    await expect.poll(working, { message: "the snapshot's working count before the handoff", timeout: 20_000 }).toBe(1);

    liveHandoff(herdr);
    // The pane is the same pane in another terminal: Herdr started a new server.
    expect(paneRow(herdr, pane).terminal_id).not.toBe(before.terminal_id);

    // No hook event follows, and the core still holds what the hook reported.
    await expect(chip).toHaveCount(0);
    await expect.poll(working, { message: "the snapshot's working count after the handoff", timeout: 20_000 }).toBe(1);
  } finally {
    daemon.stop();
    herdr.stop();
  }
});
