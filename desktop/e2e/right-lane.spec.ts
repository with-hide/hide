// Right-lane acceptance in a real, privately owned desktop window (PRD
// right-lane B3, B5, B6, B19, B22, B24; PRD lane-two-columns B17, B19,
// B20, B21): the terminals' resize count and their input through every
// lane change are observed here.
// Native captures are optional run artifacts; the behavioral assertions run in CI.
import { expect, type ElectronApplication, type Page } from "@playwright/test";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { performance } from "node:perf_hooks";
import { openProject, sendKeys, withControl } from "../../web/e2e/daemon-actions";
import { connectControl, type HidedControl } from "../../web/e2e/hided-control.mjs";
import { chord } from "../../web/e2e/chords";
import { countSent, enterWorkspace, keyboardFocus, projectOpener, rest, sendEvent, showExplorer, showProjectRow } from "../../web/e2e/wire";
import { deviceHome, proveDeviceHome, resetDeviceHome, stageBuild, writeSshConfig } from "./device-home";
import { isolate, launchShell, nodeOf, startStack, test } from "./fixture";
import { startSshPaneFixture, type SshPaneFixture } from "./ssh-pane-fixture";

type Probe = {
  arm: (marker: string) => void;
  waitArmed: () => Promise<{ arrival_ms: number; write_ms: number }>;
  paneText: (pane: string) => string;
  attachedPanes: () => string[];
  liveTerminals: () => string[];
};

async function paint(page: Page): Promise<void> {
  await page.evaluate(() => new Promise<void>((done) => requestAnimationFrame(() => requestAnimationFrame(() => done()))));
}

async function nativeCapture(app: ElectronApplication, page: Page, name: string): Promise<void> {
  const dir = process.env.HIDE_E2E_SCREENSHOT_DIR;
  if (!dir || process.platform !== "darwin") return;
  await paint(page);
  const source = await app.evaluate(({ BrowserWindow }) => {
    const windows = BrowserWindow.getAllWindows();
    if (windows.length !== 1) throw new Error("expected exactly one candidate window");
    const window = windows[0]!;
    return { pid: process.pid, window: window.getMediaSourceId(), bounds: window.getBounds(), zoom: window.webContents.getZoomFactor(), focused: window.isFocused() };
  });
  const geometry = await page.evaluate(() => {
    const box = (selector: string) => {
      const element = document.querySelector<HTMLElement>(selector);
      if (!element) return null;
      const rect = element.getBoundingClientRect();
      return { left: rect.left, width: rect.width, visible: element.checkVisibility({ visibilityProperty: true }) };
    };
    return {
      body: document.querySelector<HTMLElement>("[data-lane-row]")!.clientWidth,
      theme: document.documentElement.className,
      mode: document.querySelector<HTMLElement>("[data-workspace-screen]")?.dataset.laneMode ?? null,
      agents: box("[data-agent-area]"),
      lane: box("[data-lane-frame]"),
      explorer: box('[data-lane="explorer"]'),
      rail: Array.from(document.querySelectorAll<HTMLElement>("[data-lane-tool]")).map((control) => ({ name: control.getAttribute("aria-label"), pressed: control.getAttribute("aria-pressed"), badge: control.querySelector("[data-lane-badge]")?.textContent ?? null, focused: control.matches(":focus-visible") })),
      tooltip: document.querySelector<HTMLElement>('[data-slot="tooltip-content"]')?.textContent ?? null,
    };
  });
  const captured = spawnSync("/usr/sbin/screencapture", ["-x", "-o", "-l", source.window.split(":")[1]!, path.join(dir, `${name}.png`)], { encoding: "utf8" });
  expect(captured.status, captured.stderr).toBe(0);
  fs.writeFileSync(path.join(dir, `${name}.json`), JSON.stringify({ method: "exact candidate window screencapture without activation", collectedAt: new Date().toISOString(), source, geometry }, null, 2));
}

async function bodyWidth(app: ElectronApplication, page: Page, width: number): Promise<void> {
  const inset = await page.evaluate(() => innerWidth - document.querySelector<HTMLElement>("[data-lane-row]")!.clientWidth);
  await app.evaluate(({ BrowserWindow }, width) => {
    const window = BrowserWindow.getAllWindows()[0]!;
    // eslint-disable-next-line hide-e2e/window-size-through-fixture -- these bodies are wider than a CI screen and the lane is asserted in whole CSS pixels, which a zoomed page does not give; nothing here orders the window out and in, so macOS keeps the size.
    window.setSize(width, window.getSize()[1]!);
    // macOS can clamp to a small CI screen. Preserve the requested CSS body
    // width with the granted content size, and record zoom in native evidence.
    window.webContents.setZoomFactor(window.getContentBounds().width / width);
  }, width + inset);
  await expect.poll(() => page.locator("[data-lane-row]").evaluate((element) => element.clientWidth)).toBe(width);
}

