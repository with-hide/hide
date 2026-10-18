// Unix account-shell regression: these cases intentionally select /bin/sh
// and /bin/bash, rather than the fixture server's configured zsh. Windows
// retains its separate PowerShell recipe and cannot exercise these shells.
import { expect, test, type Page } from "@playwright/test";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { herdrTabs } from "./failure-dump";
import { startHerdr, type HerdrFixture } from "./herdr-fixture";
import { herdrGate } from "./herdr-gate";
import { startHided, type Daemon } from "./hided-fixture";
import { fixtureLoginShellPath } from "./platform-fixture";
import { quietFor } from "./wait";
import { enterWorkspace } from "./wire";

test.describe.configure({ timeout: 150_000 });
test.use({ actionTimeout: 15_000 });
test.skip(process.platform === "win32", "Unix account shells and exec-login semantics; Windows uses the PowerShell recipe");
test.skip(!fs.existsSync("/bin/sh"), "This regression requires the selected /bin/sh executable");

type Identity = { terminal_id: string; shell_pid: number };
type Start = {
  state: string;
  marker: string;
  effect?: {
    workspace_id: string;
    expected_command: string[] | null;
    shell_readiness: string;
    binding: { tab_id: string; pane_id: string; identity: Identity } | null;
  };
};

const blank = (page: Page) => page.locator("[data-agent-tab-start]");
const terminal = (page: Page) => page.locator("[data-group-start-terminal=true]");
const selector = (page: Page) => page.locator("[data-agent-group-selector]");
const stripTabs = (page: Page) => page.locator("[data-agent-tab-bar] [role=tab][data-tab]");

function starts(daemon: Daemon): Start[] {
  const file = path.join(daemon.stateDir, "workspace-views.json");
  if (!fs.existsSync(file)) return [];
  const saved = JSON.parse(fs.readFileSync(file, "utf8")) as { workspaces: { agent_starts?: Record<string, Start> }[] };
  return saved.workspaces.flatMap(view => Object.values(view.agent_starts ?? {}));
}

function topology(herdr: HerdrFixture): string[] {
  return (herdrTabs(herdr) as { tabs: string[] }[]).flatMap(workspace => workspace.tabs);
}

function identity(herdr: HerdrFixture, pane: string): Identity {
  const row = herdr.run(["pane", "get", pane]) as { result: { pane: { terminal_id: string } } };
  const info = herdr.run(["pane", "process-info", "--pane", pane]) as { result: { process_info: { shell_pid: number } } };
  return { terminal_id: row.result.pane.terminal_id, shell_pid: info.result.process_info.shell_pid };
}

function plainCommand(shell: string): string[] {
  // This is the platform-specific command contract under test, not a command
  // used to bootstrap the fixture. macOS needs argv[0] login-shell spelling.
  return process.platform === "darwin"
    ? [fs.realpathSync(path.resolve("..", "target", "debug", "hided")), "shell-exec", "--shell", shell, "--login"]
    : [shell];
}

function privateShellEnv(herdr: HerdrFixture): Record<string, string> {
  const home = herdr.env.HOME!;
  fixtureLoginShellPath(home, herdr.fixturePath);
  const rc = path.join(home, "plain-shell.rc");
  fs.writeFileSync(rc, "PS1='fixture $ '\nexport PS1\n");
  // macOS /etc/bashrc overwrites an inherited PS1. The private account's
  // startup files restore it afterward; ENV covers interactive POSIX sh.
  for (const file of [".profile", ".bash_profile", ".bashrc"]) {
    fs.appendFileSync(path.join(home, file), "\nPS1='fixture $ '\nexport PS1\n");
  }
  return { PS1: "fixture $ ", BASH_SILENCE_DEPRECATION_WARNING: "1", ENV: rc };
}

async function open(page: Page, daemon: Daemon) {
  await page.goto("about:blank");
  await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
  await enterWorkspace(page, "fixture");
  await expect(selector(page)).toBeVisible();
}

