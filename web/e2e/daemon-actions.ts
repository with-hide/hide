// What a spec does to the daemon's workspaces, tabs and panes on either
// engine (PRD engine-bundle-3-core-owns D-24): each is the one event the web
// shell sends for the operator's action (Add project, New tab, Split, Close,
// a click on a pane or a tab, the start panel's start) and is read back from
// the snapshot the daemon publishes, never from an engine's CLI, so a spec
// says the same thing on Herdr and on the core's own engine.

import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { expect } from "@playwright/test";
import { toPage } from "../../desktop/src/main/wirePath";
import { connectControl, type HidedControl, type Rest } from "./hided-control.mjs";
import type { Daemon } from "./hided-fixture";
import { paneProgramLine, type PaneShell } from "./pane-fixture";

type LayoutNode = { type: "pane"; pane_id: string } | { type: "split"; first: LayoutNode; second: LayoutNode };
type PaneLayout = { tab_id: string; focused_pane_id: string | null; zoomed: boolean; root: LayoutNode };
type NavigatorWorkspace = { id: string; path: string; label?: string; checkouts: { id: string; path?: string; tabs?: { id: string }[]; agent_groups?: { groups: { tabs: string[] }[] } }[] };

/** Runs `use` on a connection of its own to `daemon`, closed whatever happens. */
export async function withControl<T>(daemon: Pick<Daemon, "origin" | "token">, use: (control: HidedControl) => Promise<T>): Promise<T> {
  const control = await connectControl(daemon);
  try {
    return await use(control);
  } finally {
    control.close();
  }
}

/** Types `text` into `pane` as the operator's keys. */
export function sendKeys(control: HidedControl, pane: string, text: string): void {
  control.send("key", { pane_id: pane, bytes_base64: Buffer.from(text, "utf8").toString("base64") });
}

/**
 * Reports a grid for `pane` as the pane view does once it is drawn, for a
 * pane no page shows yet: on Herdr the daemon attaches a pane's terminal,
 * and so delivers keys typed into it, only once a view has sized it
 * (`hide-node` terminal, `waiting_size`); the core's own engine sizes its
 * panes itself. Never for a pane the page shows: its view's own size
 * would be replaced.
 */
export function viewPane(control: HidedControl, pane: string, { cols = 120, rows = 40 }: { cols?: number; rows?: number } = {}): void {
  control.send("terminal_viewport", { pane_id: pane, cols, rows, new_view: true });
  control.send("terminal_resize", { pane_id: pane, cols, rows });
}

/** The panes of a layout, first to last. */
export function layoutPanes(node: LayoutNode): string[] {
  return node.type === "pane" ? [node.pane_id] : [...layoutPanes(node.first), ...layoutPanes(node.second)];
}

/** The layout the snapshot publishes for `tab`, if it has one. */
export function tabLayout(rest: Rest, tab: string): PaneLayout | undefined {
  return ((rest.pane_layouts ?? []) as PaneLayout[]).find((layout) => layout.tab_id === tab);
}

/** The panes of `tab`, first to last; empty while the snapshot has no layout for it. */
export function tabPanes(rest: Rest, tab: string): string[] {
  const layout = tabLayout(rest, tab);
  return layout ? layoutPanes(layout.root) : [];
}

/** The tab whose layout holds `pane`. */
export function tabOfPane(rest: Rest, pane: string): string | undefined {
  return ((rest.pane_layouts ?? []) as PaneLayout[]).find((layout) => layoutPanes(layout.root).includes(pane))?.tab_id;
}

/** The pane the core holds focused, as the snapshot names it. */
export function corePaneFocus(rest: Rest): string | null {
  return ((rest.focused ?? null) as { pane_id?: string } | null)?.pane_id ?? null;
}

/** The tab the core shows, as the snapshot names it. */
export function shownTab(rest: Rest): string | null {
  return ((rest.tab ?? null) as { id?: string } | null)?.id ?? null;
}

/** The workspaces the navigator lists. */
function workspacesOf(rest: Rest): NavigatorWorkspace[] {
  return ((rest.navigator as { workspaces?: NavigatorWorkspace[] } | undefined)?.workspaces ?? []);
}