async function stableCount(count: () => number): Promise<number> {
  let previous = -1;
  await expect.poll(async () => {
    await new Promise((done) => setTimeout(done, 150));
    const current = count();
    const same = current === previous;
    previous = current;
    return same;
  }).toBe(true);
  return previous;
}

/** Types `text` into `pane` as the operator's keys, over the test's own connection to the daemon. */
const sendText = (control: HidedControl, pane: string, text: string): void => sendKeys(control, pane, text);

async function echo(page: Page, control: HidedControl, pane: string, phase: string, afterSend?: (index: number) => Promise<void>) {
  const samples: { marker: string; sent_ms: number; arrival_ms: number; write_ms: number; latency_ms: number }[] = [];
  for (let index = 0; index < 20; index += 1) {
    const marker = `lane_${phase}_${String(index).padStart(3, "0")}`;
    await page.evaluate((marker) => (window as unknown as { __hideProbe: Probe }).__hideProbe.arm(marker), marker);
    sendText(control, pane, `${marker}\n`);
    const sent_ms = performance.timeOrigin + performance.now();
    // Keep marker observation running while the real lane action lands.
    // Both promises are owned here, including an action or echo failure.
    const [sample] = await Promise.all([
      page.evaluate(() => (window as unknown as { __hideProbe: Probe }).__hideProbe.waitArmed()),
      afterSend?.(index),
    ]);
    samples.push({ marker, sent_ms, ...sample, latency_ms: sample.write_ms - sent_ms });
    await new Promise((done) => setTimeout(done, 80));
  }
  const values = samples.map((sample) => sample.latency_ms).sort((a, b) => a - b);
  return { method: "key event sent to xterm write callback, 20 native-window samples, nearest-rank p95; candidate observation, no baseline comparison", samples, p95_ms: values[Math.ceil(values.length * 0.95) - 1] };
}

type Saved = { workspaces: { device_id: string; path: string; open: boolean; tool: string | null; document_width?: number; explorer_width?: number }[] };

function saved(stateDir: string): Saved {
  return JSON.parse(fs.readFileSync(path.join(stateDir, "workspace-views.json"), "utf8")) as Saved;
}

type Lane = Awaited<ReturnType<typeof launchLane>>;

/**
 * One stack on a git checkout with two notes, its window wide enough for
 * the desk beside two panes, entered with the probe; `newViews` counts the
 * terminal views the page asks for after it loads.
 */
async function launchLane(name: string) {
  // The checkout is a repository before the daemon registers it.
  const stack = await startStack(name, { agents: false, prepare: (fixture) => {
    const init = spawnSync("git", ["-C", path.join(fixture.root, "fixture"), "init", "-q", "--initial-branch=main"], { encoding: "utf8" });
    expect(init.status, init.stderr).toBe(0);
  } });
  const { fixture, run, daemon } = stack;
  const keys = await connectControl(daemon);
  const checkout = path.join(fs.realpathSync(fixture.root), "fixture");
  fs.writeFileSync(path.join(checkout, "notes.md"), "# 작업 기록\n\n한글과 English를 나란히 읽습니다.\n");
  const longName = "한글과 English 작업 기록 - 긴 파일 제목과 경로 확인.md";
  fs.writeFileSync(path.join(checkout, longName), "# 긴 제목의 작업 기록\n\n한글과 English가 함께 있는 파일을 읽습니다.\n");
  const { app } = await launchShell(run.env);
  const page = await app.firstWindow();
  const sent = countSent(page);
  const counts = { newViews: 0 };
  page.on("websocket", (socket) => socket.on("framesent", (frame) => {
    const event = JSON.parse(String(frame.payload)) as { kind?: string; payload?: { new_view?: boolean } };
    if (event.kind === "terminal_viewport" && event.payload?.new_view === true) counts.newViews += 1;
  }));
  const url = new URL(page.url());
  url.searchParams.set("probe", "1");
  await page.goto(url.href);
  await expect.poll(() => page.evaluate(() => typeof (window as unknown as { __hideProbe?: Probe }).__hideProbe)).toBe("object");
  await enterWorkspace(page, "fixture");
  const workspace = page.locator("[data-workspace-screen]");
  await expect(workspace).toHaveAttribute("data-lane-mode", "closed");
  const identity = await app.evaluate(({ BrowserWindow }) => ({ pid: process.pid, executable: process.execPath, window: BrowserWindow.getAllWindows()[0]!.getMediaSourceId(), screen: BrowserWindow.getAllWindows()[0]!.getBounds() }));
  await bodyWidth(app, page, 1600);
  return {
    stack, fixture, run, keys, app, page, workspace, checkout, longName, identity, counts,
    resizeCount: () => sent.get("terminal_resize") ?? 0,
    panesShown: await page.locator("[data-pane-view]").count(),
    // ⌘E closes the lane and brings both columns back (PRD lane-two-columns B15).
    toggleLane: () => page.keyboard.press(chord("toggle_lane", "electron")),
    divider: page.locator('[data-lane-divider="lane"]'),
    close: async () => {
      keys.close();
      await app.close().catch(() => undefined);
      stack.stop();
    },
  };
}

