// The engine a web e2e spec runs its panes on, chosen by HIDE_E2E_ENGINE
// (PRD engine-bundle-3-core-owns D-14, D-24): `herdr` (the default) is the
// pinned Herdr in a private, empty server; `own` is the core's own engine,
// a daemon with the development switch on and no Herdr at all. Either way the
// fixture makes its workspace and panes with the events the web shell sends
// (`hided-control.mjs`) and reads them back from the snapshot, so a spec
// written on `startLayoutStack` runs unchanged on both. A spec that looks at
// Herdr itself stays on `herdr-fixture.ts`.

import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { expect, type Page } from "@playwright/test";
import { toPage } from "../../desktop/src/main/wirePath";
import { connectControl, type HidedControl, type Rest } from "./hided-control.mjs";
import { claudeProjects, declareAgentSession, paneShellOf, startHerdr, writeFixtureTranscript, type PaneShell } from "./herdr-fixture";
import { startHided, type Daemon, type DaemonEngine } from "./hided-fixture";
import { fixtureExecutable, fixturePrograms } from "./platform-fixture";
import { afterCleanup, ownUntilWorkerExit } from "./worker-owned";
import { enterWorkspace } from "./wire";

export type E2eEngine = "herdr" | "own";

/** The engine HIDE_E2E_ENGINE names; unset is `herdr`, anything else fails the run. */
export function e2eEngine(): E2eEngine {
  const value = process.env.HIDE_E2E_ENGINE ?? "herdr";
  if (value === "herdr" || value === "own") return value;
  throw new Error(`HIDE_E2E_ENGINE must be herdr or own, not ${JSON.stringify(value)}`);
}

/** What the daemon runs its panes on, and the private root the run's files live under. */
export type EngineServer = DaemonEngine & {
  engine: E2eEngine;
  /** The shell family a pane runs, for commands typed into it. */
  shell: PaneShell;
  /** The HOME the daemon must run in, when the engine's panes read their shell's files from it. */
  home?: string;
  stop: () => void;
};

/** The selected engine, empty: a private Herdr server, or what the own engine's daemon needs. */
export async function startEngine(engine = e2eEngine()): Promise<EngineServer> {
  if (engine === "herdr") return { engine, ...(await startHerdr({ empty: true })) };
  return startOwnEngine();
}

/**
 * The own engine's side of the fixture: no server, since the daemon's node
 * starts each pane's holder itself, only the private root, whose `home` the
 * daemon runs in, and the pane shell's environment, which the daemon hands
 * its panes. The shell is the one the Herdr fixture names, with the same
 * fixed prompt.
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
      // Ubuntu's /etc/zsh/zshrc stops at compinit's question before this HOME's .zshrc (herdr-fixture.ts).
      skip_global_compinit: "1",
      HIDE_E2E_ROOT: root,
    };
    return {
      engine: "own",
      root,
      fixturePath,
      daemonEnv,
      // A pane's shell is a child of the daemon's node, in the daemon's HOME.
      home: path.join(root, "home"),
      shell: paneShellOf(daemonEnv),
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

type LayoutNode = { type: "pane"; pane_id: string } | { type: "split"; first: LayoutNode; second: LayoutNode };
type PaneLayout = { tab_id: string; focused_pane_id: string | null; zoomed: boolean; root: LayoutNode };
type NavigatorWorkspace = { id: string; path: string; checkouts: { id: string; agent_groups?: { groups: { tabs: string[] }[] } }[] };

/** The panes of a layout, first to last. */
export function layoutPanes(node: LayoutNode): string[] {
  return node.type === "pane" ? [node.pane_id] : [...layoutPanes(node.first), ...layoutPanes(node.second)];
}

/** The layout the snapshot publishes for `tab`, if it has one. */
export function tabLayout(rest: Rest, tab: string): PaneLayout | undefined {
  return ((rest.pane_layouts ?? []) as PaneLayout[]).find((layout) => layout.tab_id === tab);
}

