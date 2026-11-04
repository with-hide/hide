// @platform: Starts cross the real shell and native process boundary. The
// Windows cases also exercise npm's PowerShell and cmd launchers over Node.
import fs from "node:fs";
import path from "node:path";
import { expect, test, type Page } from "@playwright/test";
import { agentIn, newTab, preparePaneInput, sendKeys, withControl } from "./daemon-actions";
import { startFixture, type AgentFixture } from "./engine-fixture";
import { spawnRoot } from "./fixture-agents";
import { startHided, type Daemon } from "./hided-fixture";
import { fixtureExecutable } from "./platform-fixture";
import { copyFixtureShim } from "./shims/build";
import { enterWorkspace } from "./wire";
import { afterCleanup } from "./worker-owned";

/** The process is recognized, its initial Starting state has ended, and
 * the sidebar renders that same state for this pane. Unknown is meaningful
 * for a definition without screen rules, such as omp without its hooks. */
async function listed(page: Page, daemon: Daemon, pane: string, kind: string): Promise<void> {
  // The stand-ins echo the captured resting screens used by the rules'
  // contract tests. omp has no screen rules and reports no hooks here.
  const expected = kind === "omp" ? "unknown" : "idle";
  await withControl(daemon, async (control) => {
    await control.until((rest) => agentIn(rest, pane)?.agent_kind === kind || undefined,
      `${kind} process in ${pane}`, 30_000);
    if (kind !== "omp") {
      const definition = ({ claude: "claude-code", agy: "antigravity" } as Record<string, string>)[kind] ?? kind;
      const screen = fs.readFileSync(path.resolve("..", "hide-agent-adapter", "tests", "fixtures", "screens", definition, "idle-empty-composer.ansi"), "utf8");
      await preparePaneInput(control, pane);
      sendKeys(control, pane, `\x1b[2J\x1b[H${screen}`);
    }
  });
  await withControl(daemon, (control) => control.until((rest) => {
    const agent = agentIn(rest, pane);
    return agent?.agent_kind === kind && agent.status_code === expected ? agent : undefined;
  }, `${kind} ${expected} in ${pane}`, 30_000));
  const status = page.locator(`nav[data-sidebar] [data-pane="${pane}"] [data-agent-status]`);
  await expect(status).toBeVisible();
  await expect(status).toHaveAttribute("data-agent-status", expected);
}

const resumeFlags: Record<string, string> = { codex: "resume", opencode: "-s", pi: "--session", agy: "--conversation" };

async function bothStarts(page: Page, fixture: AgentFixture, daemon: Daemon, kind: string, command: string): Promise<void> {
  await page.goto(`${daemon.origin}/#token=${daemon.token}`);
  await enterWorkspace(page, "fixture");
  const spawned = await spawnRoot(fixture, daemon, fixture.panes[0], {
    name: `start-${kind}`, kind, repo: path.join(fixture.root, "fixture"),
    // These plain native stand-ins report no session hooks. Resuming an
    // explicit session gives spawn its required identity; the typed start
    // below exercises a fresh process without any declared session.
    nativeArgs: [resumeFlags[kind] ?? "--resume", `platform-${kind}`],
  });
  await listed(page, daemon, spawned, kind);
  const typed = await withControl(daemon, async (control) => {
    const { pane } = await newTab(control, fixture, `typed-${kind}`);
    await preparePaneInput(control, pane);
    sendKeys(control, pane, `${command}\r`);
    return pane;
  });
  await listed(page, daemon, typed, kind);
}

for (const [kind, executable] of [
  ["claude", "claude"], ["codex", "codex"], ["grok", "grok"],
  ["opencode", "opencode"], ["pi", "pi"], ["omp", "omp"], ["agy", "agy"],
] as const) {
  test(`${kind} starts through spawn and a typed command with a visible state`, { tag: "@platform" }, async ({ page }) => {
    const fixture = await startFixture({ family: true });
    let daemon: Daemon | undefined;
    try {
      if (executable !== "claude") copyFixtureShim("claude-shim", path.join(fixture.root, "bin", fixtureExecutable(executable)));
      daemon = await startHided(fixture, `start-${kind}`);
      // The family's existing caller keeps its runner. New processes run
      // their own native executable, without becoming a Claude caller.
      fs.unlinkSync(path.join(fixture.root, "agent-pane-runner.config"));
      await bothStarts(page, fixture, daemon, kind, executable);
    } catch (error) {
      throw afterCleanup(afterCleanup(error, () => daemon?.stop()), () => fixture.stop());
    }
    try { daemon?.stop(); } catch (error) { throw afterCleanup(error, () => fixture.stop()); }
    fixture.stop();
  });
}

for (const launcher of ["ps1", "cmd"] as const) {
  test(`Windows npm ${launcher} launchers distinguish packages sharing cli.js`, { tag: "@platform" }, async ({ page }) => {
    test.skip(process.platform !== "win32", "Windows npm launchers require a real Windows shell and process table");
    const fixture = await startFixture({ family: true });
    let daemon: Daemon | undefined;
    try {
      const bin = path.join(fixture.root, "bin");
      for (const [kind, scope] of [["pi", "@earendil-works"], ["omp", "@oh-my-pi"]] as const) {
        const script = path.join(bin, "node_modules", scope, "pi-coding-agent", "dist", "cli.js");
        fs.mkdirSync(path.dirname(script), { recursive: true });
        fs.writeFileSync(script, `process.stdin.setRawMode(true);\nprocess.stdin.on("data", bytes => process.stdout.write(bytes));\nprocess.stdout.write("${kind} fixture ready\\r\\n");\n`);
        const psQuote = (text: string) => `'${text.replaceAll("'", "''")}'`;
        const body = launcher === "ps1"
          ? `& ${psQuote(process.execPath)} ${psQuote(script)} @args\r\nexit $LASTEXITCODE\r\n`
          : `@echo off\r\n"${process.execPath}" "${script}" %*\r\n`;
        fs.writeFileSync(path.join(bin, `${kind}.${launcher}`), body);
      }
      daemon = await startHided(fixture, `npm-${launcher}`);
      fs.unlinkSync(path.join(fixture.root, "agent-pane-runner.config"));
      for (const kind of ["pi", "omp"]) {
        // Herdr's ordinary fixture shell is cmd, which cannot directly
        // execute a .ps1. The own engine's PowerShell resolves it by name.
        const command = fixture.shell === "cmd" && launcher === "ps1"
          ? `powershell.exe -NoProfile -ExecutionPolicy Bypass -File "${path.join(bin, `${kind}.ps1`)}"`
          : kind;
        await bothStarts(page, fixture, daemon, kind, command);
      }
    } catch (error) {
      throw afterCleanup(afterCleanup(error, () => daemon?.stop()), () => fixture.stop());
    }
    try { daemon?.stop(); } catch (error) { throw afterCleanup(error, () => fixture.stop()); }
    fixture.stop();
  });
}
