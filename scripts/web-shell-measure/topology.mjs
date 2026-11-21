#!/usr/bin/env node
// Pane topology latency on the product web shell (PRD instant-pane-topology
// D-15, B23): split, zoom, unzoom, pane close and new tab, each started by the
// operator's own chord, plus a tab switch by click (B24), and a window resize
// (PRD engine-bundle-3-core-owns B43, perf-gate row 27): the page's width set
// with CDP `Emulation.setDeviceMetricsOverride`, 1280 to 1100 and back.
//
// Keys reach the page as CDP `Input.dispatchKeyEvent`, which Chrome delivers
// as an ordinary renderer keydown to the focused terminal, so they take the
// web shell's own keydown path (`keyboard.ts`, the browser chords of
// `shortcuts.ts`); a click is an element click in the page.
// Every time is the page's own clock:
// - `key_ms`: the first keydown of the chord (a capture listener), the click,
//   or, for a resize, the moment before the width is set.
// - `screen_ms`: the first animation frame whose DOM shows the change (the
//   pane count, the zoom flag or the shown tab moved).
// - `frame_ms`: the first animation frame after that in which the terminal
//   the change is about shows it: a new pane's terminal has text, or the
//   resized pane's xterm has a new grid; for a resize, every shown pane's.
// Prints JSON samples; summarize.py topology reads them.
import { spawnSync } from "node:child_process";
import { connectPage } from "./cdp.mjs";

const cdpPort = process.env.MEASURE_CDP_PORT;
const paneId = process.env.MEASURE_PANE_ID;
const switchTab = process.env.MEASURE_SWITCH_TAB;
const rounds = Number(process.env.MEASURE_TOPOLOGY_ROUNDS ?? 20);
const zoomOnly = process.env.MEASURE_TOPOLOGY_ZOOM_ONLY === "1";
if (!cdpPort || !paneId || (!zoomOnly && !switchTab)) throw new Error("MEASURE_CDP_PORT, MEASURE_PANE_ID and MEASURE_SWITCH_TAB are required");

// The browser column of the shortcut table on a Mac (`shortcuts.ts`).
const META = 4;
const ALT = 1;
const SHIFT = 8;
const CHORDS = {
  split: { code: "KeyD", key: "d", keyCode: 68, modifiers: META },
  zoom: { code: "Enter", key: "Enter", keyCode: 13, modifiers: META | SHIFT },
  close: { code: "KeyW", key: "w", keyCode: 87, modifiers: ALT | SHIFT },
  new_tab: { code: "KeyT", key: "t", keyCode: 84, modifiers: ALT },
};
const SETTLE_MS = 600;
const TIMEOUT_MS = 15000;

const page = await connectPage(cdpPort);
const load = spawnSync("uptime", { encoding: "utf8" }).stdout.trim();

