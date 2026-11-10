// Preloaded into every e2e app's main process (`-r`, before the app's own
// code), so it hears the app come forward even while it starts. Each time the
// app becomes active or a window takes the keyboard, one JSON line is appended
// to the file `--hide-e2e-focus-report` names; `fixture.ts` reads it after the
// test and fails a test not tagged @needs-focus (issue 232). A file rather than
// stderr, so a report from before Playwright attaches or just before quit is kept.

const fs = require("node:fs");
const { app } = require("electron");
const { disabledTestFeatures } = require("../../web/e2e/browser-launch.cjs");

// Merge the launcher's effective features before app readiness. An explicit
// executable (the fused candidate) runs without Playwright's default loader.
if (process.platform === "win32") app.commandLine.appendSwitch("disable-features", disabledTestFeatures(app.commandLine.getSwitchValue("disable-features")));

const file = app.commandLine.getSwitchValue("hide-e2e-focus-report");
if (!file) throw new Error("focus-guard.cjs needs --hide-e2e-focus-report=<file>");
const report = (event) => {
  try {
    fs.appendFileSync(file, `${JSON.stringify({ event, pid: process.pid, at: new Date().toISOString() })}\n`);
  } catch (error) {
    // An app that outlived its test finds the folder gone; a throw here would
    // raise Electron's error dialog, which itself comes to the front.
    process.stderr.write(`focus-guard: report not written: ${String(error)}\n`);
  }
};
app.on("did-become-active", () => report("did-become-active"));
app.on("browser-window-focus", () => report("browser-window-focus"));
