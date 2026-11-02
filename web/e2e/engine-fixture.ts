// The engine a web e2e spec runs its panes on, chosen by HIDE_E2E_ENGINE
// (PRD engine-bundle-3-core-owns D-14, D-24): `herdr` (the default) is the
// pinned Herdr in a private, empty server; `own` is the core's own engine,
// a daemon with the development switch on and no Herdr at all. Either way the
// fixture makes its workspace and panes with the events the web shell sends
// (`hided-control.ts`) and reads them back from the snapshot, so a spec
// written on `startLayoutStack` runs unchanged on both. A spec that looks at
// Herdr itself stays on `herdr-fixture.ts`.

import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { expect, type Page } from "@playwright/test";
import { toPage } from "../../desktop/src/main/wirePath";
import { connectControl, type HidedControl, type Rest } from "./hided-control";
import { paneShellOf, startHerdr, type PaneShell } from "./herdr-fixture";
import { startHided, type Daemon, type DaemonEngine } from "./hided-fixture";
import { fixtureToolPath } from "./platform-fixture";
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
    const fixturePath = fixtureToolPath(path.join(root, "bin"));
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