await page.evaluate(`(() => {
  if (window.__topology) return true;
  const state = { keyAt: null };
  window.addEventListener("keydown", (event) => { if (state.keyAt === null) state.keyAt = performance.now(); }, true);
  const canvas = () => document.querySelector("[data-canvas]");
  const panes = () => [...(canvas()?.querySelectorAll("[data-pane-view]") ?? [])].map((el) => el.dataset.paneView);
  const focused = () => document.querySelector('[data-pane-view][data-focused="true"]')?.dataset.paneView ?? null;
  const grid = (id) => { const g = window.__hideProbe.paneGrid(id); return g ? g.cols + "x" + g.rows : null; };
  const look = () => ({ tab: canvas()?.dataset.canvas ?? null, zoomed: canvas()?.dataset.zoomed === "true", panes: panes(), focused: focused(), width: Math.round(canvas()?.getBoundingClientRect().width ?? 0) });
  window.__topology = {
    look,
    grid,
    arm() { state.keyAt = null; },
    markClick() { state.keyAt = performance.now(); },
    // Resolves when the screen shows the change, then when the terminal does.
    watch(kind, before, timeoutMs) {
      const beforeGrids = Object.fromEntries(before.panes.map((id) => [id, grid(id)]));
      return new Promise((resolve) => {
        const started = performance.now();
        let screen = null;
        let subject = null;
        const step = (now) => {
          const seen = look();
          if (screen === null) {
            let moved = false;
            if (kind === "split") moved = seen.panes.length === before.panes.length + 1;
            else if (kind === "close") moved = seen.panes.length === before.panes.length - 1;
            else if (kind === "zoom" || kind === "unzoom") moved = seen.zoomed !== before.zoomed;
            else if (kind === "new_tab" || kind === "switch") moved = seen.tab !== null && seen.tab !== before.tab && seen.panes.length > 0;
            else if (kind === "resize") moved = seen.width !== before.width;
            if (moved) {
              screen = now;
              if (kind === "split") subject = seen.panes.find((id) => !before.panes.includes(id));
              else if (kind === "new_tab" || kind === "switch") subject = seen.panes[0];
              else if (kind === "close") subject = seen.panes[0];
              else subject = before.focused ?? seen.panes[0];
            }
          }
          if (screen !== null) {
            let shown = false;
            if (kind === "split" || kind === "new_tab") shown = window.__hideProbe.paneText(subject).trim().length > 0;
            else if (kind === "switch") shown = grid(subject) !== null;
            else if (kind === "resize") shown = seen.panes.every((id) => grid(id) !== null && grid(id) !== beforeGrids[id]);
            else shown = grid(subject) !== null && grid(subject) !== beforeGrids[subject];
            if (shown) return resolve({ key_ms: state.keyAt, screen_ms: screen, frame_ms: now, subject, after: look(), origin: performance.timeOrigin });
          }
          if (now - started > timeoutMs) return resolve({ key_ms: state.keyAt, screen_ms: screen, frame_ms: null, subject, after: look(), timeout: true, origin: performance.timeOrigin });
          requestAnimationFrame(step);
        };
        requestAnimationFrame(step);
      });
    },
  };
  return true;
})()`);

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const look = () => page.evaluate("window.__topology.look()");

async function press(chord) {
  const base = { code: chord.code, key: chord.key, windowsVirtualKeyCode: chord.keyCode, nativeVirtualKeyCode: chord.keyCode, modifiers: chord.modifiers };
  await page.send("Input.dispatchKeyEvent", { type: "rawKeyDown", ...base });
  await page.send("Input.dispatchKeyEvent", { type: "keyUp", ...base });
}

async function focusPane(id) {
  await page.evaluate(`(() => {
    const view = document.querySelector('[data-pane-view="${id}"]');
    view?.querySelector('.xterm-helper-textarea')?.focus();
    view?.dispatchEvent(new MouseEvent('mousedown', { bubbles: true }));
    return Boolean(view);
  })()`);
  const deadline = Date.now() + 3000;
  while (Date.now() < deadline) {
    if ((await look()).focused === id) return;
    await sleep(50);
  }
  throw new Error(`pane ${id} did not take focus`);
}

/** The page's width, as a window of that width lays it out; the window itself stays 1280 wide. */
async function resizeTo(width) {
  const height = await page.evaluate("window.innerHeight");
  await page.evaluate("window.__topology.markClick(); true");
  await page.send("Emulation.setDeviceMetricsOverride", { width, height, deviceScaleFactor: 0, mobile: false });
}

const samples = [];
// Operations the rounds made without timing them (the new tab's close), for
// the saves per operation the save watch divides by.
let unsampled = 0;
async function measure(kind, start) {
  const before = await look();
  await page.evaluate("window.__topology.arm()");
  const watching = page.evaluate(`window.__topology.watch(${JSON.stringify(kind)}, ${JSON.stringify(before)}, ${TIMEOUT_MS})`, true);
  await start();
  const result = await watching;
  const sample = {
    kind,
    screen_ms: result.screen_ms === null || result.key_ms === null ? null : result.screen_ms - result.key_ms,
    frame_ms: result.frame_ms === null || result.key_ms === null ? null : result.frame_ms - result.key_ms,
    timeout: Boolean(result.timeout),
    subject: result.subject,
    // Wall-clock times, so a sample lines up with the core's `pane_op.timing`
    // record of the same operation (its `requested_at_unix_ms` and stages):
    // key to request, request to snapshot read, and that read to the screen.
    key_unix_ms: result.key_ms === null ? null : result.origin + result.key_ms,
    screen_unix_ms: result.screen_ms === null ? null : result.origin + result.screen_ms,
  };
  samples.push(sample);
  await sleep(SETTLE_MS);
  return result;
}

