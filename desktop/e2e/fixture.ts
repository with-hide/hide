// The desktop app against a private Herdr server and a private hided state
// directory. Nothing here can reach the operator's daemon: the app is
// refused a launch unless HIDE_STATE_DIR, HOME, HERDR_SOCKET_PATH and its
// own userData all sit under this run's temporary directory.

import { _electron as electron, test as base, expect, type ElectronApplication, type Page } from "@playwright/test";
import { spawnSync, type ChildProcess } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { linkFixtureTranscripts, type HerdrFixture } from "../../web/e2e/herdr-fixture";
import { bundledExecutable, linkBundle } from "../../web/e2e/bundled-app";
import { aiSettingsFile } from "../../web/e2e/hided-fixture";
import { ownUntilWorkerExit } from "../../web/e2e/worker-owned";
import { endWindowsProcesses, fixtureExecutable, fixtureHomeEnv, fixtureLoginShellPath, fixtureOpenCommand, fixtureToolPath, inheritedFixtureEnv } from "../../web/e2e/platform-fixture";
import { SHOW_INACTIVE_SWITCH } from "../src/main/launchSwitches";
import { FUSED_ELECTRON_ENV } from "./fused-env";

export const DESKTOP_DIR = path.resolve(__dirname, "..");
export const REPO = path.resolve(DESKTOP_DIR, "..");
export const HIDE_CLI = path.join(REPO, "target", "debug", fixtureExecutable("hide"));
const isolations = new Map<string, { cleanup: () => void; candidates: Set<ChildProcess>; launching: number }>();
const MAX_ISOLATIONS = 16;
const MAX_CANDIDATES_PER_HOME = 16;

export type Isolated = {
  root: string;
  env: Record<string, string>;
  /** Runs a `hide` verb against this run's state directory. */
  hide: (args: string[]) => { status: number | null; stdout: string };
  daemonPid: () => number | null;
  /** A host exception this test causes on purpose, by a part of its message; cleanup fails on any other. */
  allowHostUncaught: (part: string) => void;
  cleanup: () => void;
};

/**
 * The app's `hide` is the worker's app bundle of the debug build
 * (`web/e2e/bundled-app.ts`), so the daemon it starts runs the install kit
 * into the private HOME as a packaged one does. The HOME carries an empty kit
 * record (`seedKitRecord`), as a Mac the kit has run on.
 *
 * A Herdr fixture with its `root` also lends the daemon its `claude`, which
 * is the label provider the fixture's transcripts are answered by, and those
 * transcripts; without one the daemon finds whatever `claude` PATH has, on a
 * HOME where it is not logged in. It also saves `claude` as the agent Hide AI runs on.
 */
