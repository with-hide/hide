// @platform: Starts cross the real shell and native process boundary. The
// Windows cases also exercise npm's PowerShell and cmd launchers over Node.
import fs from "node:fs";
import path from "node:path";
import { expect, test, type Page } from "@playwright/test";
import type { SnapshotRest } from "../src/snapshot";
import { agentIn, newTab, preparePaneInput, sendKeys, withControl, type AgentRow } from "./daemon-actions";
import { waitForAgentInput } from "./agent-pane";
import { resumeFlags, startProgram } from "./agent-start-fixture";
import { startCaller, type CallerFixture } from "./engine-fixture";
import { spawnRoot } from "./fixture-agents";
import { startHided, type Daemon } from "./hided-fixture";
import { fixtureProgram } from "./platform-fixture";
import { enterWorkspace } from "./wire";
import { afterCleanup } from "./worker-owned";

/** The process is recognized and the sidebar renders its resting state. */
async function listed(page: Page, fixture: CallerFixture, daemon: Daemon, pane: string, kind: string): Promise<void> {
  await waitForAgentInput(fixture.root, pane);
  await withControl(daemon, async (control) => {
    await control.until((rest) => agentIn(rest, pane)?.agent_kind === kind || undefined,
      `${kind} process in ${pane}`, 30_000);
    await preparePaneInput(control, pane);
    sendKeys(control, pane, "?");
  });
  await withControl(daemon, (control) => control.until((rest) => {
    const agent = agentIn(rest, pane);
    return agent?.agent_kind === kind && agent.status_code === "idle" ? agent : undefined;
  }, `${kind} idle in ${pane}`, 30_000));
  const status = page.locator(`nav[data-sidebar] [data-pane="${pane}"] [data-agent-status]`);
  await expect(status).toBeVisible();
  await expect(status).toHaveAttribute("data-agent-status", "idle");
}

/** Opens the page on the fixture's checkout; Codex's start first needs the kit to have read its installed program. */
async function openPage(page: Page, daemon: Daemon, kind: string): Promise<void> {
  await page.goto(`${daemon.origin}/#token=${daemon.token}`);
  await enterWorkspace(page, "fixture");
  if (kind === "codex") {
    // A real start requires the kit to have read this binary's daemon
    // capability. The stand-in answers features list as an older CLI.
    await withControl(daemon, (control) => control.until((rest) => {
      const devices = (rest as SnapshotRest).navigator?.devices;
      return devices?.find((device) => device.id === daemon.node)?.kit?.codex_daemon === false || undefined;
    }, "the installed start program's daemon capability", 30_000));
  }
}

/** The fixture's agent spawns `kind`, which is listed in its own pane and rests there. */
async function spawnStart(page: Page, fixture: CallerFixture, daemon: Daemon, kind: string): Promise<void> {
  const agents = (rest: { navigator?: unknown }) => (rest.navigator as { agents?: AgentRow[] } | undefined)?.agents ?? [];
  const previous = await withControl(daemon, (control) => control.until((rest) => new Set(agents(rest).map((agent) => agent.pane_id)), "existing agents"));
  // The native integration reports while spawn waits for that identity.
  // Waiting for spawn before letting the stand-in report would deadlock.
  const [spawned, observed] = await Promise.all([spawnRoot(fixture, daemon, fixture.pane, {
    name: `start-${kind}`, kind, repo: path.join(fixture.root, "fixture"),
    // A resume gives spawn its required identity.
    nativeArgs: [resumeFlags[kind] ?? "--resume", ["pi", "omp"].includes(kind)
      ? path.join(fixture.root, `platform-${kind}.jsonl`) : `platform-${kind}`],
  }), (async () => {
    const pane = await withControl(daemon, (control) => control.until((rest) => agents(rest).find((agent) =>
      agent.agent_kind === kind && !previous.has(agent.pane_id))?.pane_id, `the new ${kind} process`, 30_000));
    await listed(page, fixture, daemon, pane, kind);
    return pane;
  })()]);
  expect(spawned).toBe(observed);
}

/** `command`, typed at a new tab's shell, starts a fresh `kind` with no session declared by the caller, listed and resting. */
async function typedStart(page: Page, fixture: CallerFixture, daemon: Daemon, kind: string, command: string): Promise<void> {
  const typed = await withControl(daemon, async (control) => {
    const { pane } = await newTab(control, fixture, `typed-${kind}`);
    await preparePaneInput(control, pane);
    sendKeys(control, pane, `${command}\r`);
    return pane;
  });
  await listed(page, fixture, daemon, typed, kind);
}

