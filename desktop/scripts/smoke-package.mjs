// Proves a Windows or Linux package starts, without a window, on a CI runner
// of that system: no machine of either system is at hand to try a package on
// (operator decision 2026-10-03).
//
// Usage: node desktop/scripts/smoke-package.mjs <installed app folder>
//
// The folder is where the system's installer put the app (PRD auto-update
// D-32): %LOCALAPPDATA%\Programs\hide after the Windows installer, /opt/hide
// after the deb or the rpm. From it: Herdr reports the pinned version, the
// daemon answers in its device node role (`hided node --version`), the hook
// helper starts, `hide runtime prepare` names where the runtime runs (on
// Windows a copy under the private home, outside the install folder; D-10),
// and that runtime's `hide connect` starts its daemon, which serves the web
// shell it embeds, installs the CLI and holds both installed agents for the
// first-run choice (their files untouched), installs both hooks once that
// choice is answered over the daemon's socket, and is replaced by a second
// package fixture with a changed daemon hash, before `hide stop` ends it; on
// Windows the next prepare then removes the first build's copy (D-11). Every
// process runs with a private home and state folder, so none reaches a daemon
// or Herdr of the runner's account.
//
// macOS uses the isolated packaged-app e2e lane instead.

import { execFileSync, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { setTimeout as delay } from "node:timers/promises";

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");
const LABELS = { win32: "windows", linux: "linux" };
const label = LABELS[process.platform];
if (!label) throw new Error(`smoke-package checks Windows and Linux packages only, not ${process.platform}`);
const exe = process.platform === "win32" ? ".exe" : "";

const installed = process.argv[2] ? path.resolve(process.argv[2]) : null;
if (!installed || process.argv.length !== 3) throw new Error("usage: node desktop/scripts/smoke-package.mjs <installed app folder>");

/** One named check: it passes, or the run stops with its name. */
function check(name, body) {
  try {
    const detail = body();
    console.log(`ok ${name}${detail ? `: ${detail}` : ""}`);
  } catch (error) {
    throw new Error(`${name} failed: ${error instanceof Error ? error.message : String(error)}`, { cause: error });
  }
}

const isAlive = (pid) => {
  try { process.kill(pid, 0); return true; }
  catch (error) { return error.code === "EPERM"; }
};

/** Waits for the daemons `pids` to have ended, at most ten seconds; a daemon still running then is a named failure. */
async function daemonsEnded(pids) {
  const deadline = Date.now() + 10_000;
  while ([...pids].some(isAlive) && Date.now() < deadline) await delay(100);
  const running = [...pids].filter(isAlive);
  if (running.length > 0) throw new Error(`hided still running 10 seconds after hide stop (its files are in the scratch folder): pid ${running.join(", ")}`);
}

/**
 * Copies a package folder file by file, links as links. fs.cpSync left the
 * installed Windows package's resources/app.asar behind on the package lane,
 * so the copy is spelled out and every step that fails throws.
 */
function copyTree(source, target) {
  fs.mkdirSync(target, { recursive: true });
  for (const entry of fs.readdirSync(source, { withFileTypes: true })) {
    const from = path.join(source, entry.name);
    const to = path.join(target, entry.name);
    if (entry.isSymbolicLink()) fs.symlinkSync(fs.readlinkSync(from), to);
    else if (entry.isDirectory()) copyTree(from, to);
    else if (entry.isFile()) {
      fs.copyFileSync(from, to);
      fs.chmodSync(to, fs.statSync(from).mode & 0o7777);
    } else throw new Error(`${from} is neither a file, a folder nor a link`);
  }
}

/** A program's stdout; a non-zero exit, a timeout or a spawn failure throws with its stderr. */
const output = (file, args, env) => execFileSync(file, args, { env, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"], timeout: 30_000 }).trim();

// Windows TEMP may use an 8.3 alias. Every fixture path, including the
// expected hook command, starts from the native canonical spelling.
const scratch = fs.realpathSync.native(fs.mkdtempSync(path.join(os.tmpdir(), "hide-smoke-")));
const home = path.join(scratch, "home");
// Keep the state under its selected HOME authority, as the shipped default
// does. An unrelated external override would require authenticating the
// system drive namespace before the kit may treat a missing ledger as empty.
const state = path.join(home, ".hide", "state");

try {
  fs.mkdirSync(home);
  fs.mkdirSync(path.join(home, ".hide"));
  fs.writeFileSync(path.join(home, ".hide", "product-analytics-disabled"), "");
  const app = installed;
  let resources = path.join(app, "resources");
  const bundled = (name) => path.join(resources, `${name}${exe}`);
  // Cleanup always uses the installed package's CLI, even if preparing or
  // connecting the upgrade fails before its CLI can run.
  const controlCli = bundled("hide");

  const electron = path.join(app, `hide${exe}`);

  check("bundled files", () => {
    const expected = [path.join(resources, "app.asar"), electron, ...["hide", "hided", "hide-agent-hooks", "herdr"].map(bundled)];
    // herdr.exe finds its ConPTY runtime in the folder beside it.
    if (process.platform === "win32") expected.push(path.join(resources, "conpty", "conpty.dll"));
    const absent = expected.filter((file) => !fs.statSync(file, { throwIfNoEntry: false })?.isFile());
    if (absent.length > 0) throw new Error(`missing: ${absent.map((file) => path.relative(app, file)).join(", ")}`);
  });

  // What every process below runs with: the account's places moved under the
  // scratch home, and nothing inherited that names a Herdr or a Hide.
  const isolated = Object.fromEntries(Object.entries(process.env).filter(([key]) => !/^(HERDR_|HIDE_)/i.test(key)));
  Object.assign(isolated, {
    HOME: home,
    USERPROFILE: home,
    APPDATA: path.join(home, "AppData", "Roaming"),
    LOCALAPPDATA: path.join(home, "AppData", "Local"),
    XDG_CONFIG_HOME: path.join(home, ".config"),
    CODEX_HOME: path.join(home, ".codex"),
    CLAUDE_CONFIG_DIR: path.join(home, ".claude"),
    HERDR_SOCKET_PATH: path.join(scratch, "herdr.sock"),
    HIDE_STATE_DIR: state,
    HIDE_TAILSCALE_BIN: path.join(home, "no-tailscale"),
    // What the desktop host hands every `hide` child it starts.
    HERDR_BIN_PATH: bundled("herdr"),
  });
  delete isolated.XDG_STATE_HOME;

  // Both runtimes are installed: an agent counts only when its program is
  // found, and the kit searches `~/.local/bin` on every system. The stand-ins
  // answer nothing, and a version Hide cannot read does not hold a hook back.
  const programs = path.join(home, ".local", "bin");
  fs.mkdirSync(programs, { recursive: true });
  for (const name of ["claude", "codex"]) {
    if (process.platform === "win32") fs.writeFileSync(path.join(programs, `${name}.cmd`), "@exit /b 0\r\n");
    else fs.writeFileSync(path.join(programs, name), "#!/bin/sh\nexit 0\n", { mode: 0o755 });
  }
  // Both runtimes are configured, with an unrelated hook we must preserve.
  for (const [folder, file] of [[".claude", "settings.json"], [".codex", "hooks.json"]]) {
    fs.mkdirSync(path.join(home, folder));
    fs.writeFileSync(path.join(home, folder, file), JSON.stringify({
      smokeForeign: "preserve",
      hooks: { SessionStart: [{ hooks: [{ type: "command", command: "echo smoke-foreign" }] }] },
    }));
  }

  check("herdr --version", () => {
    const pinned = JSON.parse(fs.readFileSync(path.join(repo, "contracts", "herdr-bundle.json"), "utf8")).version;
    const reported = output(bundled("herdr"), ["--version"], isolated);
    if (reported.split(/\s+/).at(-1) !== pinned) throw new Error(`reports '${reported}', the pin is ${pinned}`);
    return reported;
  });

  // A device runs this same program in its node role.
  check("hided node --version", () => {
    const reported = output(bundled("hided"), ["node", "--version"], isolated);
    if (!reported.startsWith("hided-node ")) throw new Error(`answered '${reported}'`);
    return reported;
  });

  check("hide-agent-hooks doctor --json", () => {
    JSON.parse(output(bundled("hide-agent-hooks"), ["doctor", "--json"], isolated));
  });

  /**
   * Where the runtime of the package at `cli` runs, as the desktop host asks
   * before it starts anything: a copy under the private home's
   * `.hide/runtime` on Windows, the installed resources folder elsewhere.
   */
  function prepareRuntime(cli) {
    const answer = runtimePrepare(cli);
    if (answer.ok !== true) throw new Error(JSON.stringify(answer));
    const runtimeRoot = path.join(home, ".hide", "runtime");
    if (process.platform === "win32" ? path.dirname(answer.dir) !== runtimeRoot : answer.dir !== path.dirname(cli)) {
      throw new Error(`the runtime runs from ${answer.dir}`);
    }
    if (process.platform === "win32" && answer.dir.toLowerCase().startsWith(app.toLowerCase())) throw new Error("the runtime copy is under the install folder");
    return answer;
  }
  /**
   * `hide runtime prepare`'s answer. What it kept (in use) or could not
   * remove reaches only its stderr, as records the host logs; they are
   * printed so a run shows what each prepare did with the other copies.
   */
  function runtimePrepare(cli) {
    const run = spawnSync(cli, ["runtime", "prepare"], { env: isolated, encoding: "utf8", timeout: 30_000 });
    if (run.error || run.status !== 0) throw new Error(`hide runtime prepare: ${run.error?.message ?? `exit ${run.status}`}: ${run.stderr}`);
    for (const line of run.stderr.split(/\r?\n/).filter(Boolean)) console.log(`  runtime prepare: ${line}`);
    return JSON.parse(run.stdout);
  }

  /** The processes whose program lies under `dir`, as Windows names them. */
  function runningFrom(dir) {
    if (process.platform !== "win32") return "";
    const script = `Get-Process | Where-Object { $_.Path -and $_.Path.StartsWith('${dir.replace(/'/g, "''")}', 'OrdinalIgnoreCase') } | ForEach-Object { "$($_.Id) $($_.Path)" }`;
    return execFileSync("pwsh", ["-NoProfile", "-Command", script], { encoding: "utf8" }).trim() || "none";
  }

  let firstCopy = null;
  check("hide runtime prepare", () => {
    const answer = prepareRuntime(controlCli);
    if (process.platform === "win32") firstCopy = answer.dir;
    resources = answer.dir;
    return `${answer.dir}${answer.copied ? " (copied)" : ""}`;
  });

  // Prepare the complete second package before either daemon holds an
  // executable open. PE and ELF permit an overlay after the image; this
  // changes the production build hash without a second release build.
  const upgraded = path.join(scratch, "new 한글 package");
  check("prepare a second complete package", () => {
    copyTree(app, upgraded);
    for (const file of ["app.asar", ...["hide", "hided", "hide-agent-hooks", "herdr"].map((name) => `${name}${exe}`)]) {
      if (!fs.statSync(path.join(upgraded, "resources", file), { throwIfNoEntry: false })?.isFile()) {
        const holds = (dir) => (fs.existsSync(path.join(dir, "resources")) ? fs.readdirSync(path.join(dir, "resources")).join(", ") : "no resources folder");
        throw new Error(`second package is missing resources/${file}; its resources hold: ${holds(upgraded)}; the installed package's hold: ${holds(app)}`);
      }
    }
    fs.appendFileSync(path.join(upgraded, "resources", `hided${exe}`), "\nHide package smoke next-build fixture\n");
  });

  /**
   * The bundled `hide`'s one JSON line, also when it exits non-zero (a refused
   * connect prints its reason). A `hide` that had to be killed answered nothing:
   * its output staying open past the bound is the failure, whatever it printed.
   */
  function hide(args) {
    try {
      return JSON.parse(output(bundled("hide"), args, isolated));
    } catch (error) {
      const exited = typeof error?.status === "number" && error.signal == null && error.code !== "ETIMEDOUT";
      if (exited && typeof error.stdout === "string" && error.stdout.trim().startsWith("{")) return JSON.parse(error.stdout);
      throw new Error(`hide ${args.join(" ")}: ${error?.stderr || error?.message || error}`);
    }
  }

  /**
   * Why a daemon `hide connect` started ended before it answered: `hide
   * connect` sends its stderr nowhere, so the bundled daemon runs once more
   * on its own, bounded, and its stderr and the newest log lines are named.
   */
  function whyDaemonEnded() {
    const alone = spawnSync(bundled("hided"), [], { env: isolated, encoding: "utf8", timeout: 5_000 });
    const ended = alone.error ? alone.error.message : alone.signal ? `still running after 5 s (${alone.signal})` : `exit ${alone.status}`;
    const lines = fs.existsSync(log) ? fs.readFileSync(log, "utf8").trim().split("\n").slice(-5).join("\n") : "(no log)";
    return `hided alone: ${ended}; stderr: ${alone.stderr?.trim() || "(none)"}\nnewest log lines:\n${lines}`;
  }

  /** A GET to the daemon, its failure named by the path asked. */
  const get = (origin, route) =>
    fetch(`${origin}${route}`, { signal: AbortSignal.timeout(10_000) }).catch((error) => {
      throw new Error(`GET ${route}: ${error.message}`);
    });

  /**
   * Wait on the completed pass, not a fixed startup delay or a live PID.
   * A fresh machine is held for the first-run agent choice: its finished
   * state has the command and the retirement installed and both hooks off
   * (`held`), and only the operator's answer installs them (`applied`).
   */
  const log = path.join(state, "Logs", "core.jsonl");
  async function kitInstalled(after = 0, expect = "applied") {
    const deadline = Date.now() + 25_000;
    let last = "no kit completion";
    while (Date.now() < deadline) {
      const records = (fs.existsSync(log) ? fs.readFileSync(log).subarray(after).toString("utf8") : "").split("\n");
      for (const line of records) {
        let record;
        try { record = JSON.parse(line); } catch { continue; } // the last write may be partial
        if (record.component !== "kit" || record.kind !== "apply.completed") continue;
        last = JSON.stringify(record.components);
        const parts = new Map(record.components.map((part) => [part.id, part]));
        const hooks = expect === "held" ? "off" : "installed";
        if (parts.get("cli")?.state === "installed" && parts.get("coordination_retirement")?.state === "installed"
          && ["claude_code_hook", "codex_hook"].every((id) => parts.get(id)?.state === hooks)) return;
      }
      await delay(100);
    }
    throw new Error(`kit installation did not complete: ${last}`);
  }

  /** What a fresh user has before answering: the command, and no hook of Hide's in either agent's file. */
  function heldFiles() {
    for (const [folder, file] of [[".claude", "settings.json"], [".codex", "hooks.json"]]) {
      const config = JSON.parse(fs.readFileSync(path.join(home, folder, file), "utf8"));
      const text = JSON.stringify(config);
      if (config.smokeForeign !== "preserve" || !text.includes("echo smoke-foreign")) throw new Error(`${folder} lost unrelated settings`);
      if (text.includes("--source hide-subagents@")) throw new Error(`${folder} got Hide's hook before the first-run choice was answered`);
    }
    return "both agents' files untouched until the first-run choice is answered";
  }

  /**
   * The operator's answer to the first-run choice, sent as the web shell sends
   * it. The event does nothing until this Mac's kit has reported, so it is
   * sent again until the answered pass shows in the log.
   */
  async function answerFirstRunChoice(port, agents, after) {
    const { token } = JSON.parse(fs.readFileSync(path.join(state, "hided.json"), "utf8"));
    // The daemon admits a socket from its own origin; Node sends none unless told to.
    const socket = new WebSocket(`ws://127.0.0.1:${port}/ws`, { headers: { Origin: `http://127.0.0.1:${port}` } });
    await new Promise((resolve, reject) => {
      socket.addEventListener("open", resolve, { once: true });
      socket.addEventListener("error", () => reject(new Error("the daemon's socket did not open")), { once: true });
    });
    try {
      socket.send(JSON.stringify({ token, schema_version: 2, client_kind: "web", have_revision: 0, have_terminal_sequence: 0 }));
      const deadline = Date.now() + 25_000;
      while (Date.now() < deadline) {
        socket.send(JSON.stringify({ schema_version: 2, kind: "agent_onboarding_apply", payload: { agents } }));
        await delay(1_000);
        const records = fs.readFileSync(log).subarray(after).toString("utf8").split("\n");
        if (records.some((line) => line.includes('"apply.completed"') && line.includes('"scope":"operator"'))) return;
      }
      throw new Error("the first-run choice was not applied");
    } finally {
      socket.close();
    }
  }

  function installedFiles() {
    const localBin = path.join(home, ".local", "bin");
    const command = path.join(localBin, process.platform === "win32" ? "hide.cmd" : "hide");
    const shellEnv = { ...isolated, PATH: `${localBin}${path.delimiter}${isolated.PATH ?? isolated.Path ?? ""}` };
    // Windows environment names are case-insensitive; Node otherwise chooses
    // the first spelling and may silently discard our temporary PATH.
    for (const key of Object.keys(shellEnv)) if (key.toLowerCase() === "path" && key !== "PATH") delete shellEnv[key];
    const resolved = process.platform === "win32"
      ? output("pwsh", ["-NoProfile", "-Command", "(Get-Command hide -CommandType Application).Source"], shellEnv)
      : output("/bin/sh", ["-c", "command -v hide"], shellEnv);
    const resolvedId = fs.statSync(resolved, { bigint: true });
    const commandId = fs.statSync(command, { bigint: true });
    if (resolvedId.dev !== commandId.dev || resolvedId.ino !== commandId.ino) throw new Error(`new shell resolved ${resolved}, expected ${command}`);
    const answer = process.platform === "win32"
      ? output("pwsh", ["-NoProfile", "-Command", "hide status --json"], shellEnv)
      : output("/bin/sh", ["-c", "hide status --json"], shellEnv);
    if (JSON.parse(answer).running !== true) throw new Error(`installed command answered ${answer}`);
    const destination = process.platform === "win32" ? path.join(localBin, ".hide-kit") : command;
    const expected = process.platform === "win32" ? resources : bundled("hide");
    // A Windows junction can resolve to the long spelling while TEMP keeps
    // its 8.3 spelling. Compare the objects, not equivalent path strings.
    const actualId = fs.statSync(destination, { bigint: true });
    const expectedId = fs.statSync(expected, { bigint: true });
    if (actualId.dev !== expectedId.dev || actualId.ino !== expectedId.ino) {
      throw new Error(`the command leads to ${fs.realpathSync(destination)}, expected ${fs.realpathSync(expected)} (file identities differ)`);
    }
    for (const [folder, file] of [[".claude", "settings.json"], [".codex", "hooks.json"]]) {
      const config = JSON.parse(fs.readFileSync(path.join(home, folder, file), "utf8"));
      const hooks = Object.values(config.hooks).flat().flatMap((group) => group.hooks ?? []);
      if (config.smokeForeign !== "preserve" || !hooks.some((hook) => hook.command === "echo smoke-foreign")) throw new Error(`${folder} lost unrelated settings`);
      const owned = hooks.filter((hook) => JSON.stringify(hook).includes("--source hide-subagents@"));
      if (owned.length === 0 || owned.some((hook) => ![hook.command, ...(hook.args ?? [])].join(" ").includes(bundled("hide-agent-hooks")))) throw new Error(`${folder} hook paths are missing or stale`);
      if (process.platform === "win32" && owned.some((hook) => folder === ".claude"
        ? hook.command !== "powershell.exe" || !hook.args?.includes("-Command")
        : !hook.command.startsWith("if (Test-Path -LiteralPath"))) throw new Error(`${folder} hooks do not use PowerShell`);
    }
    return "command resolves in a fresh shell; both hooks installed; foreign settings preserved; coordination retirement complete";
  }

  let connected = null;
  // Every daemon this run started: the scratch folder is removed only once
  // none of them still runs, because a running one holds files in it.
  const daemonPids = new Set();
  try {
    connected = hide(["connect"]);
    if (connected.ok === true) daemonPids.add(connected.pid);
    check("hide connect starts the bundled daemon", () => {
      if (connected.ok !== true) throw new Error(`${JSON.stringify(connected)}${connected.reason === "start_failed" ? `\n${whyDaemonEnded()}` : ""}`);
      return `pid ${connected.pid}, port ${connected.port}`;
    });
    await kitInstalled(0, "held");
    check("first launch installs the command and holds both agents for the first-run choice", heldFiles);
    const beforeChoice = fs.statSync(log).size;
    await answerFirstRunChoice(connected.port, ["claude-code", "codex"], beforeChoice);
    await kitInstalled(beforeChoice);
    check("the first-run choice installs both hooks", installedFiles);
    const origin = `http://127.0.0.1:${connected.port}`;
    const health = await (await get(origin, "/health")).json();
    check("the daemon answers /health", () => {
      if (health.pid !== connected.pid) throw new Error(`answered ${JSON.stringify(health)}`);
      return `build ${health.build}`;
    });
    const page = await get(origin, "/");
    const html = await page.text();
    const script = /<script[^>]+src="(\/assets\/[^"]+\.js)"/.exec(html)?.[1];
    const asset = script ? await get(origin, script) : null;
    check("the daemon serves the web shell it embeds", () => {
      if (page.status !== 200 || !html.includes('<div id="root">')) throw new Error(`/ answered ${page.status} without the shell's root`);
      if (!script) throw new Error("the shell's page names no script under /assets");
      if (asset.status !== 200) throw new Error(`${script} answered ${asset.status}`);
      return script;
    });
    check("the second package's runtime", () => {
      const answer = prepareRuntime(path.join(upgraded, "resources", `hide${exe}`));
      if (process.platform === "win32" && answer.dir === firstCopy) throw new Error("a changed build reused the first build's copy");
      resources = answer.dir;
    });
    const beforeUpgrade = fs.statSync(log).size;
    const previous = connected;
    connected = hide(["connect"]);
    if (connected.ok === true) daemonPids.add(connected.pid);
    check("a newer package replaces the running daemon", () => {
      if (connected.ok !== true || connected.pid === previous.pid) throw new Error("new package did not replace the old daemon");
      return "new daemon PID";
    });
    const nextHealth = await (await get(`http://127.0.0.1:${connected.port}`, "/health")).json();
    check("the new daemon answers with the new build hash", () => {
      const expected = createHash("sha256").update(fs.readFileSync(bundled("hided"))).digest("hex");
      if (nextHealth.pid !== connected.pid || nextHealth.build !== expected || nextHealth.build === health.build) throw new Error("the second package's daemon did not answer");
    });
    await kitInstalled(beforeUpgrade);
    check("upgrade moves the command and both hooks to the new package", installedFiles);
    check("reconnecting the same package keeps its daemon", () => {
      if (hide(["connect"]).pid !== connected.pid) throw new Error("same-build connect replaced the daemon");
    });

    // A plain development-shaped copy still cannot replace a packaged daemon.
    const standalone = path.join(scratch, "target", "release");
    fs.mkdirSync(standalone, { recursive: true });
    for (const name of ["hide", "hided"]) fs.copyFileSync(path.join(app, "resources", `${name}${exe}`), path.join(standalone, `${name}${exe}`));
    check("a standalone build refuses replacement", () => {
      let refused = false;
      try { output(path.join(standalone, `hide${exe}`), ["connect"], isolated); }
      catch (error) { refused = error.status !== 0 && String(error.stdout).includes("other_build"); }
      if (!refused || hide(["status", "--json"]).pid !== connected.pid) throw new Error("standalone connect did not leave the packaged daemon alone");
    });
  } finally {
    // Asked whatever happened above: a daemon a failed connect left running is
    // this run's too, and stopping none is not an error. A failure here is
    // reported, after the scratch folder is gone, beside any failure above.
    try {
      execFileSync(controlCli, ["stop"], { env: isolated, stdio: "inherit", timeout: 30_000 });
      if (connected?.ok === true) {
        check("hide stop ends it", () => {
          const status = JSON.parse(output(controlCli, ["status", "--json"], isolated));
          if (status.running !== false) throw new Error(JSON.stringify(status));
        });
      }
    } catch (error) {
      console.error(`hide stop failed: ${error instanceof Error ? error.message : String(error)}`);
      process.exitCode = 1;
    }
    try {
      await daemonsEnded(daemonPids);
    } catch (error) {
      console.error(error instanceof Error ? error.message : String(error));
      process.exitCode = 1;
    }
  }
  // Nothing runs from the first build's copy now, so the next start removes it.
  if (firstCopy) {
    check("a later start removes the copy nothing runs from", () => {
      const answer = runtimePrepare(path.join(upgraded, "resources", `hide${exe}`));
      if (fs.existsSync(firstCopy) || !answer.removed.includes(path.basename(firstCopy))) {
        const held = fs.existsSync(firstCopy) ? fs.readdirSync(firstCopy).join(", ") : "gone";
        throw new Error(`${firstCopy} is still there: ${JSON.stringify(answer)}; it holds: ${held}; running from it: ${runningFrom(firstCopy)}; the runtime folder holds: ${fs.readdirSync(path.dirname(firstCopy)).join(", ")}`);
      }
    });
  }
} catch (error) {
  console.error(error);
  process.exitCode = 1;
} finally {
  try {
    fs.rmSync(scratch, { recursive: true, force: true });
  } catch (error) {
    console.error(`scratch cleanup failed: ${error instanceof Error ? error.message : String(error)}`);
    process.exitCode = 1;
  }
}