export function isolate(herdr: Pick<HerdrFixture, "socket" | "bin"> & Partial<Pick<HerdrFixture, "root" | "afterStop">>, label: string): Isolated {
  if (isolations.size >= MAX_ISOLATIONS) throw new Error(`desktop fixture has ${MAX_ISOLATIONS} unclosed homes; clean an owned fixture before creating another`);
  const root = fs.realpathSync.native(fs.mkdtempSync(path.join(os.tmpdir(), `hide-desktop-${label}-`)));
  const home = path.join(root, "home");
  let env: Record<string, string>;
  try {
    fs.mkdirSync(path.join(home, "projects"), { recursive: true });
    seedKitRecord(home);
    if (herdr.root) {
      linkFixtureTranscripts({ root: herdr.root }, home);
      // Hide AI asks no model until an agent is chosen (PRD settings-cleanup B47), and this app has
      // no signed-in agent for the first-run rule to pick, so the operator here has already chosen
      // the `claude` fixture shim, as a saved choice that is kept (B48).
      const aiFile = aiSettingsFile(home);
      fs.mkdirSync(path.dirname(aiFile), { recursive: true });
      fs.writeFileSync(aiFile, JSON.stringify({ provider: "claude" }));
    }
    env = {
      ...inheritedFixtureEnv(),
      ...fixtureHomeEnv(home),
      HIDE_STATE_DIR: path.join(root, "state"),
      HIDE_DESKTOP_USER_DATA_DIR: path.join(root, "user-data"),
      // The words the host draws follow the system when no language was chosen; the run must not depend on the machine.
      HIDE_DESKTOP_SYSTEM_LANGUAGE: "en-US",
      HIDE_CLI_PATH: bundledExecutable("hide"),
      HIDED_UI_DIR: path.join(REPO, "web", "dist"),
      HERDR_SOCKET_PATH: herdr.socket,
      HERDR_BIN_PATH: herdr.bin,
      PATH: fixtureToolPath(path.join(herdr.root ?? root, "bin")),
      // `open_external` must not launch a GUI application during a test.
      HIDE_OPEN_COMMAND: fixtureOpenCommand(undefined, root),
    };
  } catch (error) {
    fs.rmSync(root, { recursive: true, force: true });
    throw error;
  }
  const cleanupEnv = { ...env };
  const hide = (args: string[]) => {
    const run = spawnSync(HIDE_CLI, args, { env, encoding: "utf8", timeout: 20_000 });
    return { status: run.status, stdout: run.stdout, stderr: run.error?.message ?? run.stderr };
  };
  const daemonPid = () => {
    const answer = JSON.parse(hide(["status", "--json"]).stdout) as { running: boolean; pid?: number };
    return answer.running ? (answer.pid ?? null) : null;
  };
  // Every state folder a daemon of this home may have used: the one the app is given and the two `hide` falls back to.
  const states = () => [cleanupEnv.HIDE_STATE_DIR!, path.join(home, ".hide", "state"), path.join(home, ".local", "state", "hide")];
  const owner = { cleanup: () => {}, candidates: new Set<ChildProcess>(), launching: 0 };
  const allowed: string[] = [];
  let cleaned = false;
  const cleanup = () => {
    if (cleaned) return;
    const live = [...owner.candidates].filter((child) => child.exitCode === null && child.signalCode === null);
    if (live.length || owner.launching) {
      const pids = live.map((child) => child.pid ?? "unavailable").join(", ");
      throw new Error(`fixture cleanup incomplete; preserve ${root}: candidate exit unconfirmed (PIDs: ${pids || "none"}, pending launches: ${owner.launching}); close only the recorded owned candidates, confirm their exit, then call cleanup() again`);
    }
    // Read before the root goes: an exception nothing in the host caught fails
    // the test with its stack, instead of passing unseen (issue 675).
    const uncaught = hostLog(cleanupEnv).filter((line) => line.event === "host.uncaught"
      && !allowed.some((part) => String(line.message).includes(part)));
    const errors: unknown[] = [];
    for (const state of states()) {
      if (!fs.existsSync(state)) continue;
      const stopped = spawnSync(HIDE_CLI, ["stop"], { env: { ...cleanupEnv, HIDE_STATE_DIR: state }, encoding: "utf8", timeout: 20_000 });
      if (stopped.error || stopped.status !== 0) errors.push(new Error(`private hided stop failed for ${state}: ${stopped.error?.message ?? (stopped.stderr || stopped.stdout || stopped.status)}`));
    }
    // A spec cleans up in its own hooks, before the test's outcome is final,
    // so the logs are read here and the test fixture decides whether to keep them.
    if (daemonLogs) {
      try { daemonLogs.push(...daemonLogTails(states(), root, home)); } catch (error) { errors.push(error); }
    }
    if (errors.length) throw new AggregateError(errors, `fixture cleanup incomplete; preserve ${root} and resolve the reported stop/unload failure`);
    // Windows keeps a running executable and a process's folder locked: end what still runs from the root first.
    if (process.platform === "win32") endWindowsProcesses([], root);
    const remove = () => fs.rmSync(root, { recursive: true, force: true });
    // A project the app registered is a workspace in this Herdr, whose panes
    // keep its folder (inside this root) locked on Windows until the server's
    // processes are gone, so the root is removed by the Herdr fixture's stop.
    if (process.platform === "win32" && herdr.afterStop) herdr.afterStop(remove);
    else remove();
    cleaned = true;
    isolations.delete(home);
    owned.disown();
    if (uncaught.length) {
      throw new Error(`the host recorded ${uncaught.length} exception(s) or rejection(s) nothing caught:\n${uncaught.map((line) => `${String(line.kind)}: ${String(line.stack ?? line.message)}`).join("\n")}`);
    }
  };
  const owned = ownUntilWorkerExit(() => {
    try { cleanup(); } catch (error) { process.exitCode = 1; throw error; }
  });
  owner.cleanup = cleanup;
  isolations.set(home, owner);
  return { root, env, hide, daemonPid, allowHostUncaught: (part) => { allowed.push(part); }, cleanup };
}