/**
 * Runs `start` against a daemon over a fixture whose one agent can spawn, with
 * `kind`'s stand-in installed as `executable` (Claude runs the family's own
 * stand-in and session reporter) and Codex's install record seeded, once the
 * page shows the fixture's checkout.
 */
async function withStartStack(page: Page, kind: string, executable: string, start: (fixture: CallerFixture, daemon: Daemon) => Promise<void>): Promise<void> {
  const fixture = await startCaller();
  let daemon: Daemon | undefined;
  try {
    if (executable !== "claude") fixtureProgram(path.join(fixture.root, "bin"), executable, startProgram(fixture, kind));
    const home = kind === "codex" ? fixture.home ?? path.join(fixture.root, "home") : undefined;
    if (home) {
      // This is an existing installation, as in the desktop fixture's
      // seedKitRecord. The first-run dialog has its own flow tests.
      const kit = path.join(home, ".hide", "kit");
      fs.mkdirSync(kit, { recursive: true });
      fs.writeFileSync(path.join(kit, "installed.json"), JSON.stringify({ format: 1, installed: [] }));
    }
    daemon = await startHided(fixture, `start-${kind}`, home, {}, kind === "codex");
    await openPage(page, daemon, kind);
    await start(fixture, daemon);
  } catch (error) {
    throw afterCleanup(afterCleanup(error, () => daemon?.stop()), () => fixture.stop());
  }
  try { daemon?.stop(); } catch (error) { throw afterCleanup(error, () => fixture.stop()); }
  fixture.stop();
}

// Each start path is its own test on its own stack, so a slow runner spends
// one path's steps against the per-test budget, not both paths' and a second
// fixture agent's (with-hide/hide-private#516, #522).
for (const [kind, executable] of [
  ["claude", "claude"], ["codex", "codex"], ["grok", "grok"],
  ["opencode", "opencode"], ["pi", "pi"], ["omp", "omp"], ["agy", "agy"],
] as const) {
  test(`${kind} starts through spawn with a visible state`, { tag: "@platform" }, async ({ page }) => {
    await withStartStack(page, kind, executable, (fixture, daemon) => spawnStart(page, fixture, daemon, kind));
  });

  test(`${kind} starts from a typed command with a visible state`, { tag: "@platform" }, async ({ page }) => {
    await withStartStack(page, kind, executable, (fixture, daemon) => typedStart(page, fixture, daemon, kind, executable));
  });
}

for (const launcher of ["ps1", "cmd"] as const) {
  for (const kind of ["pi", "omp"] as const) {
    test(`Windows npm ${launcher} detects ${kind} beside another cli.js package`, { tag: "@platform" }, async ({ page }) => {
      test.skip(process.platform !== "win32", "Windows npm launchers require a real Windows shell and process table");
      const fixture = await startCaller();
      let daemon: Daemon | undefined;
      try {
        const bin = path.join(fixture.root, "bin");
        // Both packages remain installed so each case must retain the scope
        // that distinguishes their identical entry-point filenames.
        for (const [installedKind, scope] of [["pi", "@earendil-works"], ["omp", "@oh-my-pi"]] as const) {
          const script = path.join(bin, "node_modules", scope, "pi-coding-agent", "dist", "cli.js");
          fs.mkdirSync(path.dirname(script), { recursive: true });
          fs.writeFileSync(script, startProgram(fixture, installedKind));
          const psQuote = (text: string) => `'${text.replaceAll("'", "''")}'`;
          const body = launcher === "ps1"
            ? `& ${psQuote(process.execPath)} ${psQuote(script)} @args\r\nexit $LASTEXITCODE\r\n`
            : `@echo off\r\n"${process.execPath}" "${script}" %*\r\n`;
          fs.writeFileSync(path.join(bin, `${installedKind}.${launcher}`), body);
        }
        daemon = await startHided(fixture, `npm-${launcher}-${kind}`);
        // Herdr's ordinary fixture shell is cmd, which cannot directly
        // execute a .ps1. The own engine's PowerShell resolves it by name.
        const command = fixture.shell === "cmd" && launcher === "ps1"
          ? `powershell.exe -NoProfile -ExecutionPolicy Bypass -File "${path.join(bin, `${kind}.ps1`)}"`
          : kind;
        await openPage(page, daemon, kind);
        await spawnStart(page, fixture, daemon, kind);
        await typedStart(page, fixture, daemon, kind, command);
      } catch (error) {
        throw afterCleanup(afterCleanup(error, () => daemon?.stop()), () => fixture.stop());
      }
      try { daemon?.stop(); } catch (error) { throw afterCleanup(error, () => fixture.stop()); }
      fixture.stop();
    });
  }
}