/**
 * The checkout whose folder is `folder` (the page's spelling) in any project
 * the navigator lists, with its tabs in order: a project's own folder, or a
 * linked worktree, which is listed under its repository's project.
 */
export function checkoutAt(rest: Rest, folder: string): { workspace: string; checkout: string; tabs: string[] } | undefined {
  for (const workspace of workspacesOf(rest)) {
    const checkout = workspace.checkouts.find((listed) => listed.path === folder) ?? (workspace.path === folder ? workspace.checkouts[0] : undefined);
    if (checkout) return { workspace: workspace.id, checkout: checkout.id, tabs: checkout.agent_groups?.groups.flatMap((group) => group.tabs) ?? [] };
  }
  return undefined;
}

/** The tabs of `workspace` the navigator lists, in order. */
export function tabsOf(rest: Rest, workspace: string): string[] | undefined {
  const row = workspacesOf(rest).find((candidate) => candidate.id === workspace);
  return row?.checkouts.flatMap((checkout) => checkout.agent_groups?.groups.flatMap((group) => group.tabs) ?? []);
}

type AreaNode = { area: { id: string; displays: { id: string }[] } } | { split: { first: AreaNode; second: AreaNode } } | Record<string, unknown>;
type AgentAreas = { root: AreaNode; group_id?: string | null };