/** Stands the desk with notes.md on it: opened from the floating Explorer, which a press on a terminal closes (D-50). */
async function standDesk({ page, workspace, fixture }: Lane): Promise<void> {
  await showExplorer(page);
  await page.locator('[data-explorer-row$="/notes.md"]').click();
  await expect(page.locator("[data-editor-body] .cm-content")).toBeVisible();
  await page.locator(`[data-terminal-host="${fixture.panes[0]}"]`).click({ position: { x: 20, y: 20 } });
  await expect(workspace).toHaveAttribute("data-lane-mode", "docked");
}

/** Native captures of a long file title, the desk's divider and the rail's tooltips, in both themes. */
async function captureThemes({ app, page, divider, checkout, longName }: Lane): Promise<void> {
  await showExplorer(page);
  await page.locator(`[data-explorer-row$="/${longName}"]`).click();
  await expect(page.locator("[data-view-tab-bar]")).toContainText(longName);
  for (const theme of ["dark", "light"] as const) {
    await page.locator("[data-open-settings]").click();
    await page.locator('[data-settings-tab="general"]').click();
    await page.locator(`[data-theme-option="${theme}"]`).click();
    await expect(page.locator("html")).toHaveClass(new RegExp(`\\b${theme}\\b`));
    await page.keyboard.press("Escape");
    await expect(page.locator('[data-settings="true"]')).toHaveCount(0);
    await nativeCapture(app, page, `lane-${theme}-long-title`);
    await rest(page);
    await divider.hover();
    await expect(divider.locator("[data-lane-grip]")).toHaveCSS("opacity", "1");
    await nativeCapture(app, page, `lane-${theme}-divider-hover`);
    await rest(page);
    await keyboardFocus(page, divider);
    await nativeCapture(app, page, `lane-${theme}-divider-focus`);
    await rest(page);
    for (const [tool, name] of [["documents", "Documents"], ["project", "Project"]] as const) {
      await page.locator(`[data-lane-tool="${tool}"]`).hover();
      const tooltip = page.locator('[data-slot="tooltip-content"]');
      await expect(tooltip).toContainText(name);
      await nativeCapture(app, page, `lane-${theme}-${tool}-tooltip`);
      await rest(page);
    }
    await page.locator('[data-view-tab-bar] [data-display]').filter({ hasText: longName }).hover();
    await expect(page.locator('[data-slot="tooltip-content"]')).toContainText(path.join(checkout, longName));
    await nativeCapture(app, page, `lane-${theme}-full-file-path-tooltip`);
    await rest(page);
  }
}