async function createGroup(page: Page, name: string) {
  await selector(page).click();
  await page.getByRole("menuitem", { name: "New group…", exact: true }).click();
  await page.getByRole("textbox", { name: "Group name", exact: true }).fill(name);
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(selector(page)).toContainText(name);
  await expect(blank(page)).toBeVisible();
}

async function startTerminal(page: Page): Promise<string> {
  await terminal(page).click();
  await expect(blank(page)).toHaveCount(0, { timeout: 30_000 });
  await expect(page.locator("[data-pane-view]")).toHaveCount(1);
  return (await page.locator("[data-pane-view]").getAttribute("data-pane-view"))!;
}

async function acceptsInput(herdr: HerdrFixture, pane: string, suffix = "INPUT_OK") {
  // The contiguous expected output never appears in the typed command, so
  // terminal echo alone cannot satisfy this assertion.
  const sent = spawnSync(herdr.bin, ["pane", "run", pane, `printf '%s%s\\n' PLAIN_ ${suffix}`], { env: herdr.env, encoding: "utf8", timeout: 10_000 });
  expect(sent.status, sent.stderr).toBe(0);
  let output = "";
  await expect.poll(() => {
    const read = spawnSync(herdr.bin, ["pane", "read", pane, "--source", "recent-unwrapped", "--lines", "80", "--format", "text"], { env: herdr.env, encoding: "utf8", timeout: 10_000 });
    expect(read.status, read.stderr).toBe(0);
    output = read.stdout;
    return output;
  }).toContain(`PLAIN_${suffix}`);
  expect(output).toContain("fixture $ ");
}

async function startClaude(page: Page, prompt: string) {
  await expect(blank(page)).toBeVisible();
  await blank(page).locator("[data-agent-kind]").click();
  await expect(page.locator('[data-agent-kind-option="claude"]')).toBeEnabled({ timeout: 20_000 });
  await page.locator('[data-agent-kind-option="claude"]').click();
  const model = blank(page).locator("[data-agent-model-kind]");
  await expect(model).toBeEnabled();
  await model.click();
  await page.locator('[data-agent-model-option="opus"]').click();
  await blank(page).getByRole("textbox").fill(prompt);
  await page.locator("[data-group-start-agent=true]").click();
  await expect(blank(page)).toHaveCount(0, { timeout: 30_000 });
}

async function claudeIsRunning(herdr: HerdrFixture, pane: string, prompt: string) {
  await expect.poll(() => {
    const row = herdr.run(["pane", "get", pane]) as { result: { pane: { agent: string | null } } };
    return row.result.pane.agent;
  }, { timeout: 30_000 }).toBe("claude");
  await expect.poll(() => JSON.stringify(herdr.run(["pane", "process-info", "--pane", pane])), { timeout: 30_000 }).toContain("opus");
  await expect.poll(() => {
    // Herdr removes only terminal soft wrapping. Preserve the exact prompt,
    // including its spaces, rather than normalizing arbitrary output text.
    const read = spawnSync(herdr.bin, ["pane", "read", pane, "--source", "recent-unwrapped", "--lines", "80", "--format", "text"], { env: herdr.env, encoding: "utf8", timeout: 10_000 });
    expect(read.status, read.stderr).toBe(0);
    return read.stdout;
  }, { timeout: 30_000 }).toContain(prompt);
}

async function cleanup(primary: unknown, steps: (() => void | Promise<void>)[]) {
  const errors: unknown[] = primary === undefined ? [] : [primary];
  for (const step of steps) {
    try { await step(); } catch (error) { errors.push(error); }
  }
  if (errors.length === 1) throw errors[0];
  if (errors.length > 1) throw new AggregateError(errors, "Plain-shell regression or private-fixture cleanup failed");
}

