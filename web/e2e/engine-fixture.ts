// The engine a web e2e spec runs its panes on: the core's own engine, a
// daemon whose node starts each pane's holder (PRD engine-bundle-4-herdr-removal
// D-2, D-15). The fixture makes its workspace and panes with the events the
// web shell sends (`hided-control.mjs`) and reads them back from the
// snapshot.

import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { expect, type Page } from "@playwright/test";
import { connectControl, type HidedControl } from "./hided-control.mjs";
import { agentIn, areaShowing, closePane, focusPane, openProject, splitPane, startAgentInCheckout, startedAgentIn, tabOfPane, tabPanes, viewPane, waitForPaneInput } from "./daemon-actions";
import { claudeProjects, panePrompt, paneShellOf, writeFixtureTranscript, type PaneShell } from "./pane-fixture";
import { startHided, type Daemon, type DaemonEngine, type OpenedDaemon } from "./hided-fixture";
import { fixtureExecutable, fixturePrograms } from "./platform-fixture";
import { installAgentRunner, reportedMarker, type AgentPaneFixture, type AgentRun, type HideAnswer } from "./agent-pane";

/** Runs one `hide` command as an agent from inside its pane. */
export type HideRunner = (args: string[], run?: AgentRun) => Promise<HideAnswer>;
import { afterCleanup, ownUntilWorkerExit } from "./worker-owned";
import { enterWorkspace } from "./wire";
import { git } from "./work-prs-fixture";

/** What the daemon runs its panes on, and the private root the run's files live under. */
export type EngineServer = DaemonEngine & {
  /** The shell family a pane runs, for commands typed into it. */
  shell: PaneShell;
  /** The HOME the daemon must run in, when the engine's panes read their shell's files from it. */
  home?: string;
  stop: () => void;
};

/** What the engine's daemon needs, empty. */
export async function startEngine(): Promise<EngineServer> {
  return startOwnEngine();
}

/**
 * The own engine's side of the fixture: no server, since the daemon's node
 * starts each pane's holder itself, only the private root, whose `home` the
 * daemon runs in, and the pane shell's environment, which the daemon hands
 * its panes. The shell is the one the removed Herdr fixture named, with the
 * same fixed prompt.
 */
function startOwnEngine(): EngineServer {
  const root = fs.realpathSync.native(fs.mkdtempSync(path.join(os.tmpdir(), "hide-e2e-own-")));
  const after: (() => void)[] = [];
  let stopped = false;
  const { stop } = ownUntilWorkerExit(() => {
    stopped = true;
    const failed: unknown[] = [];
    for (const cleanup of after.splice(0)) {
      try { cleanup(); } catch (error) { failed.push(error); }
    }
    if (failed.length > 0) throw failed[0];
    fs.rmSync(root, { recursive: true, force: true });
  });
  try {
    for (const dir of ["home", "fixture", "bin"]) fs.mkdirSync(path.join(root, dir), { recursive: true });
    const shell = process.platform === "win32" ? process.env.COMSPEC : "/bin/zsh";
    if (!shell || !path.isAbsolute(shell) || !fs.existsSync(shell)) throw new Error("fixture shell is unavailable: Windows needs ComSpec; Unix needs /bin/zsh");
    fs.writeFileSync(path.join(root, "home", ".zshrc"), "PS1='fixture %# '\n");
    const fixturePath = fixturePrograms(path.join(root, "bin"));
    const daemonEnv: NodeJS.ProcessEnv = {
      SHELL: shell,
      ...(process.platform === "win32" ? { PROMPT: "fixture % " } : {}),
      // Ubuntu's /etc/zsh/zshrc stops at compinit's question before this HOME's .zshrc.
      skip_global_compinit: "1",
      HIDE_E2E_ROOT: root,
    };
    return {
      root,
      fixturePath,
      daemonEnv,
      // A pane's shell is a child of the daemon's node, in the daemon's HOME.
      home: path.join(root, "home"),
      shell: process.platform === "win32" ? "powershell" : paneShellOf(daemonEnv),
      stop,
      afterStop: (cleanup) => {
        if (stopped) cleanup();
        else after.push(cleanup);
      },
    };
  } catch (error) {
    throw afterCleanup(error, stop);
  }
}