/**
 * An empty kit record, so this HOME reads as a machine the kit has run on. A
 * machine with no record is held for the first-run agent choice, a dialog over
 * the whole shell that takes the rest of the page out of the accessibility tree,
 * and gets no hook until the operator answers it. A spec about the first run
 * removes `~/.hide/kit/installed.json`; the dialog itself is covered by the web
 * `agent-onboarding` spec, a component test, the core's decision tests and the
 * hide-kit hold tests.
 */
export function seedKitRecord(home: string): void {
  const dir = path.join(home, ".hide", "kit");
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(path.join(dir, "installed.json"), `${JSON.stringify({ format: 1, installed: [] })}\n`);
}

/**
 * Ends a child this fixture started and returns once it has exited, so a
 * folder it held can be removed. A child that does not exit within `ms` is a
 * failure that names it, not a wait that grows.
 */
export async function endChild(child: ChildProcess, ms = 10_000): Promise<void> {
  if (child.exitCode !== null || child.signalCode !== null) return;
  const exited = new Promise<void>((resolve) => child.once("exit", () => resolve()));
  child.kill("SIGTERM");
  let timer: NodeJS.Timeout | undefined;
  const late = new Promise<never>((_, reject) => {
    timer = setTimeout(() => reject(new Error(`child ${child.pid} did not exit within ${ms} ms after SIGTERM`)), ms);
  });
  try {
    await Promise.race([exited, late]);
  } finally {
    clearTimeout(timer);
  }
}

export function assertIsolated(env: Record<string, string>): void {
  // The Herdr fixture keeps its socket directly under /tmp for the path length limit.
  const roots = [fs.realpathSync.native(os.tmpdir()), ...(process.platform === "win32" ? [] : [fs.realpathSync.native("/tmp")])];
  const nativeHomeKeys = process.platform === "win32" ? ["USERPROFILE", "APPDATA", "LOCALAPPDATA"] : [];
  for (const key of ["HOME", "HIDE_STATE_DIR", "HIDE_DESKTOP_USER_DATA_DIR", "HERDR_SOCKET_PATH", ...nativeHomeKeys]) {
    const value = env[key];
    const parent = value ? fs.realpathSync.native(path.dirname(value)) : null;
    if (!parent || !roots.some((root) => {
      const relative = path.relative(root, parent);
      return relative === "" || (!relative.startsWith(`..${path.sep}`) && relative !== ".." && !path.isAbsolute(relative));
    })) throw new Error(`${key} is not isolated under ${roots.join(" or ")}`);
  }
  if (process.platform === "win32" && (env.USERPROFILE !== env.HOME || env.APPDATA !== path.join(env.HOME!, "AppData", "Roaming") || env.LOCALAPPDATA !== path.join(env.HOME!, "AppData", "Local"))) throw new Error("Windows native home and AppData must belong to this fixture's HOME");
}