// Shows the measured tab again, by clicking it when the area shows another.
async function backToMeasuredTab() {
  const deadline = Date.now() + 5000;
  let seen = null;
  while (Date.now() < deadline) {
    seen = await look();
    if (seen.tab !== null && seen.panes.includes(paneId)) return seen;
    if (measuredTab && seen.tab !== null && seen.tab !== measuredTab) {
      await page.evaluate(`document.querySelector('[data-agent-tab-bar] [data-tab="${measuredTab}"]')?.click(); true`);
    }
    await sleep(100);
  }
  throw new Error(`the measured tab did not come back: ${JSON.stringify(seen)}`);
}

let measuredTab = null;
measuredTab = (await backToMeasuredTab()).tab;
for (let round = 0; round < rounds; round += 1) {
  await focusPane(paneId);
  if (zoomOnly) {
    await measure("zoom", () => press(CHORDS.zoom));
    await measure("unzoom", () => press(CHORDS.zoom));
    continue;
  }
  const split = await measure("split", () => press(CHORDS.split));
  // A timed-out sample is kept as one; the round goes on only if the pane
  // came after all, and the next round starts from the measured tab.
  const created = split.subject ?? (await look()).panes.find((id) => id !== paneId);
  if (!created) {
    await backToMeasuredTab();
    continue;
  }
  await focusPane(created);
  await measure("zoom", () => press(CHORDS.zoom));
  await measure("unzoom", () => press(CHORDS.zoom));
  await focusPane(created);
  await measure("close", () => press(CHORDS.close));
  await backToMeasuredTab();

  await focusPane(paneId);
  const opened = await measure("new_tab", () => press(CHORDS.new_tab));
  if (opened.timeout && !opened.subject) {
    await backToMeasuredTab();
    continue;
  }
  // Close the new tab's only pane so every round starts at the same scale.
  await focusPane(opened.subject);
  await press(CHORDS.close);
  unsampled += 1;
  const gone = Date.now() + 5000;
  while ((await look()).tab === opened.after.tab) {
    if (Date.now() > gone) throw new Error(`new tab ${opened.after.tab} did not close`);
    await sleep(100);
  }
  await backToMeasuredTab();
  await sleep(SETTLE_MS);

  await measure("switch", async () => {
    await page.evaluate(`(() => { window.__topology.markClick(); document.querySelector('[data-agent-tab-bar] [data-tab="${switchTab}"]').click(); return true; })()`);
  });
  await measure("switch", async () => {
    await page.evaluate(`(() => { window.__topology.markClick(); document.querySelector('[data-agent-tab-bar] [data-tab="${measuredTab}"]').click(); return true; })()`);
  });
  await backToMeasuredTab();
  await measure("resize", () => resizeTo(1100));
  await measure("resize", () => resizeTo(1280));
}
await page.send("Emulation.clearDeviceMetricsOverride", {});
page.close();
console.log(JSON.stringify({
  method: "page clock; key_ms = first keydown of the chord (CDP Input.dispatchKeyEvent, delivered as a renderer keydown to the focused terminal and handled by the web shell's keydown path), the tab click, or the moment before a resize's CDP Emulation.setDeviceMetricsOverride; screen = first animation frame whose DOM shows the change; frame = first animation frame after it in which the subject terminal shows it (text in a new pane, a new grid in a resized one); nearest-rank percentiles computed by summarize.py topology",
  load, pane_id: paneId, rounds, zoom_only: zoomOnly, operations: samples.length + unsampled, samples,
}, null, 2));
