// The one rule for ending the pane holders a daemon's own engine left running
// (PRD engine-bundle-3-core-owns), shared by the e2e fixture
// (`hided-fixture.ts`) and the web shell measurement
// (`scripts/web-shell-measure/run.sh`, through the command below). A holder
// outlives its daemon by design, so the next daemon can adopt it, which makes
// whoever started the daemon its owner once the daemon is stopped: each holder
// is the pid its record under `<state>/holders/records` names. On Unix a pid
// counts as that holder only while its command line still names the record,
// since a freed pid may belong to another process; Windows ends the holder's
// tree as the caller lists it now. A holder ends its program when it goes (the
// pane's terminal hangs up). Leaked holders once used up every PTY of the
// machine, so a clean end leaves nothing that names the state folder.
// Plain JavaScript so the measurement runs it with Node; it has no
// `import.meta`, since the desktop suite's loader compiles it as CommonJS.
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import process from "node:process";

export const HOLDER_END_MS = 10_000;

function records(stateDir) {
  const folder = path.join(stateDir, "holders", "records");
  if (!fs.existsSync(folder)) return [];
  return fs.readdirSync(folder).filter((name) => name.endsWith(".json")).map((name) => {
    const file = path.join(folder, name);
    const record = JSON.parse(fs.readFileSync(file, "utf8"));
    return { file, holder: record.holder?.pid, program: record.program?.pid };
  }).filter((row) => row.holder !== undefined);
}

/** The command line of `pid`, or "" once it no longer runs (`ps` exits 1, printing nothing). */
function commandOf(pid) {
  try {
    return execFileSync("ps", ["-o", "command=", "-p", String(pid)], { encoding: "utf8", timeout: 5_000 });
  } catch (error) {
    if (error.status === 1) return "";
    throw error;
  }
}

/** The Unix holders of `stateDir` still running as their records name them, with their programs. */
export function ownedHolders(stateDir) {
  return records(stateDir).filter((row) => commandOf(row.holder).includes(row.file));
}

/** Every process line whose command names `<stateDir>/holders/` (Unix). */
export function holderStrays(stateDir) {
  const marker = `${path.join(stateDir, "holders")}${path.sep}`;
  return execFileSync("ps", ["-Ao", "pid=,command="], { encoding: "utf8", timeout: 5_000 })
    .split("\n").filter((line) => line.includes(marker));
}

/**
 * Ends the holders of `stateDir` and their programs. `endTree(pid)` ends a
 * holder's process tree on Windows, where no command line is read. Throws,
 * naming `stateDir`, when one still runs after HOLDER_END_MS or, on Unix,
 * when anything still names the state folder's holders.
 */
export function endPaneHolders(stateDir, { endTree } = {}) {
  const owned = [];
  if (endTree) {
    for (const row of records(stateDir)) endTree(row.holder);
  } else {
    for (const row of ownedHolders(stateDir)) {
      process.kill(row.holder, "SIGTERM");
      owned.push(row.holder);
      if (row.program !== undefined) owned.push(row.program);
    }
  }
  const running = () => owned.filter((pid) => {
    try {
      process.kill(pid, 0);
      return true;
    } catch {
      return false;
    }
  });
  const deadline = Date.now() + HOLDER_END_MS;
  while (running().length > 0 && Date.now() < deadline) {
    // A synchronous worker-exit cleanup cannot await a timer.
    Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 50);
  }
  const left = running();
  if (left.length > 0) throw new Error(`pane holder processes ${left.join(", ")} still run ${HOLDER_END_MS / 1000} s after their end; preserve ${stateDir}`);
  if (endTree) return;
  const strays = holderStrays(stateDir);
  if (strays.length > 0) throw new Error(`processes of ${stateDir} still run after its holders ended: ${strays.join("; ")}`);
}