test("a tool and a desk floating with it come and go over the terminals without a resize, and a standing desk resizes each terminal once as it comes, goes or its divider lands (D-50)", async () => {
  const lane = await launchLane("right-lane");
  const { fixture, run, app, page, workspace, checkout, identity, resizeCount, panesShown, toggleLane } = lane;
  try {
    const dir = process.env.HIDE_E2E_SCREENSHOT_DIR;
    if (dir) fs.writeFileSync(path.join(dir, "lane-identity.json"), JSON.stringify({ ...identity, daemonPid: run.daemonPid(), state: run.env.HIDE_STATE_DIR, profile: run.env.HIDE_DESKTOP_USER_DATA_DIR, home: run.env.HOME }, null, 2));
    await nativeCapture(app, page, "lane-closed");

    // The Explorer floats over the agents and the document opened from it
    // floats beside it, fitting no terminal again (B3, B24; lane-two-columns
    // B20; D-50). A press on a terminal closes the Explorer and the desk
    // takes its place, each terminal resizing once; ⌘E closes it and brings
    // it back standing, once each again (D-50).
    const beforeOverlay = await stableCount(resizeCount);
    await showExplorer(page);
    await page.locator('[data-explorer-row$="/notes.md"]').click();
    await expect(page.locator("[data-editor-body] .cm-content")).toBeVisible();
    await expect(workspace).toHaveAttribute("data-lane-mode", "overlay");
    await expect(workspace).toHaveAttribute("data-lane-view", "documents");
    await expect(workspace).toHaveAttribute("data-lane-shows", "explorer");
    await nativeCapture(app, page, "lane-overlay");
    expect(await stableCount(resizeCount)).toBe(beforeOverlay);
    await page.locator(`[data-terminal-host="${fixture.panes[0]}"]`).click({ position: { x: 20, y: 20 } });
    await expect(workspace).toHaveAttribute("data-lane-mode", "docked");
    await expect(workspace).toHaveAttribute("data-lane-shows", "none");
    for (const mode of ["closed", "docked"]) {
      await toggleLane();
      await expect(workspace).toHaveAttribute("data-lane-mode", mode);
    }
    await expect(workspace).toHaveAttribute("data-lane-shows", "none");
    expect(await stableCount(resizeCount)).toBe(beforeOverlay + 3 * panesShown);


    // The divider at the standing desk's edge moves a guide, resizing
    // nothing during the drag, and lands once on release, each terminal
    // resizing once; the width is the Workspace's and survives a reload (B6,
    // B22; PRD lane-desk-pair-view D-50).
    const divider = page.locator('[data-lane-divider="lane"]');
    await expect(divider).toHaveAttribute("aria-valuenow", "520");
    const box = (await divider.boundingBox())!;
    const before = await stableCount(resizeCount);
    await page.mouse.move(box.x + box.width / 2, box.y + 100);
    await page.mouse.down();
    await page.mouse.move(box.x + box.width / 2 + 32, box.y + 100, { steps: 6 });
    await expect(page.locator("[data-lane-guide]")).toBeVisible();
    expect(resizeCount()).toBe(before);
    await nativeCapture(app, page, "lane-divider-guide");
    await page.mouse.up();
    await expect(page.locator("[data-lane-guide]")).toHaveCount(0);
    await expect(divider).toHaveAttribute("aria-valuenow", "488");
    await expect.poll(resizeCount).toBeGreaterThan(before);
    const resizeAfterRelease = await stableCount(resizeCount) - before;
    expect(resizeAfterRelease).toBe(panesShown);
    await expect.poll(() => saved(run.env.HIDE_STATE_DIR!).workspaces.find((entry) => entry.path === checkout)).toMatchObject({ open: true, tool: null, document_width: 488 });
    await page.reload();
    await expect(workspace).toHaveAttribute("data-lane-mode", "docked");
    await expect(divider).toHaveAttribute("aria-valuenow", "488");
    await nativeCapture(app, page, "lane-restored-width");
    if (dir) fs.writeFileSync(path.join(dir, "lane-divider.json"), JSON.stringify({ identity, resizeDuringDrag: 0, resizeAfterRelease, panesShown, restoredLaneWidth: 488 }, null, 2));
  } finally {
    await lane.close();
  }
});