/**
 * Every launch runs beside the operator's own work (issue 232): the host shows
 * its window behind the operator's without activating the app, so a run never
 * takes the screen, the keyboard or the frontmost app, and the covered window
 * keeps painting, so a capture by window id is current.
 */
const BACKGROUND_SWITCHES = [`--${SHOW_INACTIVE_SWITCH}`, "--disable-backgrounding-occluded-windows"];

/**
 * The tag of a test that needs the key window or native input (a page
 * holding the keyboard, a pinch, a native drag). Such a test brings its
 * window to the front itself, so it takes the keyboard while it runs;
 * `--grep-invert @needs-focus` leaves it out.
 */
/** The core's own node id, as the state folder's `node.json` records it once its daemon has started. */
export function nodeOf(env: Record<string, string>): string {
  return (JSON.parse(fs.readFileSync(path.join(env.HIDE_STATE_DIR!, "node.json"), "utf8")) as { node: string }).node;
}

export const NEEDS_FOCUS = "@needs-focus";

const FOCUS_GUARD = path.join(__dirname, "focus-guard.cjs");

/** The running test's report folder and the apps it launched; null outside a test from this module's `test`. */
let focusReports: { dir: string; apps: { app: ElectronApplication; child: ChildProcess }[] } | null = null;

/**
 * The desktop e2e `test`: after each test not tagged `NEEDS_FOCUS`, any app
 * it launched that became active or gave a window the keyboard fails it, so
 * "an e2e app never comes to the front" is checked on every run, CI included.
 * It sees only this app; other programs a spec could open (a browser, Finder)
 * are stubbed by the specs that reach them. A spec must take `test` from
 * here; `launch` refuses otherwise.
 */
export const test = base.extend<{ focusGuard: void }>({
  focusGuard: [
    // eslint-disable-next-line no-empty-pattern -- Playwright requires the fixtures argument to be destructured.
    async ({}, use, testInfo) => {
      const dir = fs.mkdtempSync(path.join(os.tmpdir(), "hide-e2e-focus-"));
      focusReports = { dir, apps: [] };
      daemonLogs = [];
      const errors: unknown[] = [];
      try {
        await use();
      } catch (error) {
        errors.push(error);
      } finally {
        // Test failures and timeouts skip local finally blocks. The fixture
        // still closes each app; cleanup retains any home whose candidate
        // has not exited, even if close returned or threw.
        for (const { app, child } of focusReports.apps) {
          if (child.exitCode === null && child.signalCode === null) {
            try { await app.close(); } catch (error) { errors.push(error); }
          }
        }
        for (const { cleanup } of [...isolations.values()]) {
          try { cleanup(); } catch (error) { errors.push(error); }
        }
        if (testInfo.status !== testInfo.expectedStatus) {
          try {
            fs.mkdirSync(testInfo.outputPath(), { recursive: true });
            daemonLogs.forEach((text, index) => fs.writeFileSync(testInfo.outputPath(`hided-${index + 1}.jsonl`), text));
          } catch (error) { errors.push(error); }
        }
        daemonLogs = null;
        const reports = fs.readdirSync(dir).flatMap((file) => fs.readFileSync(path.join(dir, file), "utf8").split("\n").filter(Boolean));
        focusReports = null;
        fs.rmSync(dir, { recursive: true, force: true });
        if (!testInfo.tags.includes(NEEDS_FOCUS)) {
          try { expect(reports, `the app came to the front in a test not tagged ${NEEDS_FOCUS}`).toEqual([]); } catch (error) { errors.push(error); }
        }
      }
      if (errors.length) throw new AggregateError(errors, "desktop fixture teardown failed");
    },
    { auto: true },
  ],
});

/** The most of one daemon log a failed attempt keeps: the end of it, where the failure is. */
const KEPT_LOG_BYTES = 256 * 1024;

/**
 * The end of each private daemon log the running test's cleanups have read,
 * written beside its report as `hided-<n>.jsonl` when the test fails: the
 * host log does not show the order of the core's focus, tab and pane records,
 * and without them a CI flake could only be classified by inference (issue
 * 629). Null outside a test from this module's `test`.
 */