/** The Agent area of the workspace in front that shows `tab`, as the start panel's target names it. */
export function areaShowing(rest: Rest, tab: string): { group_id: string | null; area_id: string } | undefined {
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

/** The fields of a sidebar agent row (`navigator.agents`) a spec reads. */
export type AgentRow = {
  pane_id: string;
  agent_kind?: string | null;
  status_code?: string | null;
  activity?: string | null;
  blocked?: boolean;
  unread?: boolean;
  session_id?: string | null;
  identity_label?: string | null;
  /** The pane of the agent's responsibility parent, when it is a delegated child. */
  lineage_parent_pane_id?: string | null;
};

/** The agent row the snapshot lists for `pane`, once an agent runs there. */
export function agentIn(rest: Rest, pane: string): AgentRow | undefined {
  const agents = ((rest.navigator as { agents?: AgentRow[] } | undefined)?.agents ?? []);
  return agents.find((row) => row.pane_id === pane && row.agent_kind);
}

/** A project the daemon opened: its checkout, the checkout's first tab and that tab's shell. */
export type OpenedProject = { folder: string; workspace: string; checkout: string; tab: string; pane: string };

/**
 * Adds the folder `dir` as a project, as Add project does once the operator
 * confirmed a folder outside HOME, and waits for its checkout's first tab
 * and shell. `dir` must exist.
 */
export async function openProject(control: HidedControl, dir: string, label: string): Promise<OpenedProject> {
  const folder = toPage(fs.realpathSync.native(dir));
  control.send("create_workspace", { path: folder, label, initialize_git: false, confirmed_outside_home: folder });
  return control.until((rest) => {
    const checkout = checkoutAt(rest, folder);
    const tab = checkout?.tabs[0];
    const pane = tab ? tabPanes(rest, tab)[0] : undefined;
    return checkout && tab && pane ? { folder, workspace: checkout.workspace, checkout: checkout.checkout, tab, pane } : undefined;
  }, `the project ${folder} with its first tab`, 30_000);
}

/** A new tab in `checkout`, as New tab makes it, and its shell. */
export async function newTab(control: HidedControl, at: { workspace: string; checkout: string }, label: string): Promise<{ tab: string; pane: string }> {
  const before = await control.until((rest) => tabsOf(rest, at.workspace), `the tabs of ${at.workspace}`);
  control.send("create_tab", { workspace_id: at.workspace, checkout_id: at.checkout, label, request_id: `e2e-${crypto.randomUUID()}` });
  return control.until((rest) => {
    const tab = tabsOf(rest, at.workspace)?.find((id) => !before.includes(id));
    const pane = tab ? tabPanes(rest, tab)[0] : undefined;
    return tab && pane ? { tab, pane } : undefined;
  }, `a new tab in ${at.workspace}`, 30_000);
}

/**
 * Splits `tab` with a new pane at `cwd` (a folder in the page's spelling),
 * running `command` or the shell, as Split does; returns the new pane.
 */
export async function splitPane(control: HidedControl, tab: string, cwd: string, { direction = "right", command = null }: { direction?: "right" | "down"; command?: string | null } = {}): Promise<string> {
  const before = await control.until((rest) => tabLayout(rest, tab) ? tabPanes(rest, tab) : undefined, `the layout of ${tab}`);
  control.send("create_pane", { tab_id: tab, cwd, command, direction, request_id: `e2e-${crypto.randomUUID()}` });
  return control.until((rest) => tabPanes(rest, tab).find((pane) => !before.includes(pane)), `a new pane in ${tab}`, 30_000);
}

/** Closes `pane` as the operator's confirmed Close does, and waits until no layout holds it. */
export async function closePane(control: HidedControl, pane: string): Promise<void> {
  control.send("close_pane", { pane_id: pane, confirmed: true });
  await control.until((rest) => tabOfPane(rest, pane) === undefined || undefined, `the close of ${pane}`, 30_000);
}

/** Gives `pane` the keyboard as the operator's click does, and waits until the core holds it there. */
export async function focusPane(control: HidedControl, pane: string): Promise<void> {
  control.send("focus_pane", { pane_id: pane, origin: "operator" });
  await control.until((rest) => corePaneFocus(rest) === pane || undefined, `the focus of ${pane}`, 30_000);
}

/** Shows `tab` of `at` as a click on its tab does, and waits until the core shows it. */
export async function focusTab(control: HidedControl, at: { workspace: string; checkout: string }, tab: string): Promise<void> {
  control.send("focus_tab", { workspace_id: at.workspace, checkout_id: at.checkout, tab_id: tab });
  await control.until((rest) => shownTab(rest) === tab || undefined, `${tab} shown`, 30_000);
}

/**
 * Runs `argv` in the shell of `pane`, typed at its prompt as an agent in that
 * tab would run it, and returns its exit status once the shell has written
 * it. Output goes to `<root>/<name>.out` and `<name>.err`; `name` is unique
 * per call.
 */
export async function runInPane(
  fixture: { root: string; shell: PaneShell },
  daemon: Pick<Daemon, "origin" | "token">,
  pane: string,
  name: string,
  program: { env?: Record<string, string>; argv: string[]; stdout?: boolean },
): Promise<{ status: number; stdout: string; stderr: string }> {
  const files = { stdout: path.join(fixture.root, `${name}.out`), stderr: path.join(fixture.root, `${name}.err`), status: path.join(fixture.root, `${name}.status`) };
  const line = paneProgramLine(fixture.shell, { env: program.env, argv: program.argv, stdout: program.stdout ? files.stdout : undefined, stderr: files.stderr, status: files.status });
  await withControl(daemon, async (control) => sendKeys(control, pane, `${line}\r`));
  const read = (file: string): string => (fs.existsSync(file) ? fs.readFileSync(file, "utf8") : "");
  await expect.poll(() => /^\d+/.test(read(files.status)), { message: `${name} to finish in pane ${pane}; stderr ${read(files.stderr)}`, timeout: 20_000 }).toBe(true);
  return { status: Number.parseInt(read(files.status), 10), stdout: read(files.stdout), stderr: read(files.stderr) };
}

/**
 * Opens a terminal in the linked worktree `dir` of a project the daemon
 * already lists, as New tab here on its row does: waits until the core has
 * read the repository and lists the worktree, then makes its first tab.
 */
export async function openWorktree(control: HidedControl, dir: string): Promise<OpenedProject> {
  const folder = toPage(fs.realpathSync.native(dir));
  const listed = await control.until((rest) => checkoutAt(rest, folder), `the worktree ${folder} listed`, 30_000);
  const made = await newTab(control, listed, "worktree");
  return { folder, workspace: listed.workspace, checkout: listed.checkout, ...made };
}

/**
 * `runInPane` for a pane the operator is not in. Typing the line gives that
 * pane the core's keyboard focus on either engine, which an agent running a
 * command never does, so the program waits at a gate while the focus goes
 * back to the pane that held it, and runs only then: it sees the focus as
 * the operator had it.
 */
export async function runInPaneKeepingFocus(
  fixture: { root: string; shell: PaneShell },
  daemon: Pick<Daemon, "origin" | "token">,
  pane: string,
  name: string,
  program: { env?: Record<string, string>; argv: string[]; stdout?: boolean },
): Promise<{ status: number; stdout: string; stderr: string }> {
  const gate = path.join(fixture.root, `${name}.gate`);
  const arrived = `${gate}.arrived`;
  fs.rmSync(arrived, { force: true });
  const wait = [
    'const fs = require("node:fs");',
    'const { spawnSync } = require("node:child_process");',
    "const [program, ...args] = process.argv.slice(1);",
    `fs.writeFileSync(${JSON.stringify(arrived)}, "");`,
    waitForInputGate(gate),
    'process.exit(spawnSync(program, args, { stdio: "inherit" }).status ?? 1);',
  ].join(" ");
  let running: ReturnType<typeof runInPane> | undefined;
  try {
    await withControl(daemon, (control) => restoreFocusAfterInput(control, pane, () => {
      running = runInPane(fixture, daemon, pane, name, { ...program, argv: [process.execPath, "-e", wait, ...program.argv] });
      // Keep an early process failure observed while the focus barrier waits.
      void running.catch(() => undefined);
    }, arrived));
    fs.writeFileSync(gate, "");
    return await running!;
  } finally {
    fs.rmSync(gate, { force: true });
    fs.rmSync(arrived, { force: true });
  }
}

/** Node source for a fixture command's bounded file gate; the caller already imports fs. */
export function waitForInputGate(gate: string, deadlineMs = 30_000): string {
  return `const gateStart = Date.now(); while (!fs.existsSync(${JSON.stringify(gate)})) {
    if (Date.now() - gateStart > ${deadlineMs}) process.exit(9);
    Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 50);
  }`;
}

/** Restore the keyboard after a fixture trigger reaches its pane, before releasing that command's gate. */
export async function restoreFocusAfterInput(control: HidedControl, pane: string, trigger: () => void | Promise<void>, arrived: string): Promise<void> {
  const before = await control.until((rest) => corePaneFocus(rest) ?? undefined, "the focused pane", 20_000);
  await preparePaneInput(control, pane);
  await trigger();
  // Key input's transient focus can be overwritten by an engine snapshot.
  // The process reaching its gate proves the input was actually consumed.
  await expect.poll(() => fs.existsSync(arrived), { message: `the fixture input to reach ${pane}`, timeout: 20_000 }).toBe(true);
  await focusPane(control, before);
}

/** Attach a hidden fixture terminal before sending synthetic input, keeping the operator's selection. */
export async function preparePaneInput(control: HidedControl, pane: string): Promise<void> {
  const state = await control.until((rest) => ({ focus: corePaneFocus(rest), ready: terminalReady(rest, pane) }), "the fixture terminal state");
  if (state.ready) return;
  await showTabOf(control, pane);
  viewPane(control, pane);
  await waitForPaneInput(control, pane);
  if (state.focus && state.focus !== pane) {
    await showTabOf(control, state.focus);
    await focusPane(control, state.focus);
  }
}

/** Wait for an already requested terminal attachment without changing the selected checkout or tab. */
export async function waitForPaneInput(control: HidedControl, pane: string): Promise<void> {
  await control.until((rest) => terminalReady(rest, pane) || undefined, `${pane} controlling its terminal`, 20_000);
}

function terminalReady(rest: Rest, pane: string): boolean {
  const terminals = (rest.terminal as { panes?: { pane_id: string; transport_state: string }[] } | undefined)?.panes;
  return terminals?.some((entry) => entry.pane_id === pane && entry.transport_state === "controlling") ?? false;
}

/**
 * Shows the tab that holds `pane`, as a click on its tab does, wherever the
 * navigator lists it: a tab nobody has shown is not attached on Herdr, so the
 * keys typed into a pane of a spawned child's tab are dropped until it is.
 */
export async function showTabOf(control: HidedControl, pane: string): Promise<void> {
  const where = await control.until((rest) => {
    const tab = tabOfPane(rest, pane);
    if (!tab) return undefined;
    for (const workspace of workspacesOf(rest)) {
      for (const checkout of workspace.checkouts) {
        if (checkout.tabs?.some((listed) => listed.id === tab)) return { at: { workspace: workspace.id, checkout: checkout.id }, tab };
      }
    }
    return undefined;
  }, `the tab holding ${pane} listed`, 30_000);
  await focusTab(control, where.at, where.tab);
}