test("input continues through every change of a standing desk and a held drag of its divider, with the same terminals attached (D-50)", async () => {
  const lane = await launchLane("right-lane-input");
  const { fixture, keys, app, page, workspace, identity, counts, resizeCount, panesShown, toggleLane, divider } = lane;
  try {
    const dir = process.env.HIDE_E2E_SCREENSHOT_DIR;
    await standDesk(lane);
    // Idle and driven echo, then input that continues through every lane
    // change and a held drag, with the same terminals attached (B24).
    const pane = fixture.panes[0];
    sendText(keys, pane, "stty -echo -icanon; echo LANE_IDLE_READY; cat\n");
    await expect.poll(() => page.evaluate((id) => (window as unknown as { __hideProbe: Probe }).__hideProbe.paneText(id), pane)).toContain("LANE_IDLE_READY");
    const idle = await echo(page, keys, pane, "idle");
    sendText(keys, pane, "\u0003");
    const driver = path.join(fixture.root, "lane-driven.sh");
    fs.writeFileSync(driver, '#!/bin/bash\nstty -echo -icanon\n(while true; do printf "lane driven output\\n"; sleep 0.008; done) &\ndriver=$!\ntrap \'kill "$driver" 2>/dev/null; wait "$driver" 2>/dev/null\' EXIT HUP TERM\necho LANE_DRIVEN_READY\ncat\n');
    sendText(keys, pane, `bash '${driver}'\n`);
    await expect.poll(() => page.evaluate((id) => (window as unknown as { __hideProbe: Probe }).__hideProbe.paneText(id), pane)).toContain("LANE_DRIVEN_READY");
    const driven = await echo(page, keys, pane, "driven");
    await nativeCapture(app, page, "lane-driven-terminal");
    await stableCount(resizeCount);
    const session = () => page.evaluate(() => {
      const probe = (window as unknown as { __hideProbe: Probe }).__hideProbe;
      return { attached: probe.attachedPanes().sort(), live: probe.liveTerminals().sort() };
    });
    const originalSession = await session();
    const originalElements = await page.evaluateHandle(() => Array.from(document.querySelectorAll("[data-agent-area] .xterm")));
    const beforeNewViews = counts.newViews;
    const transitionActions: { action: string; start_ms: number; end_ms: number; resizes: number }[] = [];
    let drivenChanges;
    let drivenDrag;
    const retained = async () => {
      expect(await session()).toEqual(originalSession);
      expect(counts.newViews).toBe(beforeNewViews);
      expect(await page.evaluate((original) => {
        const current = Array.from(document.querySelectorAll("[data-agent-area] .xterm"));
        return current.length === original.length && current.every((element, index) => element === original[index]);
      }, originalElements)).toBe(true);
      expect(await page.evaluate((id) => (window as unknown as { __hideProbe: Probe }).__hideProbe.paneText(id), pane)).toContain("lane driven output");
    };
    try {
      const changes = [[toggleLane, "closed", panesShown], [toggleLane, "docked", panesShown], [toggleLane, "closed", panesShown], [toggleLane, "docked", panesShown]] as const;
      drivenChanges = await echo(page, keys, pane, "change", async (index) => {
        if (index % 5 !== 0) return;
        const [control, mode, expected] = changes[index / 5]!;
        const before = await stableCount(resizeCount);
        const start_ms = Date.now();
        await control();
        await expect(workspace).toHaveAttribute("data-lane-mode", mode);
        if (expected > 0) await expect.poll(resizeCount).toBeGreaterThan(before);
        const resizes = await stableCount(resizeCount) - before;
        expect(resizes).toBe(expected);
        await retained();
        transitionActions.push({ action: mode, start_ms, end_ms: Date.now(), resizes });
        await nativeCapture(app!, page, `lane-driven-${index / 5}-${mode}`);
      });
      const dragBox = (await divider.boundingBox())!;
      const beforeDrag = await stableCount(resizeCount);
      const start_ms = Date.now();
      drivenDrag = await echo(page, keys, pane, "drag", async (index) => {
        if (index === 0) {
          await page.mouse.move(dragBox.x + dragBox.width / 2, dragBox.y + 100);
          await page.mouse.down();
          await page.mouse.move(dragBox.x + dragBox.width / 2 - 32, dragBox.y + 100, { steps: 6 });
          await expect(page.locator("[data-lane-guide]")).toBeVisible();
          expect(resizeCount()).toBe(beforeDrag);
          await nativeCapture(app!, page, "lane-driven-drag-held");
        } else if (index === 3) {
          await page.mouse.move(dragBox.x + dragBox.width / 2 - 64, dragBox.y + 100, { steps: 6 });
          expect(resizeCount()).toBe(beforeDrag);
        } else if (index === 5) {
          await page.mouse.up();
          await expect(page.locator("[data-lane-guide]")).toHaveCount(0);
          await expect.poll(resizeCount).toBeGreaterThan(beforeDrag);
          const resizes = await stableCount(resizeCount) - beforeDrag;
          expect(resizes).toBe(panesShown);
          transitionActions.push({ action: "drag:release", start_ms, end_ms: Date.now(), resizes });
          await nativeCapture(app!, page, "lane-driven-drag-released");
        }
        await retained();
      });
      expect(drivenChanges.samples).toHaveLength(20);
      expect(drivenDrag.samples).toHaveLength(20);
    } finally {
      await page.mouse.up();
      await originalElements.dispose();
    }
    if (dir) fs.writeFileSync(path.join(dir, "lane-echo.json"), JSON.stringify({ identity, transitions: { actions: transitionActions, originalSession, newViewRequests: counts.newViews - beforeNewViews, terminalElementsRetained: true, drivenChanges, drivenDrag }, load: spawnSync("uptime", { encoding: "utf8" }).stdout.trim(), workload: "two visible shell panes; idle cat and driven line every 8ms in measured pane; 20 markers per idle, driven, driven-change and driven-drag phase; native captures at action states, no continuous blank-frame recording, baseline or speed claim", idle, driven }, null, 2));
    if (dir) await captureThemes(lane);
  } finally {
    await lane.close();
  }
});