let daemonLogs: string[] | null = null;

/**
 * The end of each core log under `states`, at most `KEPT_LOG_BYTES` of whole
 * records, with the run's folders, the repository, the home and the
 * temporary folder written as placeholders; the core log carries no token or
 * credential.
 */
function daemonLogTails(states: string[], root: string, home: string): string[] {
  const placeholders: [string, string][] = [
    [home, "<run>/home"], [root, "<run>"], [REPO, "<repo>"], [os.homedir(), "<home>"],
    [fs.realpathSync.native(os.tmpdir()), "<tmp>"], [os.tmpdir(), "<tmp>"],
  ];
  // Paths reach the log as JSON strings, so a Windows path appears with its backslashes escaped.
  const spellings = placeholders
    .flatMap(([from, to]): [string, string][] => [[JSON.stringify(from).slice(1, -1), to], [from, to]])
    .sort(([a], [b]) => b.length - a.length);
  const tails: string[] = [];
  for (const state of states) {
    const file = path.join(state, "Logs", "core.jsonl");
    if (!fs.existsSync(file)) continue;
    const size = fs.statSync(file).size;
    const start = Math.max(0, size - KEPT_LOG_BYTES);
    const handle = fs.openSync(file, "r");
    const bytes = Buffer.alloc(size - start);
    try { fs.readSync(handle, bytes, 0, bytes.length, start); } finally { fs.closeSync(handle); }
    let text = bytes.toString("utf8");
    // A cut record is dropped whole, so every kept line is JSON.
    if (start > 0) text = text.slice(text.indexOf("\n") + 1);
    for (const [from, to] of spellings) text = text.replaceAll(from, to);
    const note = start > 0 ? `${JSON.stringify({ kind: "e2e.log_tail", kept_bytes: KEPT_LOG_BYTES, total_bytes: size })}\n` : "";
    tails.push(note + text);
  }
  return tails;
}

/** Starts the app with the focus guard preloaded, reporting into the running test's folder. */
async function start(appDir: string, env: Record<string, string>, executablePath?: string): Promise<ElectronApplication> {
  assertIsolated(env);
  if (!focusReports) throw new Error("launch the desktop app from a test imported from desktop/e2e/fixture.ts, so its focus guard runs");
  const owner = isolations.get(env.HOME!);
  if (!owner) throw new Error("launch the desktop app with the HOME of an unclosed isolate() fixture");
  for (const child of owner.candidates) {
    if (child.exitCode !== null || child.signalCode !== null) owner.candidates.delete(child);
  }
  if (owner.candidates.size + owner.launching >= MAX_CANDIDATES_PER_HOME) throw new Error(`desktop fixture has ${MAX_CANDIDATES_PER_HOME} live or launching candidates; close an owned candidate before launching another`);
  // The bundle is linked when an app is about to run its `hide`, so a fixture that launches nothing needs no build.
  if (env.HIDE_CLI_PATH === bundledExecutable("hide")) linkBundle();
  // Written at each launch, since a spec may put its own shims on the PATH after `isolate`.
  fixtureLoginShellPath(env.HOME!, env.PATH!);
  const report = path.join(focusReports.dir, `launch-${focusReports.apps.length + 1}.jsonl`);
  owner.launching++;
  try {
    // A packaged app is given as `executablePath`; otherwise the app folder runs on the fused copy of Electron.
    const fused = executablePath ?? process.env[FUSED_ELECTRON_ENV];
    if (!fused) throw new Error(`${FUSED_ELECTRON_ENV} is not set: the global setup makes the fused Electron an end-to-end launch runs on`);
    const app = await electron.launch({ executablePath: fused, args: ["-r", FOCUS_GUARD, ...(!executablePath ? [appDir] : []), ...BACKGROUND_SWITCHES, `--hide-e2e-focus-report=${report}`], cwd: appDir, env });
    // Taken now: `app.process()` throws once the app is closed. The home
    // retains this handle beyond test teardown if closing the app fails.
    const child = app.process();
    owner.candidates.add(child);
    focusReports.apps.push({ app, child });
    return app;
  } finally {
    owner.launching--;
  }
}