/** The pane the core holds focused, as the snapshot names it. */
export function corePaneFocus(rest: Rest): string | null {
  return ((rest.focused ?? null) as { pane_id?: string } | null)?.pane_id ?? null;
}

/**
 * One checkout, the folder `fixture` under the run's root, whose one tab
 * holds `panes` shells side by side (two by default), made through the
 * daemon on either engine. `control` is the fixture's own connection, for a
 * spec that reads what the core published.
 */
export type LayoutStack = {
  engine: E2eEngine;
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
    const folder = toPage(fs.realpathSync.native(path.join(server.root, "fixture")));
    // The run's root is outside the daemon's HOME, so the registration carries
    // the confirmation Add project sends once the operator has given it.
    control.send("create_workspace", { path: folder, label: "fixture", initialize_git: false, confirmed_outside_home: folder });
    const made = await control.until((rest) => {
      const workspaces = ((rest.navigator as { workspaces?: NavigatorWorkspace[] } | undefined)?.workspaces ?? []);
      const workspace = workspaces.find((row) => row.path === folder);
      const checkout = workspace?.checkouts[0];
      const tab = checkout?.agent_groups?.groups.flatMap((group) => group.tabs)[0];
      const layout = tab ? tabLayout(rest, tab) : undefined;
      return workspace && checkout && tab && layout ? { workspace: workspace.id, checkout: checkout.id, tab, first: layoutPanes(layout.root)[0]! } : undefined;
    }, `the checkout ${folder} with its first tab`, 30_000);
    const ids = [made.first];
    if (panes === 2) {
      control.send("create_pane", { tab_id: made.tab, cwd: folder, command: null, direction: "right", request_id: `e2e-${crypto.randomUUID()}` });
      const both = await control.until((rest) => {
        const layout = tabLayout(rest, made.tab);
        return layout && layoutPanes(layout.root).length === 2 ? layoutPanes(layout.root) : undefined;
      }, `the split of ${made.tab}`, 30_000);
      ids.splice(0, 1, ...both);
    }
    return { engine: server.engine, server, daemon, control, folder, workspace: made.workspace, checkout: made.checkout, tab: made.tab, panes: ids, stop };
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
    await expect.poll(() => page.evaluate((id) => window.__hideProbe?.paneText(id) ?? "", pane), { timeout: 20_000, message: `the prompt in ${pane}` }).toContain("fixture %");
  }
}

type AreaNode = { area: { id: string; displays: { id: string }[] } } | { split: { first: AreaNode; second: AreaNode } } | Record<string, unknown>;
type AgentAreas = { root: AreaNode; group_id?: string | null };

/** The Agent area of the workspace in front that shows `tab`, as the start panel's target names it. */
function areaShowing(rest: Rest, tab: string): { group_id: string | null; area_id: string } | undefined {
  const layout = ((rest.workspace_view ?? null) as { agent_layout?: AgentAreas | null } | null)?.agent_layout;
  if (!layout) return undefined;
  const find = (node: AreaNode): string | undefined => {
    if ("area" in node && node.area) {
      const area = node.area as { id: string; displays: { id: string }[] };
      return area.displays.some((display) => display.id === tab) ? area.id : undefined;
    }
    for (const child of Object.values(node)) {
      if (child && typeof child === "object") {
        const found = find(child as AreaNode);
        if (found) return found;
      }
    }
    return undefined;
  };
  const area = find(layout.root);
  return area ? { group_id: layout.group_id ?? null, area_id: area } : undefined;
}

/** The checkout `folder` names in the navigator, with its tabs in order. */
function checkoutAt(rest: Rest, folder: string): { workspace: string; checkout: string; tabs: string[] } | undefined {
  const workspaces = ((rest.navigator as { workspaces?: NavigatorWorkspace[] } | undefined)?.workspaces ?? []);
  const workspace = workspaces.find((row) => row.path === folder);
  const checkout = workspace?.checkouts[0];
  if (!workspace || !checkout) return undefined;
  return { workspace: workspace.id, checkout: checkout.id, tabs: checkout.agent_groups?.groups.flatMap((group) => group.tabs) ?? [] };
}