test("lane dividers retain every rapid arrow step before a snapshot arrives", async () => {
  const stack = await startStack("lane-key-burst", { agents: false });
  const { fixture, run } = stack;
  let app: ElectronApplication | null = null;
  try {
    fs.writeFileSync(path.join(fixture.root, "fixture", "notes.md"), "# notes\n");
    ({ app } = await launchShell(run.env));
    const page = await app.firstWindow();
    await enterWorkspace(page, "fixture");
    await bodyWidth(app, page, 1600);
    await showExplorer(page);
    await page.locator('[data-explorer-row$="/notes.md"]').click();
    await expect(page.locator("[data-workspace-screen]")).toHaveAttribute("data-lane-mode", "overlay");
    const lane = page.locator('[data-lane-divider="lane"]');
    const column = page.locator('[data-lane-divider="column"]');
    const explorer = () => page.locator('[data-lane-column="tool"]').evaluate((element) => element.clientWidth);
    // Both dividers name the document's width: the lane's at its edge, the column's as its position.
    await expect(lane).toHaveAttribute("aria-valuenow", "520");
    await expect(column).toHaveAttribute("aria-valuenow", "520");
    await expect.poll(explorer).toBe(240);
    const burst = async (divider: typeof lane, key: "ArrowLeft" | "ArrowRight") => {
      await divider.focus();
      // Deliver both keys in one browser turn, so the regression does not
      // depend on the machine winning a race against the next snapshot.
      await divider.evaluate((element, key) => {
        for (let count = 0; count < 2; count += 1) {
          element.dispatchEvent(new KeyboardEvent("keydown", { key, code: key, bubbles: true, cancelable: true }));
        }
      }, key);
    };
    // The lane's divider widens the document; the column divider trades the
    // document's width with the Explorer's and leaves the lane and Agent
    // Views as they are (B6, B10; lane-two-columns B17).
    await burst(lane, "ArrowLeft");
    await expect(lane).toHaveAttribute("aria-valuenow", "584");
    const agents = await page.locator("[data-agent-area]").evaluate((element) => element.clientWidth);
    await burst(column, "ArrowLeft");
    await expect(column).toHaveAttribute("aria-valuenow", "520");
    await expect(lane).toHaveAttribute("aria-valuenow", "520");
    await expect.poll(explorer).toBe(304);
    await expect(page.locator("[data-agent-area]")).toHaveJSProperty("clientWidth", agents);
    await burst(column, "ArrowRight");
    await expect(column).toHaveAttribute("aria-valuenow", "584");
    await expect.poll(explorer).toBe(240);
    await burst(lane, "ArrowRight");
    await expect(lane).toHaveAttribute("aria-valuenow", "520");
    await page.reload();
    await expect(lane).toHaveAttribute("aria-valuenow", "520");
    await expect(column).toHaveAttribute("aria-valuenow", "520");
    await expect.poll(explorer).toBe(240);
    await nativeCapture(app, page, "lane-rapid-keys-restored");
  } finally {
    await app?.close().catch(() => undefined);
    stack.stop();
  }
});

function prepareLaneRepositories(folders: string[], env: NodeJS.ProcessEnv): void {
  for (const folder of folders) {
    fs.mkdirSync(folder);
    fs.writeFileSync(path.join(folder, "한글-notes.md"), "# Remote lane notes\n");
    const initialized = spawnSync("git", ["-C", folder, "init", "-q"], { env, encoding: "utf8", timeout: 10_000 });
    expect(initialized.status, initialized.stderr).toBe(0);
  }
}

