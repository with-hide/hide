import { createLegacyDraft } from "./group-start-helper";
// Unix account-shell regression: these cases intentionally select /bin/sh
// and /bin/bash, rather than the fixture server's configured zsh. Windows
// retains its separate PowerShell recipe and cannot exercise these shells.
import { expect, test, type Page } from "@playwright/test";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { toPage } from "../../desktop/src/main/wirePath";
import { herdrTabs } from "./failure-dump";
import { agentIn, checkoutAt, sendKeys, withControl } from "./daemon-actions";
import { startFixture, type AgentFixture } from "./engine-fixture";
import { startHerdr, type HerdrFixture } from "./herdr-fixture";
import { herdrGate } from "./herdr-gate";
import { startHided, type Daemon } from "./hided-fixture";
import { fixtureLoginShellPath } from "./platform-fixture";
import { quietFor } from "./wait";
import { enterWorkspace, sendEvent } from "./wire";

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

function privateShellEnv(home: string, fixturePath: string): Record<string, string> {
  fixtureLoginShellPath(home, fixturePath);
  const rc = path.join(home, "plain-shell.rc");
  fs.writeFileSync(rc, "PS1='fixture $ '\nexport PS1\n");
  // macOS /etc/bashrc overwrites an inherited PS1. The private account's
  // startup files restore it afterward; ENV covers interactive POSIX sh.
  for (const file of [".profile", ".bash_profile", ".bashrc"]) {
    fs.appendFileSync(path.join(home, file), "\nPS1='fixture $ '\nexport PS1\n");
  }
  return { PS1: "fixture $ ", BASH_SILENCE_DEPRECATION_WARNING: "1", ENV: rc };
}

/** The folder of the fixture's checkout, as the core spells it. */
const folderOf = (fixture: AgentFixture) => toPage(fs.realpathSync(path.join(fixture.root, "fixture")));

/** The tabs the fixture's checkout holds now, as the daemon publishes them. */
function tabsHeld(fixture: AgentFixture, daemon: Daemon): Promise<string[]> {
  return withControl(daemon, (control) => control.until((rest) => checkoutAt(rest, folderOf(fixture))?.tabs, "the checkout's tabs"));
}

/** The shell identity the saved start of `pane` records, once the start is bound to it. */
async function shellIdentity(daemon: Daemon, pane: string): Promise<Identity> {
  let found: Identity | undefined;
  await expect.poll(() => (found = starts(daemon).find((start) => start.effect?.binding?.pane_id === pane)?.effect?.binding?.identity)).toBeTruthy();
  expect(found!.shell_pid).toBeGreaterThan(0);
  // The shell the start recorded is still the one running.
  expect(() => process.kill(found!.shell_pid, 0)).not.toThrow();
  return found!;
}

/**
 * The HOME the pane's shell reads its startup files from: a private Herdr's
 * panes keep the server's, the own engine's are children of the daemon, whose
 * HOME is `plain`.
 */
function shellHome(fixture: AgentFixture, plain: string): string {
  return fixture.engine === "herdr" ? (fixture as AgentFixture & Pick<HerdrFixture, "env">).env.HOME! : plain;
}

function paneText(page: Page, pane: string): Promise<string> {
  return page.evaluate((id) => window.__hideProbe?.paneText(id) ?? "", pane);
}

async function typesInto(page: Page, daemon: Daemon, pane: string, suffix = "INPUT_OK") {
  // The contiguous expected output never appears in the typed command, so
  // terminal echo alone cannot satisfy this assertion.
  await withControl(daemon, async (control) => sendKeys(control, pane, `printf '%s%s\\n' PLAIN_ ${suffix}\r`));
  let output = "";
  await expect.poll(async () => (output = await paneText(page, pane))).toContain(`PLAIN_${suffix}`);
  expect(output).toContain("fixture $ ");
}