/** `appDir` is the app folder to run, the desktop package unless a test copies it. */
export async function launch(
  env: Record<string, string>,
  { appDir = DESKTOP_DIR, executablePath }: { appDir?: string; executablePath?: string } = {},
): Promise<{ app: ElectronApplication; page: Page }> {
  const app = await start(appDir, env, executablePath);
  const page = await app.firstWindow();
  return { app, page };
}

/**
 * A launch that attaches to a daemon already running and waits for the shell
 * in the window, read through the main process. Such a launch leaves the
 * status page for the daemon's origin within tens of milliseconds, and
 * Playwright can lose that early renderer swap: its page stays on the status
 * page, or no window event arrives at all, while the window shows the shell
 * (checked by reading the window through the main process when Playwright's
 * page did not). The main process sees the window as the operator does.
 */
export async function relaunch(env: Record<string, string>, { appDir = DESKTOP_DIR }: { appDir?: string } = {}): Promise<ElectronApplication> {
  const app = await start(appDir, env);
  await expect
    .poll(
      () =>
        app.evaluate(async ({ BrowserWindow }) => {
          const window = BrowserWindow.getAllWindows()[0];
          if (!window || window.webContents.isLoading()) return false;
          return window.webContents
            .executeJavaScript("document.querySelector('[data-main-screen], [data-workspace-screen]') !== null")
            .catch(() => false) as Promise<boolean>;
        }),
      { timeout: 30_000 },
    )
    .toBe(true);
  return app;
}

/**
 * The page that holds the shell. A browser display is a page of its own to
 * Playwright, and one a relaunch restores can be the first page it reports,
 * so the shell is found by what it draws rather than by order.
 */
export async function shellPage(app: ElectronApplication): Promise<Page> {
  let shell: Page | null = null;
  await expect
    .poll(
      async () => {
        for (const page of app.windows()) {
          const drawn = await page.locator("[data-main-screen], [data-workspace-screen]").count().catch(() => 0);
          if (drawn > 0) {
            shell = page;
            return true;
          }
        }
        return false;
      },
      { timeout: 30_000 },
    )
    .toBe(true);
  return shell!;
}

/**
 * TAKES THE APP'S FOCUS: gives the keyboard to the browser page whose URL
 * or title is `page`, once. A page holds the keyboard only in the key
 * window, and macOS makes the window key after the app activates; Electron
 * then restores the shell's focus, so a page focused while that activation
 * is in flight loses the keyboard a moment later. This activates the app,
 * waits for the window's `focus` event, and only then focuses the shell and
 * the page: a page already holding native focus announces nothing when
 * focused again, so the shell takes it first and the page enters as an
 * operator's click would. Throws when the page does not take it. The caller
 * waits for the host to show the page first: a page focused while its view
 * is hidden does not keep the keyboard once shown.
 */
export async function focusPage(app: ElectronApplication, page: { url: string } | { title: string }): Promise<void> {
  await app.evaluate(async ({ app: electron, BrowserWindow }, page) => {
    const window = BrowserWindow.getAllWindows()[0]!;
    if (!window.isFocused()) {
      await new Promise<void>((resolve, reject) => {
        const deadline = setTimeout(() => reject(new Error("the window never became the key window")), 10_000);
        window.once("focus", () => {
          clearTimeout(deadline);
          resolve();
        });
        electron.focus({ steal: true });
        window.focus();
      });
    }
    const view = window.contentView.children.find((child) => {
      const contents = (child as { webContents?: Electron.WebContents }).webContents;
      return contents !== undefined && ("url" in page ? contents.getURL() === page.url : contents.getTitle() === page.title);
    });
    if (!view) throw new Error(`no page ${JSON.stringify(page)} in the window`);
    const contents = (view as unknown as { webContents: Electron.WebContents }).webContents;
    window.webContents.focus();
    contents.focus();
    if (!contents.isFocused()) throw new Error(`page ${JSON.stringify(page)} did not take the keyboard in the key window`);
  }, page);
}