test("an SSH Workspace's lane keeps its chords, narrow fallback and saved widths apart from the same local path", async () => {
  test.skip(!process.env.HIDE_E2E_SSH_PORT, "requires the proved private SSH HOME in device-home.ts");
  test.setTimeout(300_000);
  const local = await startSshPaneFixture();
  let remote: SshPaneFixture | null = null;
  let run: ReturnType<typeof isolate> | null = null;
  let app: ElectronApplication | null = null;
  let remoteHome: string | null = null;
  const errors: unknown[] = [];
  try {
    remote = await startSshPaneFixture();
    run = isolate(local, "remote-lane");
    run.env.HIDE_HOST_HELPER_ROOT = path.join(run.root, "device-helper");
    run.env.HIDE_HOST_CLI_DIR = path.join(run.root, "device-bin");
    writeSshConfig(run.env.HOME!, { "isolated-lane": "lane-ssh" });
    remoteHome = deviceHome();
    proveDeviceHome(run.env, "isolated-lane", remoteHome);
    resetDeviceHome(remoteHome);
    // The host starts and owns the staged daemon through its existing CLI
    // boundary; no second resident-process launcher belongs to this spec.
    run.env.HIDE_CLI_PATH = path.join(stageBuild(run.root), "hide");
    const shared = path.join(fs.realpathSync(local.root), "shared-lane");
    const other = path.join(fs.realpathSync(remote.root), "remote-only");
    prepareLaneRepositories([shared, other], run.env);
    ({ app } = await launchShell(run.env));
    await local.attach(run);
    await local.open(shared, "shared-lane");
    const page = await app.firstWindow();
    const dir = process.env.HIDE_E2E_SCREENSHOT_DIR;
    if (dir) {
      const candidate = await app.evaluate(({ BrowserWindow }) => ({ pid: process.pid, window: BrowserWindow.getAllWindows()[0]!.getMediaSourceId(), executable: process.execPath }));
      fs.writeFileSync(path.join(dir, "remote-lane-identity.json"), JSON.stringify({ ...candidate, daemonPid: run.daemonPid(), home: run.env.HOME, deviceHome: remoteHome, state: run.env.HIDE_STATE_DIR, profile: run.env.HIDE_DESKTOP_USER_DATA_DIR, transport: "real loopback SSH to a separate private device" }, null, 2));
    }
    await enterWorkspace(page, "fixture");
    const state = JSON.parse(fs.readFileSync(path.join(run.env.HIDE_STATE_DIR!, "hided.json"), "utf8")) as { token: string };
    await sendEvent(page, { origin: new URL(page.url()).origin, token: state.token }, "register_device", {
      label: "SSH lane", ssh_alias: "isolated-lane", host_consent: true,
    });
    const rail = (device: string) => page.locator(`[data-rail-tile="${device}"]`);
    await expect(rail("lane-ssh")).toHaveAttribute("data-rail-connected", "true", { timeout: 120_000 });
    await remote.attach(run, "lane-ssh", remoteHome);
    await remote.open(shared, "shared-lane");
    await remote.open(other, "remote-only");
    await rail("lane-ssh").click();
    const row = (label: string) => page.locator("[data-project]", { hasText: label }).locator("[data-checkout]").first();
    await row("shared-lane").click();
    await expect(row("shared-lane")).toHaveAttribute("aria-current", "true");
    const workspace = page.locator("[data-workspace-screen]");
    await bodyWidth(app, page, 1600);
    await expect(workspace).toHaveAttribute("data-lane-mode", "closed");
    // ⌘⇧E opens the Explorer; ⌘E closes and opens the lane with both columns (lane-two-columns B4, B15).
    await page.keyboard.press(chord("toggle_lane_explorer", "electron"));
    await expect(page.locator('[data-explorer-row$="/한글-notes.md"]')).toBeVisible({ timeout: 60_000 });
    await page.locator('[data-explorer-row$="/한글-notes.md"]').click();
    await expect(page.locator("[data-editor-body] .cm-content")).toContainText("Remote lane notes");
    await page.keyboard.press(chord("toggle_lane", "electron"));
    await expect(workspace).toHaveAttribute("data-lane-mode", "closed");
    await page.keyboard.press(chord("toggle_lane", "electron"));
    await expect(workspace).toHaveAttribute("data-lane-mode", "overlay");
    await expect(workspace).toHaveAttribute("data-lane-view", "documents");
    await expect(page.locator('[data-lane="explorer"]')).toBeVisible();
    // A released drag and an arrow key on the column divider save this
    // device's widths (B6, B10, B22; lane-two-columns B17).
    const divider = page.locator('[data-lane-divider="lane"]');
    const box = (await divider.boundingBox())!;
    await page.mouse.move(box.x + box.width / 2, box.y + 100);
    await page.mouse.down();
    await page.mouse.move(box.x + box.width / 2 + 96, box.y + 100);
    await expect(page.locator("[data-lane-guide]")).toBeVisible();
    await page.mouse.up();
    await expect(divider).toHaveAttribute("aria-valuenow", "424");
    await page.locator('[data-lane-divider="column"]').press("ArrowLeft");
    await expect(divider).toHaveAttribute("aria-valuenow", "392");
    await expect.poll(() => saved(run!.env.HIDE_STATE_DIR!).workspaces.find((entry) => entry.device_id === "lane-ssh" && entry.path === shared)).toMatchObject({ open: true, tool: "explorer", document_width: 392, explorer_width: 272 });
    await nativeCapture(app, page, "remote-lane-saved");
    await bodyWidth(app, page, 800);
    await expect(workspace).toHaveAttribute("data-lane-mode", "overlay");
    await nativeCapture(app, page, "remote-lane-narrow");
    await bodyWidth(app, page, 1600);
    await expect(workspace).toHaveAttribute("data-lane-mode", "overlay");
    // The same path on this machine is another Workspace with its own lane.
    await rail(nodeOf(run.env)).click();
    await row("shared-lane").click();
    await expect(row("shared-lane")).toHaveAttribute("aria-current", "true");
    await expect(workspace).toHaveAttribute("data-lane-mode", "closed");
    await showExplorer(page);
    await page.locator('[data-explorer-row$="/한글-notes.md"]').click();
    await expect(page.locator("[data-editor-body] .cm-content")).toContainText("Remote lane notes");
    await expect(workspace).toHaveAttribute("data-lane-mode", "overlay");
    expect(saved(run.env.HIDE_STATE_DIR!).workspaces.find((entry) => entry.device_id !== "lane-ssh" && entry.path === shared)?.document_width).not.toBe(392);
    await nativeCapture(app, page, "remote-lane-same-local-path");
    await rail("lane-ssh").click();
    await row("remote-only").click();
    await expect(workspace).toHaveAttribute("data-lane-mode", "closed");
    await row("shared-lane").click();
    await expect(workspace).toHaveAttribute("data-lane-mode", "overlay");
    await expect(divider).toHaveAttribute("aria-valuenow", "392");
    await page.reload();
    await expect(divider).toHaveAttribute("aria-valuenow", "392");
    await expect(page.locator('[data-lane-divider="column"]')).toHaveAttribute("aria-valuenow", "392");
    await nativeCapture(app, page, "remote-lane-restored");
    if (dir) fs.writeFileSync(path.join(dir, "remote-lane-saved.json"), JSON.stringify(saved(run.env.HIDE_STATE_DIR!), null, 2));
  } catch (error) {
    errors.push(error);
  } finally {
    try { await app?.close(); } catch (error) { errors.push(error); }
    for (const cleanup of [
      () => run?.cleanup(),
      () => remote?.stop(),
      () => local.stop(),
    ]) {
      try { cleanup(); } catch (error) { errors.push(error); }
    }
  }
  if (errors.length) throw new AggregateError(errors, "private SSH lane fixture failed; inspect all test and cleanup errors before removing its declared homes");
});