/**
 * One checkout, the folder `fixture` under the run's root, whose one tab
 * holds `panes` shells side by side (two by default), made through the
 * daemon. `control` is the fixture's own connection, for a
 * spec that reads what the core published.
 */
export type LayoutStack = {
  server: EngineServer;
  daemon: Daemon;
  control: HidedControl;
  /** The checkout's folder, as the core spells it on the wire. */
  folder: string;
  workspace: string;
  checkout: string;
  tab: string;
  panes: string[];
  stop: () => void;
};

export async function startLayoutStack({ panes = 2, label = "layout" }: { panes?: 1 | 2; label?: string } = {}): Promise<LayoutStack> {
  const server = await startEngine();
  let daemon: Daemon | null = null;
  let control: HidedControl | null = null;
  const stop = () => {
    control?.close();
    try { daemon?.stop(); } finally { server.stop(); }
  };
  try {
    daemon = await startHided(server, label, server.home);
    control = await connectControl(daemon);
    // The run's root is outside the daemon's HOME, so the registration carries
    // the confirmation Add project sends once the operator has given it.
    const made = await openProject(control, path.join(server.root, "fixture"), "fixture");
    const ids = [made.pane];
    if (panes === 2) ids.push(await splitPane(control, made.tab, made.folder));
    return { server, daemon, control, folder: made.folder, workspace: made.workspace, checkout: made.checkout, tab: made.tab, panes: ids, stop };
  } catch (error) {
    throw afterCleanup(error, stop);
  }
}

/**
 * Opens the shell on the stack's checkout, with the probe, and returns once
 * every pane of its tab is drawn with the fixture's prompt.
 */
export async function openLayout(page: Page, stack: LayoutStack): Promise<void> {
  await page.goto(`${stack.daemon.origin}/?probe=1#token=${stack.daemon.token}`);
  await enterWorkspace(page, "fixture");
  await expect(page.locator("[data-canvas]")).toHaveAttribute("data-canvas", stack.tab);
  await expect(page.locator("[data-pane-view]")).toHaveCount(stack.panes.length);
  for (const pane of stack.panes) {
    await expect.poll(() => page.evaluate((id) => (window as unknown as { __hideProbe?: { paneText: (id: string) => string } }).__hideProbe?.paneText(id) ?? "", pane), { timeout: 20_000, message: `the prompt in ${pane}` }).toMatch(panePrompt(stack.server.shell));
  }
}

/** The file the `claude` shim in `pane` logs every byte its terminal delivered to, under the run's root (`HIDE_E2E_ROOT`). */
export function paneInputLog(root: string, pane: string): string {
  return path.join(root, `input-${pane.replaceAll(":", "-")}.log`);
}

/** The `hide-agent-hooks` this worktree built, which a stand-in runs from inside its pane as the agent's own integration does. */
export function builtHooks(): string {
  return path.resolve("..", "target", "debug", fixtureExecutable("hide-agent-hooks"));
}

/** The session each of the fixture's two agents runs, whose transcript names its task. */
const FIXTURE_AGENTS = [
  { session: "fixture-one", task: "Agent one" },
  { session: "fixture-two", task: "Agent two" },
] as const;

/**
 * The default fixture of a spec that looks at Hide rather than at an engine:
 * one checkout, the folder `fixture`
 * under the run's root, whose one tab holds two panes side by side, each
 * running the `claude` shim on a session whose transcript names it
 * `Agent one` and `Agent two` (`agents: false` leaves two shells).
 * Everything in it is made through the first daemon `startHided` starts on
 * it, with the events the web shell sends: Add project, then the start
 * panel's start twice, the second beside the first. So `workspace`, `tab`,
 * `panes` and `inputLogs` are read only after `startHided` returned.
 */
export type AgentFixture = EngineServer & {
  /** Whether the second agent is the first one's child (`family`), which opens in a tab of its own; `tab` is the parent's. */
  family: boolean;
  workspace: string;
  checkout: string;
  tab: string;
  panes: [string, string];
  /** Per pane, the file its `claude` shim appends every byte its terminal delivered to. */
  inputLogs: [string, string];
  /** Resolves at once: the daemon that made the agents waited for both to run. */
  ready: () => Promise<void>;
  /** With `family`, the way to run `hide` as the agent in `pane` (`agent-pane.ts`). */
  runAs: (pane: string) => HideRunner;
};