async function agentRunning(page: Page, daemon: Daemon, herdr: AgentFixture, pane: string, prompt: string) {
  await withControl(daemon, (control) => control.until((rest) => agentIn(rest, pane)?.agent_kind === "claude" || undefined, `a Claude agent in ${pane}`, 30_000));
  await expect.poll(async () => paneText(page, pane), { timeout: 30_000 }).toContain(prompt);
  // Herdr also names the agent's command line, which carries the model; the own engine publishes no process arguments to read.
  if (herdr.engine !== "herdr") return;
  const direct = herdr as AgentFixture & HerdrFixture;
  await expect.poll(() => JSON.stringify(direct.run(["pane", "process-info", "--pane", pane])), { timeout: 30_000 }).toContain("opus");
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
    const herdr = await startFixture({ agents: false });
    let gate: Awaited<ReturnType<typeof herdrGate>> | undefined;
    let daemon: Daemon | null = null;
    let failure: unknown;
    try {
      // The own engine's panes read the HOME its fixture made; a private Herdr's daemon runs in a home of its own.
      const home = herdr.engine === "own" ? herdr.home! : path.join(herdr.root, "plain-home");
      fs.mkdirSync(path.join(home, ".hide"), { recursive: true });
      const shellEnv = privateShellEnv(shellHome(herdr, home), herdr.fixturePath);
      // A pane made by the product reaches the private shell's environment by the engine's own route.
      if (herdr.engine === "herdr") gate = await herdrGate(herdr as AgentFixture & HerdrFixture, shellEnv);
      const blocked = path.join(home, ".hide", "shell-integration");
      if (scenario.blockIntegration) fs.writeFileSync(blocked, "integration directory deliberately unavailable\n");
      daemon = await startHided(herdr, "group-plain-shell", home, { SHELL: scenario.shell, ...(gate ? { HERDR_SOCKET_PATH: gate.socket } : shellEnv) });
      // The own engine's panes read the daemon's HOME, whose login files the daemon's start rewrote.
      if (herdr.engine === "own") privateShellEnv(home, herdr.fixturePath);
      await open(page, daemon);
      await createGroup(page, "Plain terminal");
      const pane = await startTerminal(page);
      await expect(selector(page)).toContainText("Plain terminal");
      await typesInto(page, daemon, pane);
      const actual = await shellIdentity(daemon, pane);
      await expect.poll(() => starts(daemon!).find(start => start.effect?.binding?.pane_id === pane)).toMatchObject({
        state: "ready",
        effect: { shell_readiness: "plain", expected_command: plainCommand(scenario.shell), binding: { pane_id: pane, identity: actual } },
      });
      // Herdr names the same terminal and shell process for the pane as the start recorded.
      if (herdr.engine === "herdr") expect(identity(herdr as AgentFixture & HerdrFixture, pane)).toEqual({ terminal_id: actual.terminal_id, shell_pid: actual.shell_pid });
      const tab = (await stripTabs(page).getAttribute("data-tab"))!;
      const originalTabs = await tabsHeld(herdr, daemon);
      // D-01 removes Add agent. Exercise the preserved start lifecycle
      // through its public event instead of inventing another UI entry.
      const group = await selector(page).getAttribute("data-agent-group-selector");
      const prompt = "Plain shell added agent prompt";
      await sendEvent(page, daemon, "agent_start_in_checkout", { checkout_path: path.join(herdr.root, "fixture"), provider: "claude", model: "opus", prompt, request_id: "start-plain-add", target: { group_id: group, area_id: "a1", tab_id: tab, add_pane: true } });
      await expect(page.locator("[data-pane-view]")).toHaveCount(2);
      await expect(page.locator(`[data-pane-view="${pane}"]`)).toBeVisible();
      await expect(stripTabs(page)).toHaveAttribute("data-tab", tab);
      const added = (await page.locator("[data-pane-view]").evaluateAll(panes => panes.map(pane => pane.getAttribute("data-pane-view")!))).find(id => id !== pane)!;
      await agentRunning(page, daemon, herdr, added, prompt);
      expect(await shellIdentity(daemon, pane)).toEqual(actual);
      if (herdr.engine === "herdr") expect(identity(herdr as AgentFixture & HerdrFixture, pane)).toEqual({ terminal_id: actual.terminal_id, shell_pid: actual.shell_pid });
      expect((await tabsHeld(herdr, daemon)).sort()).toEqual(originalTabs.sort());
      await typesInto(page, daemon, pane, "AFTER_AGENT_ADD");
      await expect.poll(() => starts(daemon!).find(start => start.effect?.binding?.pane_id === added)).toMatchObject({
        state: "ready",
        effect: { shell_readiness: "plain", expected_command: plainCommand(scenario.shell) },
      });
      if (!scenario.blockIntegration) {
        await createGroup(page, "First plain agent");
        await createLegacyDraft(page, daemon!, path.join(herdr.root, "fixture"));
        await expect(blank(page)).toHaveAttribute("data-agent-tab-start", /^draft:/);
        const draft = await blank(page).getAttribute("data-agent-tab-start");
        const firstPrompt = "Plain shell first draft agent prompt";
        await startClaude(page, firstPrompt);
        await expect(stripTabs(page)).toHaveCount(1);
        await expect(page.locator(`[data-tab="${draft}"]`)).toHaveCount(0);
        await expect(page.locator("[data-pane-view]")).toHaveCount(1);
        await expect(selector(page)).toContainText("First plain agent");
        const first = (await page.locator("[data-pane-view]").getAttribute("data-pane-view"))!;
        await agentRunning(page, daemon, herdr, first, firstPrompt);
        expect(await tabsHeld(herdr, daemon)).toHaveLength(originalTabs.length + 1);
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

// Herdr only: it holds Herdr's layout.apply answer, ends the daemon, and checks the restarted daemon adopts the shell Herdr made.
test("a plain sh start with a lost answer adopts the same shell after daemon restart @herdr", async ({ page }) => {
  const herdr = await startHerdr({ agents: false });
  let gate: Awaited<ReturnType<typeof herdrGate>> | undefined;
  let daemon: Daemon | null = null;
  let failure: unknown;
  try {
    gate = await herdrGate(herdr, privateShellEnv(herdr.env.HOME!, herdr.fixturePath));
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