// Changing Workspaces while the pointer is captured cancels the old guide;
// releasing it must not land a width in the newly front Workspace.
test("a Workspace switch cancels its lane drag without changing another Workspace", async () => {
  // Both folders are repositories with a document before the daemon registers them.
  const stack = await startStack("lane-drag-switch", { agents: false, prepare: (fixture) => {
    fs.mkdirSync(path.join(fixture.root, "beta"));
    for (const folder of [path.join(fixture.root, "fixture"), path.join(fixture.root, "beta")]) {
      fs.writeFileSync(path.join(folder, "notes.md"), "# Workspace notes\n");
      const init = spawnSync("git", ["-C", folder, "init", "-q"], { encoding: "utf8" });
      expect(init.status, init.stderr).toBe(0);
    }
  } });
  const { fixture, run, daemon } = stack;
  let app: ElectronApplication | null = null;
  try {
    await withControl(daemon, (control) => openProject(control, path.join(fixture.root, "beta"), "beta"));
    ({ app } = await launchShell(run.env));
    const page = await app.firstWindow();
    const sent = countSent(page);
    await page.reload();
    await enterWorkspace(page, "fixture");
    await bodyWidth(app, page, 1600);
    // A project that is not open is opened from its own row; its main checkout's row is drawn once it holds the focus.
    const opener = (label: string) => projectOpener(page.locator("[data-project]", { hasText: label }));
    const row = (label: string) => page.locator("[data-project]", { hasText: label }).locator("[data-checkout]").first();
    const openNotes = async () => {
      await showExplorer(page);
      await page.locator('[data-explorer-row$="/notes.md"]').click();
      await expect(page.locator("[data-editor-body] .cm-content")).toBeVisible();
      await expect(page.locator("[data-workspace-screen]")).toHaveAttribute("data-lane-mode", "overlay");
    };
    await (await showProjectRow(page, opener("beta"))).click({ position: { x: 12, y: 8 } });
    await expect(row("beta")).toHaveAttribute("aria-current", "true");
    await openNotes();
    for (const theme of ["dark", "light"] as const) {
      await (await showProjectRow(page, opener("fixture"))).click({ position: { x: 12, y: 8 } });
      await expect(row("fixture")).toHaveAttribute("aria-current", "true");
      await openNotes();
      await page.locator("[data-open-settings]").click();
      await page.locator('[data-settings-tab="general"]').click();
      await page.locator(`[data-theme-option="${theme}"]`).click();
      await expect(page.locator("html")).toHaveClass(new RegExp(`\\b${theme}\\b`));
      await page.keyboard.press("Escape");
      await expect(page.locator('[data-settings="true"]')).toHaveCount(0);
      const divider = page.locator('[data-lane-divider="lane"]');
      const box = (await divider.boundingBox())!;
      await page.mouse.move(box.x + box.width / 2, box.y + 100);
      await page.mouse.down();
      await page.mouse.move(box.x + box.width / 2 - 64, box.y + 100);
      await expect(page.locator("[data-lane-guide]")).toBeVisible();
      await expect(page.locator("html")).toHaveAttribute("data-view-drag", "col-resize");
      const before = sent.get("workspace_view") ?? 0;
      // Invoke the candidate's Workspace control while capture is held, as a
      // keyboard Workspace switch can do without releasing the divider.
      await opener("beta").evaluate((element: HTMLElement) => element.click());
      await expect(row("beta")).toHaveAttribute("aria-current", "true");
      await expect(page.locator("[data-lane-guide]")).toHaveCount(0);
      await expect(page.locator("html")).not.toHaveAttribute("data-view-drag");
      await page.mouse.up();
      await paint(page);
      expect(sent.get("workspace_view") ?? 0).toBe(before);
      await expect(divider).toHaveAttribute("aria-valuenow", "520");
      expect(saved(run.env.HIDE_STATE_DIR!).workspaces.every((entry) => entry.document_width === undefined)).toBe(true);
      await nativeCapture(app, page, `lane-${theme}-drag-switch-cancelled`);
    }
  } finally {
    await app?.close().catch(() => undefined);
    stack.stop();
  }
});
