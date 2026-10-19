// Main-process-only observation: renderer CDP focus emulation would hold a
// visible capturer even when this candidate's native window is hidden.
const fs = require("node:fs");
const path = require("node:path");
const { setInterval, setTimeout, clearInterval, clearTimeout } = require("node:timers");
const { app, BrowserWindow } = require("electron");
// This case measures native visibility with motion allowed. GitHub's macOS
// image enables Reduce Motion; unlike Playwright pages this raw host inherits it.
// Scope the preference to this owned candidate, leaving OS settings untouched.
app.commandLine.appendSwitch("force-prefers-no-reduced-motion");
const root = process.env.HIDE_DESKTOP_USER_DATA_DIR;
if (!root) throw new Error("native owl probe requires isolated userData");
fs.mkdirSync(root, { recursive: true });
const command = path.join(root, "owl-command.json");
const answer = path.join(root, "owl-answer.json");
function publish(value) {
  fs.writeFileSync(`${answer}.tmp`, JSON.stringify(value));
  fs.renameSync(`${answer}.tmp`, answer);
}
let last = 0;
let busy = false;
const timer = setInterval(async () => {
  if (busy || !fs.existsSync(command)) return;
  busy = true;
  let id;
  try {
    const request = JSON.parse(fs.readFileSync(command, "utf8"));
    id = request.id;
    const window = BrowserWindow.getAllWindows()[0];
    if (id <= last || !window) return;
    last = id;
    if (request.action === "hide") window.hide();
    if (request.action === "show") window.showInactive();
    if (request.action === "minimize") window.minimize();
    if (request.action === "restore") { window.restore(); window.showInactive(); }
    const state = await window.webContents.executeJavaScript(`(() => {
      const owl = document.querySelector('[data-brand-owl]');
      return {hidden: document.hidden, reducedMotion: matchMedia('(prefers-reduced-motion: reduce)').matches,
        motion: document.body.dataset.owlMotion ?? owl?.dataset.owlMotion,
        pose: owl?.dataset.brandOwl, owls: document.querySelectorAll('[data-brand-owl]').length,
        connecting: !!document.getElementById('connecting'), shell: !!document.querySelector('[data-main-screen]')};
    })()`);
    publish({ id, pid: process.pid, visible: window.isVisible(), minimized: window.isMinimized(), ...state });
  } catch (error) { publish({ id, error: String(error) }); }
  finally { busy = false; }
}, 20);
const deadline = setTimeout(() => app.exit(1), 85_000);
app.once("before-quit", () => { clearInterval(timer); clearTimeout(deadline); });
process.once("SIGTERM", () => app.exit(0));
process.stdin.once("end", () => app.exit(0));
process.stdin.once("error", () => app.exit(1));
process.stdin.resume();