/**
 * Sizes the first window to what a layout needs, within the primary work
 * area: a CI runner's screen is 1024 points wide and its usable height
 * differs by runner (681 on one, 700 or more on another), and macOS clamps a
 * window to the work area without saying so. It can keep a larger size until
 * the window is next ordered in (a `blur()` or `focus()` of the test's own)
 * and clamp it then, which changes the layout in the middle of a test (issue
 * 511), so a spec sizes its window here (`hide-e2e/window-size-through-fixture`).
 * The width is the layout's and must be granted whole; the height is the work
 * area's when that is shorter. Returns the size macOS granted, so a spec
 * asserts its layout against it.
 */
export async function fitWindow(app: ElectronApplication, wanted: { width: number; height: number }): Promise<{ width: number; height: number }> {
  const { granted, area } = await app.evaluate(({ BrowserWindow, screen }, size) => {
    const work = screen.getPrimaryDisplay().workArea;
    const window = BrowserWindow.getAllWindows()[0]!;
    window.setBounds({ x: work.x, y: work.y, width: size.width, height: Math.min(size.height, work.height) });
    const bounds = window.getBounds();
    return { granted: { width: bounds.width, height: bounds.height }, area: { width: work.width, height: work.height } };
  }, wanted);
  expect(granted.width, `the screen's work area is ${area.width} wide and this layout needs ${wanted.width}`).toBe(wanted.width);
  expect(granted.height, "macOS granted a different height than the work area allows").toBe(Math.min(wanted.height, area.height));
  return granted;
}

/**
 * A launch that returns once the host's own navigation from the status page
 * to the shell has landed, for a spec that reloads or navigates the window
 * itself. A spec navigation sent while the host's is in flight replaces it:
 * the host drops a superseded load as not a failure, so the window stays on
 * the status page with no socket, or the spec's reload is the one aborted.
 */
export async function launchShell(env: Record<string, string>): Promise<{ app: ElectronApplication; page: Page }> {
  const app = await relaunch(env);
  return { app, page: await shellPage(app) };
}

/**
 * The built app copied under this run's directory, so an unpackaged launch
 * has no worktree `target/` beside it and searches for `hide` the way an
 * installed app does.
 */
export function detachedApp(root: string): string {
  const dir = path.join(root, "app");
  fs.mkdirSync(dir);
  fs.copyFileSync(path.join(DESKTOP_DIR, "package.json"), path.join(dir, "package.json"));
  fs.cpSync(path.join(DESKTOP_DIR, "dist"), path.join(dir, "dist"), { recursive: true });
  return dir;
}

/** The host's structured log lines for this run's profile. */
export function hostLog(env: Record<string, string>): { event: string; [key: string]: unknown }[] {
  const file = path.join(env.HIDE_DESKTOP_USER_DATA_DIR!, "logs", "desktop.log");
  if (!fs.existsSync(file)) return [];
  return fs
    .readFileSync(file, "utf8")
    .split("\n")
    .filter(Boolean)
    .map((line) => JSON.parse(line) as { event: string });
}

/**
 * A screenshot under the run directory when one is named; nothing is written
 * otherwise. Running CSS transitions are finished first, so a capture taken
 * right after a click shows the state the click produced rather than a fade.
 */
export async function screenshot(page: Page, name: string): Promise<void> {
  const dir = process.env.HIDE_E2E_SCREENSHOT_DIR;
  if (dir) await page.screenshot({ path: path.join(dir, `${name}.png`), animations: "disabled" });
}