/** The agent row the snapshot lists for `pane`, once an agent runs there. */
export function agentIn(rest: Rest, pane: string): FixtureAgentRow | undefined {
  const agents = ((rest.navigator as { agents?: FixtureAgentRow[] } | undefined)?.agents ?? []);
  return agents.find((row) => row.pane_id === pane && row.agent_kind);
}

/** The fields of a sidebar agent row (`navigator.agents`) a fixture reads. */
export type FixtureAgentRow = { pane_id: string; agent_kind?: string | null; status_code?: string | null };

/** The file the `claude` shim in `pane` logs every byte its terminal delivered to, under the run's root (`HIDE_E2E_ROOT`). */
export function paneInputLog(root: string, pane: string): string {
  return path.join(root, `input-${pane.replaceAll(":", "-")}.log`);
}

/** The session each of the fixture's two agents runs, whose transcript names its task. */
const FIXTURE_AGENTS = [
  { session: "fixture-one", task: "Agent one" },
  { session: "fixture-two", task: "Agent two" },
] as const;

/**
 * The default fixture of a spec that looks at Hide rather than at an engine,
 * on the engine HIDE_E2E_ENGINE names: one checkout, the folder `fixture`
 * under the run's root, whose one tab holds two panes side by side, each
 * running the `claude` shim on a session whose transcript names it
 * `Agent one` and `Agent two` (`agents: false` leaves two shells).
 * Everything in it is made through the first daemon `startHided` starts on
 * it, with the events the web shell sends: Add project, then the start
 * panel's start twice, the second beside the first. So `workspace`, `tab`,
 * `panes` and `inputLogs` are read only after `startHided` returned.
 */
export type AgentFixture = EngineServer & {
  workspace: string;
  tab: string;
  panes: [string, string];
  /** Per pane, the file its `claude` shim appends every byte its terminal delivered to. */
  inputLogs: [string, string];
  /** Resolves at once: the daemon that made the agents waited for both to run. */
  ready: () => Promise<void>;
};

export async function startFixture({ agents = true }: { agents?: boolean } = {}): Promise<AgentFixture> {
  const server = await startEngine();
  let made: { workspace: string; tab: string; panes: [string, string] } | null = null;
  const need = () => {
    if (!made) throw new Error("the fixture's checkout and panes are made by its first daemon: read them after startHided");
    return made;
  };
  const opened = async (daemon: Daemon) => {
    if (made) return;
    const control = await connectControl(daemon);
    try {
      made = await openFixture(control, server, agents);
    } catch (error) {
      // Run evidence for a reviewer: what the daemon published when the setup gave up.
      const keep = process.env.HIDE_E2E_SCREENSHOT_DIR;
      if (keep) {
        const rest = await control.until((now) => now, "the last snapshot", 1_000).catch(() => null);
        fs.writeFileSync(path.join(keep, `fixture-setup-${path.basename(server.root)}.json`), JSON.stringify(rest));
      }
      throw error;
    } finally {
      control.close();
    }
  };
  return {
    ...server,
    // The stand-in reports a new session through Hide's own hook from inside its pane (`setAgentSession`).
    daemonEnv: { ...server.daemonEnv, HIDE_E2E_HOOKS: path.resolve("..", "target", "debug", fixtureExecutable("hide-agent-hooks")) },
    opened,
    get workspace() { return need().workspace; },
    get tab() { return need().tab; },
    get panes() { return need().panes; },
    get inputLogs() { const [one, two] = need().panes; return [paneInputLog(server.root, one), paneInputLog(server.root, two)] as [string, string]; },
    ready: async () => { need(); },
  };
}

