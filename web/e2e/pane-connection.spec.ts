// The integration status control on an isolated pinned Herdr and hided (PRD settings-cleanup B26, B28, B29, B31):
// a Claude Code pane that was running when Hide's hook was installed wears the chip, its popover offers
// Reopen and Not now, Reopen ends the agent and starts the same conversation in the same pane, and the
// pane's own SessionStart hook (the real helper, run the way the agent would run it) turns the chip off.
// A refused Reopen leaves the pane as it was and says why in the popover. A pane the hook connected stays
// connected across a Herdr live handoff, which empties every pane token, with no further hook event (issue 799).
// The Codex shared-server reasons
// need a Codex pane Herdr detects and a machine kit that read it, which this fixture has no program for:
// `PaneConnection.test.tsx` and the core's `runtime::tests::agent_connection` own them.

import { expect, test, type Page } from "@playwright/test";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { claudeProjects, liveHandoff, setFixtureLifecycle, setFixtureSession, startHerdr, writeFixtureTranscript, type HerdrFixture } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { snapshotWorking } from "./subagent-counts";
import { countSent, screenshot } from "./wire";

test.describe.configure({ timeout: 150_000 });

const SESSION = "22222222-3333-4444-5555-666666666666";
const HELPER = path.resolve("..", "target", "debug", "hide-agent-hooks");

type ProcessInfo = { result: { process_info: { foreground_processes?: { argv?: string[] | null }[] } } };