export async function startFixture({ agents = true, family = false }: { agents?: boolean; family?: boolean } = {}): Promise<AgentFixture> {
  const server = await startEngine();
  const { opened, need } = openedByFirstDaemon(server, (control, daemon) => openFixture(control, server, daemon, family ? "family" : agents ? "agents" : "shells"));
  return {
    ...server,
    // The stand-in reports a new session through Hide's own hook from inside its pane (`setAgentSession`).
    daemonEnv: { ...server.daemonEnv, HIDE_E2E_HOOKS: builtHooks() },
    opened,
    get workspace() { return need().workspace; },
    get checkout() { return need().checkout; },
    get tab() { return need().tab; },
    get panes() { return need().panes; },
    get inputLogs() { const [one, two] = need().panes; return [paneInputLog(server.root, one), paneInputLog(server.root, two)] as [string, string]; },
    ready: async () => { need(); },
    family,
    runAs: (pane) => {
      const runAs = need().runAs;
      if (!runAs) throw new Error("only a family fixture's agents run hide commands: startFixture({ family: true })");
      return runAs(pane);
    },
  };
}

/**
 * One agent that runs `hide` as a real agent does, from inside its pane: the
 * family fixture's parent alone, in a Git checkout `fixture` on `main`, for a
 * spec whose subject is a command an agent runs (a spawn), not a family.
 * Made, like `startFixture`'s, through the first daemon `startHided` starts.
 */
export type CallerFixture = EngineServer & {
  workspace: string;
  checkout: string;
  tab: string;
  /** The agent's pane. */
  pane: string;
  /** The way to run `hide` as the agent in `pane` (`agent-pane.ts`). */
  runAs: (pane: string) => HideRunner;
};

export async function startCaller(): Promise<CallerFixture> {
  const server = await startEngine();
  const { opened, need } = openedByFirstDaemon(server, (control, daemon) => openCaller(control, server, daemon));
  return {
    ...server,
    daemonEnv: { ...server.daemonEnv, HIDE_E2E_HOOKS: builtHooks() },
    opened,
    get workspace() { return need().workspace; },
    get checkout() { return need().checkout; },
    get tab() { return need().tab; },
    get pane() { return need().pane; },
    runAs: (pane) => need().runAs(pane),
  };
}

/**
 * Makes a fixture's checkout and panes with `open` through the first daemon
 * `startHided` starts on `server`; `need` reads what it made. A setup that
 * gives up keeps the daemon's last snapshot and output as run evidence.
 */
function openedByFirstDaemon<T>(server: EngineServer, open: (control: HidedControl, daemon: OpenedDaemon) => Promise<T>): { opened: (daemon: OpenedDaemon) => Promise<void>; need: () => T } {
  let made: T | null = null;
  const need = () => {
    if (!made) throw new Error("the fixture's checkout and panes are made by its first daemon: read them after startHided");
    return made;
  };
  const opened = async (daemon: OpenedDaemon) => {
    if (made) return;
    // Preserve the setup's last output when no page could open yet. A fixed
    // byte cap keeps an unexpected output flood from growing the run's memory.
    const terminal: string[] = [];
    let terminalBytes = 0;
    let terminalDropped = 0;
    const control = await connectControl(daemon, 15_000, (chunks) => {
      const encoded = JSON.stringify(chunks);
      const bytes = Buffer.byteLength(encoded);
      if (bytes > 256 * 1024) { terminalDropped += 1; return; }
      while (terminal.length && (terminal.length >= 32 || terminalBytes + bytes > 256 * 1024)) {
        terminalBytes -= Buffer.byteLength(terminal.shift()!);
        terminalDropped += 1;
      }
      terminal.push(encoded);
      terminalBytes += bytes;
    });
    try {
      made = await open(control, daemon);
    } catch (error) {
      // Run evidence for a reviewer: what the daemon published when the setup gave up.
      const keep = process.env.HIDE_E2E_SCREENSHOT_DIR;
      if (keep) {
        const rest = await control.until((now) => now, "the last snapshot", 1_000).catch(() => null);
        fs.writeFileSync(path.join(keep, `fixture-setup-${path.basename(server.root)}.json`), JSON.stringify(rest));
        fs.writeFileSync(path.join(keep, `fixture-terminal-${path.basename(server.root)}.json`), JSON.stringify({ dropped: terminalDropped, chunks: terminal.map((encoded) => JSON.parse(encoded)) }));
      }
      throw error;
    } finally {
      control.close();
    }
  };
  return { opened, need };
}

