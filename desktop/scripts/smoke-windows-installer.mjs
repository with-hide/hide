// Proves the Windows installer on a CI runner (PRD auto-update B2, B10, B13,
// D-32), since no Windows machine is at hand to try one on:
//
//   1. `setup.exe /S` installs for this user, without elevation, into
//      %LOCALAPPDATA%\Programs\hide, with a Start menu entry and an entry in
//      Settings > Apps that names the version;
//   2. with a Herdr server and a daemon running from the build's runtime copy
//      (as the desktop host starts them), installing again leaves both running:
//      the installer stops every program under the install folder, and the copy
//      lives under the private home instead;
//   3. the uninstaller removes the app, its Start menu entry and its Apps
//      entry, stops the daemon and leaves the Herdr server running.
//
// Usage: node desktop/scripts/smoke-windows-installer.mjs <setup.exe>
//
// The installer itself writes to this account's real install folder, Start
// menu and registry, which is what is being tested; a CI runner's account is
// discarded with it. Every hide and Herdr process runs with a private home,
// state folder and Herdr socket.

import { execFileSync, spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { setTimeout as delay } from "node:timers/promises";

if (process.platform !== "win32") throw new Error("smoke-windows-installer checks the Windows installer only");
const setup = process.argv[2] ? path.resolve(process.argv[2]) : null;
if (!setup || process.argv.length !== 3) throw new Error("usage: node desktop/scripts/smoke-windows-installer.mjs <setup.exe>");
const version = /^hide-v(.+)-windows-x64-setup\.exe$/.exec(path.basename(setup))?.[1];
if (!version) throw new Error(`${path.basename(setup)} is not a hide Windows installer name`);

const localAppData = process.env.LOCALAPPDATA;
const appData = process.env.APPDATA;
if (!localAppData || !appData) throw new Error("LOCALAPPDATA and APPDATA name this account's folders and must be set");
const installDir = path.join(localAppData, "Programs", "hide");
const startMenuEntry = path.join(appData, "Microsoft", "Windows", "Start Menu", "Programs", "hide.lnk");

function check(name, body) {
  try {
    const detail = body();
    console.log(`ok ${name}${detail ? `: ${detail}` : ""}`);
  } catch (error) {
    throw new Error(`${name} failed: ${error instanceof Error ? error.message : String(error)}`, { cause: error });
  }
}

async function until(name, condition, timeoutMs = 60_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (condition()) return;
    await delay(250);
  }
  throw new Error(`${name} did not happen within ${timeoutMs / 1000} s`);
}

const isAlive = (pid) => {
  try { process.kill(pid, 0); return true; }
  catch (error) { return error.code === "EPERM"; }
};