/** Hide's hook entries as the kit writes them (`hide-agent-hooks/src/install.rs`), so this machine's files say the hook is installed. */
function installHook(home: string): void {
  const entry = (event: string) => [
    {
      // The spawn guard's entry selects the shell tool and asks for no Memory.
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

/** Runs the real helper for one of the agent's hook events, the way the agent would run it in `pane`: its input names the session. */
function runHook(daemon: Daemon, herdr: HerdrFixture, pane: string, event: "SessionStart" | "SubagentStart"): void {
  execFileSync(HELPER, ["hook", "--runtime", "claude-code", "--event", event, "--memory-injection", "--source", "hide-subagents@6"], {
    input: JSON.stringify({ session_id: SESSION }),
    env: { ...process.env, HOME: daemon.home, HERDR_PANE_ID: pane, HERDR_SOCKET_PATH: herdr.socket, HERDR_ENV: "1" },
    timeout: 20_000,
  });
}

async function start(page: Page, label: string, options: { shortRoot?: boolean } = {}): Promise<{ herdr: HerdrFixture; daemon: Daemon; pane: string; sent: Map<string, number>; working: () => number | null | undefined }> {
  await page.setViewportSize({ width: 1440, height: 900 });
  const herdr = await startHerdr(options);
  const [, pane] = herdr.panes;
  writeFixtureTranscript(claudeProjects(herdr), SESSION, { task: "Agent two" });
  setFixtureSession(herdr, pane, SESSION);
  const home = path.join(herdr.root, "hided-home");
  installHook(home);
  const daemon = await startHided(herdr, label, home);
  const sent = countSent(page);
  // Listening before the page loads: the snapshot frames of its first connection are the ones a later read is compared with.
  const working = snapshotWorking(page, pane);
  await page.goto(`${daemon.origin}/#token=${daemon.token}`);
  await page.locator("[data-checkout]").first().click();
  await expect(page.locator(`[data-pane-view="${pane}"]`)).toBeVisible({ timeout: 20_000 });
  return { herdr, daemon, pane, sent, working };
}

// Herdr leaves `foreground_processes` out of the answer while the pane's shell has not started a program yet.
const processArgv = (herdr: HerdrFixture, pane: string) =>
  ((herdr.run(["pane", "process-info", "--pane", pane]) as ProcessInfo).result.process_info.foreground_processes ?? []).map((process) => process.argv?.join(" "));

test("a session started before the hook wears the chip, Not now keeps it, and Reopen continues the conversation and connects it", async ({ page }) => {
  const { herdr, daemon, pane, sent } = await start(page, "pane-connection");
  try {
    const view = page.locator(`[data-pane-view="${pane}"]`);
    const chip = view.locator("[data-pane-connection]");
    // B26: both fixture agents were running before the hook was installed, so both panes wear the chip.
    const other = page.locator(`[data-pane-view="${herdr.panes[0]}"] [data-pane-connection]`);
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

    // B29: Reopen sends one event and the same conversation starts again in the same pane.
    await chip.click();
    await popover.locator(`[data-pane-reopen="${pane}"]`).dblclick();
    await expect.poll(() => sent.get("pane_reopen")).toBe(1);
    await expect.poll(() => processArgv(herdr, pane), { timeout: 40_000 }).toContain(`claude --resume ${SESSION}`);

    // The resumed agent runs its own SessionStart hook, which is what tells Hide it is listening.
    runHook(daemon, herdr, pane, "SessionStart");
    await expect(chip).toHaveCount(0, { timeout: 30_000 });
    await expect(page.locator("[data-pane-connection-popover]")).toHaveCount(0);
    // Only the pane that connected lost its chip.
    await expect(other).toBeVisible();
    expect(sent.get("pane_reopen")).toBe(1);
  } finally {
    daemon.stop();
    herdr.stop();
  }
});

test("a Reopen the core refuses leaves the pane as it was and the popover says why", async ({ page }) => {
  const { herdr, daemon, pane, sent } = await start(page, "pane-connection-busy");
  try {
    await setFixtureLifecycle(herdr, pane, "working");
    const view = page.locator(`[data-pane-view="${pane}"]`);
    await expect(view.locator("[data-pane-connection]")).toBeVisible({ timeout: 30_000 });
    await expect.poll(() => processArgv(herdr, pane).length, { timeout: 20_000 }).toBeGreaterThan(0);
    const before = processArgv(herdr, pane);

    await view.locator("[data-pane-connection]").click();
    await page.locator(`[data-pane-reopen="${pane}"]`).click();
    await expect.poll(() => sent.get("pane_reopen")).toBe(1);
    const failure = page.locator("[data-pane-reopen-failed]");
    await expect(failure).toHaveAttribute("data-pane-reopen-failed", "agent_busy", { timeout: 20_000 });
    await expect(failure).toContainText("working or waiting");
    await screenshot(page, "pane-connection-refused");
    // Everything knowable first was refused first: the agent was not touched.
    expect(processArgv(herdr, pane)).toEqual(before);
    await expect(page.locator(`[data-pane-reopen="${pane}"]`)).toBeEnabled();
  } finally {
    daemon.stop();
    herdr.stop();
  }
});

type PaneRow = { result: { pane: { terminal_id: string; tokens?: Record<string, string> } } };
const paneRow = (herdr: HerdrFixture, pane: string) => (herdr.run(["pane", "get", pane]) as PaneRow).result.pane;

// Herdr's live handoff is what an update runs (`scripts/install-local-runtime.sh`) and it starts the new server with
// no pane tokens. Windows has no handoff.
test("a pane the hook connected stays connected across a Herdr live handoff with no further hook event", async ({ page }) => {
  test.skip(process.platform === "win32", "Herdr's live handoff is a Unix server feature");
  const { herdr, daemon, pane, working } = await start(page, "pane-connection-handoff", { shortRoot: true });
  try {
    const chip = page.locator(`[data-pane-view="${pane}"] [data-pane-connection]`);
    await expect(chip).toHaveAttribute("data-pane-connection", "started_before_hide", { timeout: 30_000 });

    // The agent's own hooks connect the pane and count one running subagent.
    runHook(daemon, herdr, pane, "SessionStart");
    runHook(daemon, herdr, pane, "SubagentStart");
    await expect(chip).toHaveCount(0, { timeout: 30_000 });
    const before = paneRow(herdr, pane);
    expect(before.tokens).toMatchObject({ hide_sub_working: "1", hide_sub_done: "0" });
    await expect.poll(working, { message: "the snapshot's working count before the handoff", timeout: 20_000 }).toBe(1);

    liveHandoff(herdr);
    // The pane is the same pane in another terminal: Herdr started a new server, and that server took no tokens.
    expect(paneRow(herdr, pane).terminal_id).not.toBe(before.terminal_id);

    // No hook event follows. hided puts back what the hook's file holds, on Herdr itself and so in what the core reads.
    await expect.poll(() => paneRow(herdr, pane).tokens, { message: "the pane's tokens on the new server", timeout: 30_000 }).toEqual(before.tokens);
    await expect(chip).toHaveCount(0);
    await expect.poll(working, { message: "the snapshot's working count after the handoff", timeout: 20_000 }).toBe(1);
  } finally {
    daemon.stop();
    herdr.stop();
  }
});