type Opened = { workspace: string; checkout: string; tab: string; panes: [string, string]; runAs?: (pane: string) => HideRunner };
type Caller = { workspace: string; checkout: string; tab: string; pane: string; runAs: (pane: string) => HideRunner };

async function openFixture(control: HidedControl, server: EngineServer, daemon: OpenedDaemon, kind: "shells" | "agents" | "family"): Promise<Opened> {
  const dir = path.join(server.root, "fixture");
  // A family's child opens on the parent's own branch, so the checkout is a Git one (docs/delivery.md, Agent registration and spawning).
  if (kind === "family") gitRepository(dir);
  const first = await openProject(control, dir, "fixture");
  const folder = first.folder;
  if (kind === "shells") {
    const second = await splitPane(control, first.tab, folder);
    // No page shows the fixture's panes before its spec opens one.
    for (const pane of [first.pane, second]) viewPane(control, pane);
    // A published split can precede its holder. The fixture promises two
    // running shells, including to a spec that restarts the daemon at once.
    // Waiting must not select a checkout before the spec opens its page.
    for (const pane of [first.pane, second]) await waitForPaneInput(control, pane);
    // Match the agent fixture and the removed Herdr fixture: a command
    // from the first pane is a command from the operator's current pane.
    await focusPane(control, first.pane);
    return { workspace: first.workspace, checkout: first.checkout, tab: first.tab, panes: [first.pane, second] };
  }
  // A family's agents run `hide` as a real agent does, from inside their panes.
  const runAs = kind === "family" ? installAgentRunner(server as AgentPaneFixture, daemon.stateDir) : undefined;
  for (const agent of FIXTURE_AGENTS) writeFixtureTranscript(claudeProjects(server), agent.session, { task: agent.task });
  const one = await startFirstAgent(control, folder);
  let panes: [string, string];
  if (runAs) {
    // A delegated child opens in a tab of its own: an agent beside its caller is the operator's and has no parent (docs/delivery.md).
    await parentOnItsSession(control, one.pane, runAs);
    const spawned = await runAs(one.pane)(["agent", "spawn", "--parent", "here", "--name", "agent-two", "--intent", "fixture-two", "--kind", "claude", "--repo", fs.realpathSync.native(dir), "--branch", "main", "--", "--resume", FIXTURE_AGENTS[1].session]);
    const answer = JSON.parse(spawned.out.trim().split("\n").at(-1) ?? "null") as { ok?: boolean; value?: { pane?: string } } | null;
    const child = answer?.ok ? answer.value?.pane : undefined;
    if (spawned.status !== 0 || !child) throw new Error(`the fixture's parent could not spawn its child: ${spawned.out}`);
    panes = await control.until((rest) => (agentIn(rest, one.pane) && agentIn(rest, child) ? [one.pane, child] as [string, string] : undefined), "the fixture's child in its own tab", 30_000);
  } else {
    await startAgentInCheckout(control, folder, { resume_session_id: FIXTURE_AGENTS[1].session, target: { ...one.area, tab_id: one.tab, add_pane: true } }, "the second fixture agent");
    panes = await control.until((rest) => {
      const listed = tabPanes(rest, one.tab);
      return listed.length === 2 && listed.every((pane) => agentIn(rest, pane)) ? listed as [string, string] : undefined;
    }, "the second fixture agent beside the first", 30_000);
  }
  viewPane(control, panes[1]);
  // The engine learns a pane's session from the start's `--resume`
  // (docs/ARCHITECTURE.md, Agents on the core's own engine).
  // The shell Add project opened is not the fixture's: its tab goes.
  await closePane(control, first.pane);
  await agentsResting(control, server, panes, Boolean(runAs));
  return { workspace: first.workspace, checkout: first.checkout, tab: one.tab, panes, runAs };
}

