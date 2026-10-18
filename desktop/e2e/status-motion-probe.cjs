// Main-process-only observation: renderer CDP focus emulation would hold a
// visible capturer even when this candidate's native window is hidden.
const fs = require("node:fs");
const path = require("node:path");
const { setInterval, setTimeout, clearInterval, clearTimeout } = require("node:timers");
const { app, BrowserWindow } = require("electron");
const root = process.env.HIDE_DESKTOP_USER_DATA_DIR;
if (!root) throw new Error("native owl probe requires isolated userData");
fs.mkdirSync(root, { recursive: true });
const command = path.join(root, "owl-command.json");
const answer = path.join(root, "owl-answer.json");
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
      return {hidden: document.hidden, motion: document.body.dataset.owlMotion ?? owl?.dataset.owlMotion,
        pose: owl?.dataset.brandOwl, owls: document.querySelectorAll('[data-brand-owl]').length,
        connecting: !!document.getElementById('connecting'), shell: !!document.querySelector('[data-main-screen]')};
    })()`);
    fs.writeFileSync(answer, JSON.stringify({ id, pid: process.pid, visible: window.isVisible(), minimized: window.isMinimized(), ...state }));
  } catch (error) { fs.writeFileSync(answer, JSON.stringify({ id, error: String(error) })); }
  finally { busy = false; }
}, 20);
const deadline = setTimeout(() => app.exit(1), 85_000);
app.once("before-quit", () => { clearInterval(timer); clearTimeout(deadline); });
process.once("SIGTERM", () => app.exit(0));
