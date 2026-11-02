#!/usr/bin/env node
// How long a restart after a reboot takes to give the operator their work back
// (PRD engine-bundle-3-core-owns B44, perf-gate row 30). run.sh killed every
// process the run started (the daemon, the Herdr server or the pane holders,
// every pane's shell), noted MEASURE_RESTORE_T0 (epoch ms) and started the
// daemon again, and the Herdr server first on Herdr. From t0, on this
// machine's wall clock:
// - `tabs_ms`: the daemon's snapshot lays out every tab of the fixture again.
// - `front_ms`: the page, opened as the daemon answers, shows the measured
//   checkout's tab with every pane of it drawn (its terminal has text).
// - `shells_ms`: every pane's shell has read its rc file again: the private
//   `.zshrc` (run.sh) touches one file per shell under MEASURE_READY_DIR, and
//   this counts the files written after t0.
// Prints JSON; each time is null when it did not come within the bound.
import fs from "node:fs";
import path from "node:path";
import { connectPage } from "./cdp.mjs";
import { connectStateDir } from "../../web/e2e/hided-control.mjs";

const stateDir = process.env.MEASURE_HIDE_STATE;
const cdpPort = process.env.MEASURE_CDP_PORT;
const readyDir = process.env.MEASURE_READY_DIR;
const t0 = Number(process.env.MEASURE_RESTORE_T0);
const tabs = Number(process.env.MEASURE_RESTORE_TABS);
const panes = Number(process.env.MEASURE_RESTORE_PANES);
const boundMs = Number(process.env.MEASURE_RESTORE_SECONDS ?? 180) * 1000;
const openFixture = fs.readFileSync(new URL("./open-fixture.js", import.meta.url), "utf8");
if (!stateDir || !cdpPort || !readyDir || !Number.isFinite(t0) || !(tabs > 0) || !(panes > 0)) {
  throw new Error("MEASURE_HIDE_STATE, MEASURE_CDP_PORT, MEASURE_READY_DIR, MEASURE_RESTORE_T0, MEASURE_RESTORE_TABS and MEASURE_RESTORE_PANES are required");
}
const deadline = t0 + boundMs;
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const since = () => Date.now() - t0;

/** The daemon's state file once it answers /health, as the restarted daemon wrote it. */
async function daemonUp() {
  while (Date.now() < deadline) {
    try {
      const state = JSON.parse(fs.readFileSync(path.join(stateDir, "hided.json"), "utf8"));
      if ((await fetch(`http://127.0.0.1:${state.port}/health`, { signal: AbortSignal.timeout(1000) })).ok) return state;
    } catch {
      /* not up yet */
    }
    await sleep(20);
  }
  return null;
}

async function shellsBack() {
  while (Date.now() < deadline) {
    const fresh = fs.readdirSync(readyDir).filter((name) => fs.statSync(path.join(readyDir, name)).mtimeMs >= t0).length;
    if (fresh >= panes) return since();
    await sleep(20);
  }
  return null;
}

async function tabsBack() {
  let control = null;
  try {
    control = await connectStateDir(stateDir, Math.max(1, deadline - Date.now()));
    await control.until((rest) => (rest?.pane_layouts ?? []).length >= tabs, `${tabs} tabs`);
    return since();
  } catch {
    return null;
  } finally {
    control?.close();
  }
}

// What the page showed at its last look, for a front that did not come back.
let frontLast = null;

async function frontBack(page, state) {
  await page.send("Page.navigate", { url: `http://127.0.0.1:${state.port}/?probe=1#token=${state.token}` });
  while (Date.now() < deadline) {
    try {
      if (await page.evaluate(openFixture)) {
        frontLast = await page.evaluate(`(() => {
          const views = [...document.querySelectorAll('[data-canvas] [data-pane-view]')].map((el) => el.dataset.paneView);
          return { url: location.href.split('#')[0], tab: document.querySelector('[data-canvas]')?.dataset.canvas ?? null, panes: views.map((id) => ({ id, chars: (window.__hideProbe?.paneText(id) ?? '').trim().length })) };
        })()`);
        if (frontLast.panes.length > 0 && frontLast.panes.every((pane) => pane.chars > 0)) return since();
      } else {
        frontLast = { url: await page.evaluate("location.href.split('#')[0]"), opened: false };
      }
    } catch (error) {
      frontLast = { error: String(error) };
    }
    await sleep(20);
  }
  return null;
}

const page = await connectPage(cdpPort, false);
try {
  const shells = shellsBack();
  const state = await daemonUp();
  const healthMs = state ? since() : null;
  const [tabsMs, frontMs, shellsMs] = state ? await Promise.all([tabsBack(), frontBack(page, state), shells]) : [null, null, await shells];
  console.log(JSON.stringify({
    method: "wall clock from t0, the moment after every killed process was confirmed gone; tabs = the snapshot lays out every fixture tab; front = the measured checkout's tab drawn in the page opened once the daemon answers /health; shells = every pane's shell touched its file under MEASURE_READY_DIR after t0",
    tabs_expected: tabs, panes_expected: panes,
    health_ms: healthMs, tabs_ms: tabsMs, front_ms: frontMs, shells_ms: shellsMs,
    ...(frontMs === null ? { front_last: frontLast } : {}),
  }, null, 2));
} finally {
  page.close();
}