const output = (file, args, env) => execFileSync(file, args, { env, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"], timeout: 120_000 }).trim();

/** This user's uninstall entries named hide, as Settings > Apps lists them. */
function appsEntries() {
  const script = "Get-ItemProperty 'HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\*' -ErrorAction SilentlyContinue | Where-Object DisplayName -eq 'hide' | ForEach-Object { $_.DisplayVersion }";
  return output("pwsh", ["-NoProfile", "-Command", script], process.env).split(/\r?\n/).filter(Boolean);
}

/** The records hided wrote to its Logs file in `stateDir`, oldest first. */
function daemonRecords(stateDir) {
  const file = path.join(stateDir, "Logs", "core.jsonl");
  if (!fs.existsSync(file)) return [];
  return fs.readFileSync(file, "utf8").split(/\r?\n/).filter(Boolean).map((line) => {
    try { return JSON.parse(line); } catch { return { line }; }
  });
}

/** How long each step of the daemon's start took, as it logged them. */
const startSteps = (stateDir) => daemonRecords(stateDir)
  .filter((record) => record.kind === "start.step")
  .map((record) => `${record.stage} ${record.took_ms} ms`)
  .join(", ");

const scratch = fs.realpathSync.native(fs.mkdtempSync(path.join(os.tmpdir(), "hide-installer-")));
const home = path.join(scratch, "home");
fs.mkdirSync(home);
fs.mkdirSync(path.join(home, ".hide"));
fs.writeFileSync(path.join(home, ".hide", "product-analytics-disabled"), "");
const isolated = Object.fromEntries(Object.entries(process.env).filter(([key]) => !/^(HERDR_|HIDE_)/i.test(key)));
Object.assign(isolated, {
  HOME: home,
  USERPROFILE: home,
  XDG_CONFIG_HOME: path.join(home, ".config"),
  HERDR_SOCKET_PATH: path.join(scratch, "herdr.sock"),
  HIDE_STATE_DIR: path.join(home, ".hide", "state"),
  HIDE_TAILSCALE_BIN: path.join(home, "no-tailscale"),
});

let runtime = null;
let herdrPid = null;
try {
  check("silent per-user install", () => {
    execFileSync(setup, ["/S"], { stdio: "inherit", timeout: 300_000 });
    for (const file of [path.join(installDir, "hide.exe"), path.join(installDir, "resources", "hide.exe"), path.join(installDir, "resources", "herdr.exe")]) {
      if (!fs.statSync(file, { throwIfNoEntry: false })?.isFile()) throw new Error(`${file} is missing`);
    }
    if (!fs.existsSync(startMenuEntry)) throw new Error(`no Start menu entry at ${startMenuEntry}`);
    const entries = appsEntries();
    if (entries.length !== 1 || entries[0] !== version) throw new Error(`Settings > Apps lists hide as [${entries.join(", ")}], expected ${version}`);
    return installDir;
  });

  check("the runtime is copied out of the install folder", () => {
    const answer = JSON.parse(output(path.join(installDir, "resources", "hide.exe"), ["runtime", "prepare"], isolated));
    if (answer.ok !== true || answer.dir.toLowerCase().startsWith(installDir.toLowerCase())) throw new Error(JSON.stringify(answer));
    runtime = answer.dir;
    return runtime;
  });

  // As the desktop host starts them: Herdr's server from the copy, then the copy's `hide connect`.
  const herdr = path.join(runtime, "herdr.exe");
  const server = spawn(herdr, ["server"], { env: isolated, detached: true, stdio: "ignore", windowsHide: true });
  herdrPid = server.pid;
  server.unref();
  await until("the Herdr server answering", () => {
    try { return JSON.parse(output(herdr, ["status", "server", "--json"], isolated)).running === true; } catch { return false; }
  });
  const connectBegan = Date.now();
  const connected = JSON.parse(output(path.join(runtime, "hide.exe"), ["connect"], { ...isolated, HERDR_BIN_PATH: herdr }));
  const connectMs = Date.now() - connectBegan;
  check("the daemon starts from the copy", () => {
    if (connected.ok !== true) throw new Error(JSON.stringify(connected));
    return `daemon ${connected.pid}, Herdr server ${herdrPid}; connect ${connectMs} ms; start steps: ${startSteps(isolated.HIDE_STATE_DIR)}`;
  });

  check("installing again keeps the Herdr server and the daemon running", () => {
    execFileSync(setup, ["/S"], { stdio: "inherit", timeout: 300_000 });
    if (!isAlive(herdrPid)) throw new Error(`the Herdr server ${herdrPid} ended`);
    const status = JSON.parse(output(path.join(runtime, "hide.exe"), ["status", "--json"], isolated));
    if (status.running !== true || status.pid !== connected.pid) throw new Error(`the daemon answered ${JSON.stringify(status)}`);
    if (appsEntries().length !== 1) throw new Error("a second Apps entry appeared");
  });

  // The uninstaller copies itself aside and returns at once; its result is
  // awaited by what it removes. It runs with the private home, as an
  // operator's runs with theirs, so its `hide stop` finds this daemon; the
  // account's own profile stays, since Windows spells the Start menu folder
  // the installer wrote into from %USERPROFILE%.
  const uninstalling = { ...isolated, USERPROFILE: process.env.USERPROFILE };
  execFileSync(path.join(installDir, "Uninstall hide.exe"), ["/S"], { env: uninstalling, stdio: "inherit", timeout: 300_000 });
  await until("the uninstaller removing the app", () => !fs.existsSync(path.join(installDir, "hide.exe")) && appsEntries().length === 0);
  await until("the daemon stopping", () => !isAlive(connected.pid), 30_000);
  check("uninstalling removes the app and stops the daemon, and leaves the Herdr server", () => {
    if (fs.existsSync(startMenuEntry)) throw new Error("the Start menu entry is still there");
    if (!isAlive(herdrPid)) throw new Error(`the Herdr server ${herdrPid} ended`);
    if (!fs.existsSync(path.join(home, ".hide"))) throw new Error("~\\.hide was removed");
  });
} catch (error) {
  console.error(error);
  // A detached daemon's stderr goes nowhere; what it logged says where its start stood.
  for (const record of daemonRecords(isolated.HIDE_STATE_DIR).slice(-60)) console.error(JSON.stringify(record));
  process.exitCode = 1;
} finally {
  // The private Herdr server, then the folder it ran from.
  if (runtime && herdrPid && isAlive(herdrPid)) {
    try {
      execFileSync(path.join(runtime, "herdr.exe"), ["server", "stop"], { env: isolated, stdio: "inherit", timeout: 30_000 });
      await until("the Herdr server ending", () => !isAlive(herdrPid), 15_000);
    } catch (error) {
      console.error(`herdr server stop failed: ${error instanceof Error ? error.message : String(error)}`);
      process.exitCode = 1;
    }
  }
  try {
    if (runtime) execFileSync(path.join(runtime, "hide.exe"), ["stop"], { env: isolated, stdio: "inherit", timeout: 30_000 });
    fs.rmSync(scratch, { recursive: true, force: true });
  } catch (error) {
    console.error(`cleanup failed: ${error instanceof Error ? error.message : String(error)}`);
    process.exitCode = 1;
  }
}
