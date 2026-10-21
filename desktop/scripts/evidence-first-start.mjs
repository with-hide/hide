// TEMPORARY evidence for #1515, removed before merge: repeats the installed
// app's first start (runtime copy into a fresh home, Herdr server from the
// copy, `hide connect`) and prints how long each step of hided's start took.
//
// Usage: node desktop/scripts/evidence-first-start.mjs <installed app folder> <runs>

import { execFileSync, spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { setTimeout as delay } from "node:timers/promises";

const [app, runs] = [process.argv[2], Number(process.argv[3] ?? 5)];
const output = (file, args, env) => execFileSync(file, args, { env, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"], timeout: 120_000 }).trim();

function records(stateDir) {
  const file = path.join(stateDir, "Logs", "core.jsonl");
  if (!fs.existsSync(file)) return [];
  return fs.readFileSync(file, "utf8").split(/\r?\n/).filter(Boolean).map((line) => { try { return JSON.parse(line); } catch { return { line }; } });
}

let failed = 0;
for (let run = 1; run <= runs; run += 1) {
  const scratch = fs.realpathSync.native(fs.mkdtempSync(path.join(os.tmpdir(), "hide-first-start-")));
  const home = path.join(scratch, "home");
  fs.mkdirSync(path.join(home, ".hide"), { recursive: true });
  fs.writeFileSync(path.join(home, ".hide", "product-analytics-disabled"), "");
  const env = Object.fromEntries(Object.entries(process.env).filter(([key]) => !/^(HERDR_|HIDE_)/i.test(key)));
  Object.assign(env, {
    HOME: home, USERPROFILE: home, XDG_CONFIG_HOME: path.join(home, ".config"),
    HERDR_SOCKET_PATH: path.join(scratch, "herdr.sock"),
    HIDE_STATE_DIR: path.join(home, ".hide", "state"),
    HIDE_TAILSCALE_BIN: path.join(home, "no-tailscale"),
  });
  const runtime = JSON.parse(output(path.join(app, "resources", "hide.exe"), ["runtime", "prepare"], env)).dir;
  const herdr = path.join(runtime, "herdr.exe");
  const server = spawn(herdr, ["server"], { env, detached: true, stdio: "ignore", windowsHide: true });
  server.unref();
  for (let i = 0; i < 240; i += 1) {
    try { if (JSON.parse(output(herdr, ["status", "server", "--json"], env)).running === true) break; } catch {}
    await delay(250);
  }
  const began = Date.now();
  const connect = spawnSync(path.join(runtime, "hide.exe"), ["connect"], { env: { ...env, HERDR_BIN_PATH: herdr }, encoding: "utf8", timeout: 180_000 });
  const took = Date.now() - began;
  const steps = records(env.HIDE_STATE_DIR).filter((record) => record.kind === "start.step").map((record) => `${record.stage} ${record.took_ms}`).join(", ");
  console.log(`run ${run}: connect ${took} ms, exit ${connect.status}: ${connect.stdout.trim()}`);
  console.log(`  start steps (ms): ${steps}`);
  if (connect.status !== 0 || took > 3000) {
    if (connect.status !== 0) failed += 1;
    console.log(`  stderr: ${connect.stderr.trim()}`);
    for (const record of records(env.HIDE_STATE_DIR)) console.log(`  ${JSON.stringify(record)}`);
  }
  spawnSync(path.join(runtime, "hide.exe"), ["stop"], { env, stdio: "inherit", timeout: 30_000 });
  spawnSync(herdr, ["server", "stop"], { env, stdio: "inherit", timeout: 30_000 });
  await delay(1000);
  fs.rmSync(scratch, { recursive: true, force: true, maxRetries: 5 });
}
console.log(`${failed} of ${runs} first starts failed`);