/** The family fixture's parent alone (`startCaller`). */
async function openCaller(control: HidedControl, server: EngineServer, daemon: OpenedDaemon): Promise<Caller> {
  const dir = path.join(server.root, "fixture");
  // A spawn names its checkout by repository and branch.
  gitRepository(dir);
  const first = await openProject(control, dir, "fixture");
  const runAs = installAgentRunner(server as AgentPaneFixture, daemon.stateDir);
  writeFixtureTranscript(claudeProjects(server), FIXTURE_AGENTS[0].session, { task: FIXTURE_AGENTS[0].task });
  const one = await startFirstAgent(control, first.folder);
  await parentOnItsSession(control, one.pane, runAs);
  // The shell Add project opened is not the fixture's: its tab goes.
  await closePane(control, first.pane);
  await agentsResting(control, server, [one.pane], true);
  return { workspace: first.workspace, checkout: first.checkout, tab: one.tab, pane: one.pane, runAs };
}

/**
 * Starts the fixture's first agent in a tab of its own in `folder`, as the
 * start panel does, and sizes its pane. It returns once the core has
 * started it, so the start that follows meets a free task slot.
 */
async function startFirstAgent(control: HidedControl, folder: string) {
  const pane = await startAgentInCheckout(control, folder, { resume_session_id: FIXTURE_AGENTS[0].session }, "the first fixture agent");
  const one = await control.until((rest) => {
    const tab = tabOfPane(rest, pane);
    const area = tab ? areaShowing(rest, tab) : undefined;
    return tab && area && agentIn(rest, pane) ? { tab, pane, area } : undefined;
  }, "the first fixture agent in a tab of its own", 30_000);
  viewPane(control, one.pane);
  return one;
}

/**
 * The parent reports its session before it can name itself as one, and the core can name
 * the pane as a caller only once a session pass has laid its delivery observation, which
 * can follow the row that shows the session (with-hide/hide-private#130: `agent spawn` was
 * refused `agent_pane_required` from a pane whose row already read its session). `agent
 * show here` resolves the caller the same way a spawn does and only reads, so the wait
 * polls it and sends no spawn. It also refuses a pane with no ledger record
 * (`participant_unavailable`), and that answer already proves the observation.
 */
async function parentOnItsSession(control: HidedControl, pane: string, runAs: (pane: string) => HideRunner): Promise<void> {
  await control.until((rest) => agentIn(rest, pane)?.session_id === FIXTURE_AGENTS[0].session || undefined, `${pane} on its session`, 30_000);
  const show = runAs(pane);
  let last = "no answer yet";
  try {
    await expect.poll(async () => {
      const answer = await show(["agent", "show", "here"]);
      last = answer.out;
      // A pane the core can name but has no ledger record for is refused
      // `participant_unavailable`, which is past the observation a spawn needs;
      // only `agent_pane_required` says the observation is not laid yet.
      return answer.status === 0 || refusalCode(answer.out) === "participant_unavailable";
    }, { message: `the core to name ${pane} as a caller`, timeout: 30_000 }).toBe(true);
  } catch (error) {
    throw new Error(`${error instanceof Error ? error.message : String(error)}\nthe last answer of \`agent show here\`: ${last}`, { cause: error });
  }
}

/** The `error.code` of a failed `hide` answer (`{"ok":false,"error":{"code":...}}`), or undefined when the output is not that. */
function refusalCode(out: string): string | undefined {
  try {
    const code = (JSON.parse(out) as { error?: { code?: unknown } }).error?.code;
    return typeof code === "string" ? code : undefined;
  } catch { return undefined; }
}

/** Waits until every agent of `panes` reads its terminal and rests, then gives the first the keyboard. */
async function agentsResting(control: HidedControl, server: EngineServer, panes: [string, ...string[]], family: boolean): Promise<void> {
  // Each stand-in opens its input log once it runs and reads its terminal,
  // which the engine can list it before.
  // A family's agents mark their session report instead.
  const reading = (pane: string) => fs.existsSync(family ? reportedMarker(server.root, pane) : paneInputLog(server.root, pane));
  await expect.poll(() => panes.every(reading), { message: "the fixture's agents reading their terminals", timeout: 30_000 }).toBe(true);
  // A spec reads the fixture's rows as they rest, never as they settle.
  await control.until((rest) => panes.every((pane) => startedAgentIn(rest, pane)) || undefined, "the fixture's agents started", 30_000);
  // The first agent holds the keyboard.
  await focusPane(control, panes[0]);
}

/** Makes `dir` a Git repository on `main` with one commit. */
function gitRepository(dir: string): void {
  git(dir, ["init", "-q", "-b", "main"]);
  git(dir, ["commit", "-q", "--allow-empty", "-m", "baseline"]);
}