async function openFixture(control: HidedControl, server: EngineServer, agents: boolean): Promise<{ workspace: string; tab: string; panes: [string, string] }> {
  const folder = toPage(fs.realpathSync.native(path.join(server.root, "fixture")));
  control.send("create_workspace", { path: folder, label: "fixture", initialize_git: false, confirmed_outside_home: folder });
  const first = await control.until((rest) => {
    const checkout = checkoutAt(rest, folder);
    const tab = checkout?.tabs[0];
    const layout = tab ? tabLayout(rest, tab) : undefined;
    return checkout && tab && layout ? { ...checkout, tab, pane: layoutPanes(layout.root)[0]! } : undefined;
  }, `the checkout ${folder} with its first tab`, 30_000);
  if (!agents) {
    control.send("create_pane", { tab_id: first.tab, cwd: folder, command: null, direction: "right", request_id: `e2e-${crypto.randomUUID()}` });
    const both = await control.until((rest) => {
      const layout = tabLayout(rest, first.tab);
      return layout && layoutPanes(layout.root).length === 2 ? layoutPanes(layout.root) as [string, string] : undefined;
    }, `the split of ${first.tab}`, 30_000);
    return { workspace: first.workspace, tab: first.tab, panes: both };
  }
  for (const agent of FIXTURE_AGENTS) writeFixtureTranscript(claudeProjects(server), agent.session, { task: agent.task });
  const start = (session: string, target?: Record<string, unknown>) => control.send("agent_start_in_checkout", {
    checkout_path: folder, provider: "claude", resume_session_id: session, request_id: `e2e-${crypto.randomUUID()}`, ...(target ? { target } : {}),
  });
  start(FIXTURE_AGENTS[0].session);
  const one = await control.until((rest) => {
    const tab = checkoutAt(rest, folder)?.tabs.find((id) => id !== first.tab);
    const layout = tab ? tabLayout(rest, tab) : undefined;
    const pane = layout ? layoutPanes(layout.root)[0] : undefined;
    const area = tab ? areaShowing(rest, tab) : undefined;
    return tab && pane && area && agentIn(rest, pane) ? { tab, pane, area } : undefined;
  }, "the first fixture agent in a tab of its own", 30_000);
  start(FIXTURE_AGENTS[1].session, { ...one.area, tab_id: one.tab, add_pane: true });
  const panes = await control.until((rest) => {
    const layout = tabLayout(rest, one.tab);
    const listed = layout ? layoutPanes(layout.root) : [];
    return listed.length === 2 && listed.every((pane) => agentIn(rest, pane)) ? listed as [string, string] : undefined;
  }, "the second fixture agent beside the first", 30_000);
  // Herdr learns a pane's session from its integration's report, the own
  // engine from the start's `--resume` (docs/ARCHITECTURE.md, Agents on the
  // core's own engine).
  if (server.engine === "herdr") {
    const herdr = server as EngineServer & { env: NodeJS.ProcessEnv; bin: string };
    panes.forEach((pane, index) => declareAgentSession(herdr, pane, FIXTURE_AGENTS[index]!.session));
  }
  // The shell Add project opened is not the fixture's: its tab goes.
  control.send("close_pane", { pane_id: first.pane, confirmed: true });
  await control.until((rest) => !checkoutAt(rest, folder)?.tabs.includes(first.tab), `the close of ${first.tab}`, 30_000);
  // Each stand-in opens its input log once it runs and reads its terminal,
  // which an engine can list it before (Herdr lists a start at once).
  await expect.poll(() => panes.every((pane) => fs.existsSync(paneInputLog(server.root, pane))), { message: "both fixture agents reading their terminals", timeout: 30_000 }).toBe(true);
  // The first agent holds the keyboard, as a spec on Herdr's fixture found it.
  control.send("focus_pane", { pane_id: panes[0], origin: "operator" });
  await control.until((rest) => corePaneFocus(rest) === panes[0], `the focus of ${panes[0]}`, 30_000);
  return { workspace: first.workspace, tab: one.tab, panes };
}