for (const scenario of [
  { name: "a plain sh group terminal accepts input and saves its exact launch", shell: "/bin/sh", blockIntegration: false },
  { name: "an integration install failure starts a plain bash group terminal", shell: "/bin/bash", blockIntegration: true },
]) {
  test(scenario.name, async ({ page }) => {
    test.skip(!fs.existsSync(scenario.shell), `This regression requires ${scenario.shell}`);
    const herdr = await startHerdr({ agents: false });
    let gate: Awaited<ReturnType<typeof herdrGate>> | undefined;
    let daemon: Daemon | null = null;
    let failure: unknown;
    try {
      gate = await herdrGate(herdr, privateShellEnv(herdr));
      const home = path.join(herdr.root, "plain-home");
      fs.mkdirSync(path.join(home, ".hide"), { recursive: true });
      const blocked = path.join(home, ".hide", "shell-integration");
      if (scenario.blockIntegration) fs.writeFileSync(blocked, "integration directory deliberately unavailable\n");
      daemon = await startHided(herdr, "group-plain-shell", home, { SHELL: scenario.shell, HERDR_SOCKET_PATH: gate.socket });
      await open(page, daemon);
      await createGroup(page, "Plain terminal");
      const pane = await startTerminal(page);
      await expect(selector(page)).toContainText("Plain terminal");
      await acceptsInput(herdr, pane);
      const actual = identity(herdr, pane);
      expect(actual.shell_pid).toBeGreaterThan(0);
      await expect.poll(() => starts(daemon!).find(start => start.effect?.binding?.pane_id === pane)).toMatchObject({
        state: "ready",
        effect: { shell_readiness: "plain", expected_command: plainCommand(scenario.shell), binding: { pane_id: pane, identity: actual } },
      });
      const tab = (await stripTabs(page).getAttribute("data-tab"))!;
      const originalTabs = topology(herdr);
      await page.locator(`[data-add-agent="${tab}"]`).click();
      const prompt = "Plain shell added agent prompt";
      await startClaude(page, prompt);
      await expect(page.locator("[data-pane-view]")).toHaveCount(2);
      await expect(page.locator(`[data-pane-view="${pane}"]`)).toBeVisible();
      await expect(stripTabs(page)).toHaveAttribute("data-tab", tab);
      const added = (await page.locator("[data-pane-view]").evaluateAll(panes => panes.map(pane => pane.getAttribute("data-pane-view")!))).find(id => id !== pane)!;
      await claudeIsRunning(herdr, added, prompt);
      expect(identity(herdr, pane)).toEqual(actual);
      expect(topology(herdr).sort()).toEqual(originalTabs.sort());
      await acceptsInput(herdr, pane, "AFTER_AGENT_ADD");
      await expect.poll(() => starts(daemon!).find(start => start.effect?.binding?.pane_id === added)).toMatchObject({
        state: "ready",
        effect: { shell_readiness: "plain", expected_command: plainCommand(scenario.shell) },
      });
      if (!scenario.blockIntegration) {
        await createGroup(page, "First plain agent");
        await page.locator("[data-new-agent-tab=true]").click();
        await expect(blank(page)).toHaveAttribute("data-agent-tab-start", /^draft:/);
        const draft = await blank(page).getAttribute("data-agent-tab-start");
        const firstPrompt = "Plain shell first draft agent prompt";
        await startClaude(page, firstPrompt);
        await expect(stripTabs(page)).toHaveCount(1);
        await expect(page.locator(`[data-tab="${draft}"]`)).toHaveCount(0);
        await expect(page.locator("[data-pane-view]")).toHaveCount(1);
        await expect(selector(page)).toContainText("First plain agent");
        const first = (await page.locator("[data-pane-view]").getAttribute("data-pane-view"))!;
        await claudeIsRunning(herdr, first, firstPrompt);
        expect(topology(herdr)).toHaveLength(originalTabs.length + 1);
        await expect.poll(() => starts(daemon!).find(start => start.effect?.binding?.pane_id === first)).toMatchObject({
          state: "ready",
          effect: { shell_readiness: "plain", expected_command: plainCommand(scenario.shell) },
        });
      }
      if (scenario.blockIntegration) expect(fs.statSync(blocked).isFile()).toBe(true);
    } catch (error) { failure = error; }
    finally { await cleanup(failure, [() => daemon?.stop(), async () => { await gate?.stop(); }, () => herdr.stop()]); }
  });
}

test("a plain sh start with a lost answer adopts the same shell after daemon restart", async ({ page }) => {
  const herdr = await startHerdr({ agents: false });
  let gate: Awaited<ReturnType<typeof herdrGate>> | undefined;
  let daemon: Daemon | null = null;
  let failure: unknown;
  try {
    gate = await herdrGate(herdr, privateShellEnv(herdr));
    daemon = await startHided(herdr, "group-plain-recover", undefined, { SHELL: "/bin/sh", HERDR_SOCKET_PATH: gate.socket });
    await open(page, daemon);
    await createGroup(page, "Owner");
    await startTerminal(page);
    const originalTabs = topology(herdr);
    await createGroup(page, "Plain recovery");
    const held = gate.arm("layout.apply");
    await terminal(page).click();
    await held;
    const pending = starts(daemon).filter(start => start.state === "pending");
    expect(pending).toHaveLength(1);
    expect(pending[0]!.effect).toMatchObject({ shell_readiness: "plain", expected_command: plainCommand("/bin/sh"), binding: null });
    const claim = pending[0]!.marker;
    await page.goto("about:blank");
    const previousPid = daemon.pid;
    process.kill(previousPid, "SIGKILL");
    await expect.poll(() => { try { process.kill(previousPid, 0); return true; } catch { return false; } }).toBe(false);
    let created!: { tab: string; pane: string; identity: Identity };
    daemon = await daemon.restart(async () => {
      // Forward once to real Herdr only after the caller died. Capture the
      // actual incarnation before a new daemon can attempt confirmation.
      await gate!.release();
      const added = topology(herdr).filter(tab => !originalTabs.includes(tab));
      expect(added).toHaveLength(1);
      const panes = (herdr.run(["pane", "list"]) as { result: { panes: { tab_id: string; pane_id: string }[] } }).result.panes.filter(pane => pane.tab_id === added[0]);
      expect(panes).toHaveLength(1);
      created = { tab: added[0]!, pane: panes[0]!.pane_id, identity: identity(herdr, panes[0]!.pane_id) };
      expect(created.identity.shell_pid).toBeGreaterThan(0);
      await acceptsInput(herdr, created.pane, "BEFORE_RECOVERY");
    });
    await open(page, daemon);
    await page.getByRole("button", { name: "Check start status", exact: true }).click();
    await expect(blank(page)).toHaveCount(0, { timeout: 30_000 });
    await expect(page.locator("[data-pane-view]")).toHaveCount(1);
    await expect(page.locator("[data-pane-view]")).toHaveAttribute("data-pane-view", created.pane);
    await expect.poll(() => starts(daemon!).find(start => start.marker === claim)).toMatchObject({
      state: "ready",
      effect: { binding: { tab_id: created.tab, pane_id: created.pane, identity: created.identity } },
    });
    expect(identity(herdr, created.pane)).toEqual(created.identity);
    await acceptsInput(herdr, created.pane, "AFTER_RECOVERY");
    await quietFor(page, 300, "confirming the restored plain-shell intent must not create a replacement shell");
    expect(topology(herdr).sort()).toEqual([...originalTabs, created.tab].sort());
    expect(gate.params("layout.apply")).toHaveLength(1);
  } catch (error) { failure = error; }
  finally { await cleanup(failure, [() => daemon?.stop(), async () => { await gate?.stop(); }, () => herdr.stop()]); }
});
