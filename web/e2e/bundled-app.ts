// The debug build laid out as an installed app's resources, the one place a
// daemon runs the install kit (`hide_kit::bundled_kit_dir` reads the layout by
// name on every system), so a fixture daemon meets what a packaged one does,
// the first-run choice included. The binaries are hard links: `hide connect`
// compares builds by content, so the bundle is the build it was linked from,
// with the holder's runtime copied beside them on Windows. A link cannot cross
// volumes, and a Windows runner's temporary folder is on another drive, so the bundle sits beside the build
// in `target/debug`, one per worker, and is removed when the worker exits.
//
// A packaged app also carries the pinned Herdr in its resources, which the kit
// installs agents' Herdr integrations with. The plain bundle leaves it out, so
// a daemon's kit sees no Herdr CLI and touches no agent's Herdr integration; a
// spec about those integrations asks for the `herdr` bundle, a second one per
// worker, so whether a daemon has Herdr never depends on which spec ran first.

import fs from "node:fs";
import path from "node:path";
import { fixtureExecutable } from "./platform-fixture";
// Its exit handler stops every daemon the worker started; registered on import,
// it runs before this module's, so no daemon still runs from a removed bundle.
import "./worker-owned";

// Both suites run from their package folder (`web`, `desktop`), one below the repository.
const DEBUG = path.resolve("..", "target", "debug");
const BUNDLES = path.join(DEBUG, "e2e-apps");
/** The build's programs a bundle carries, the pane holder among them. */
const binaries = (): string[] => ["hided", "hide", "hide-agent-hooks", "hide-holder"];

/** `plain` is the build alone; `herdr` also carries the fixture's pinned Herdr, as a packaged app does. */
export type Bundle = "plain" | "herdr";
const root = (bundle: Bundle) => path.join(BUNDLES, bundle === "plain" ? String(process.pid) : `${process.pid}-herdr`);
const resources = (bundle: Bundle) => path.join(root(bundle), "hide.app", "Contents", "Resources");
const linked = new Set<Bundle>();

/** Where this worker's bundle keeps `name` (`hided`, `hide`, `hide-agent-hooks`); `linkBundle` makes it. */
export function bundledExecutable(name: string, bundle: Bundle = "plain"): string {
  return path.join(resources(bundle), fixtureExecutable(name));
}

/**
 * Links this worker's bundle the first time a daemon is about to run from it; the `herdr` bundle also takes
 * `herdrBin`, linked where a link can be made and copied where the pinned binary is on another volume.
 */
export function linkBundle(bundle: Bundle = "plain", herdrBin?: string): void {
  if (linked.has(bundle)) return;
  if (bundle === "herdr" && !herdrBin) throw new Error("the herdr bundle needs the fixture's Herdr binary");
  if (linked.size === 0) removeOrphans();
  const folder = root(bundle);
  // A folder under this pid is an earlier process's that the pid was reused from.
  fs.rmSync(folder, { recursive: true, force: true });
  fs.mkdirSync(resources(bundle), { recursive: true });
  for (const name of binaries()) {
    const built = path.join(DEBUG, fixtureExecutable(name));
    if (!fs.existsSync(built)) throw new Error(`${built} is missing; the lane must build it (scripts/verify-cargo.sh cli, and -p hide-holder)`);
    fs.linkSync(built, bundledExecutable(name, bundle));
  }
  if (process.platform === "win32") {
    // The node resolves the pinned runtime beside hided, as in the packaged app.
    // A missing lane input fails here, before any pane starts from this bundle.
    fs.cpSync(path.join(DEBUG, "holder-conpty"), path.join(resources(bundle), "holder-conpty"), { recursive: true });
  }
  if (herdrBin) {
    const target = bundledExecutable("herdr", bundle);
    try {
      fs.linkSync(herdrBin, target);
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "EXDEV") throw error;
      fs.copyFileSync(herdrBin, target);
      fs.chmodSync(target, 0o755);
    }
  }
  process.once("exit", () => {
    try {
      fs.rmSync(folder, { recursive: true, force: true });
    } catch (error) {
      console.error(`e2e app bundle ${folder} was not removed at worker exit`, error);
    }
  });
  linked.add(bundle);
}

// A worker that was killed never ran its exit handler; its bundle would keep a
// rebuilt binary's old copy on disk, so the bundles of workers that are gone go.
function removeOrphans(): void {
  if (!fs.existsSync(BUNDLES)) return;
  for (const entry of fs.readdirSync(BUNDLES)) {
    // `<pid>` or `<pid>-herdr`.
    const pid = Number(entry.split("-")[0]);
    if (Number.isInteger(pid) && pid > 0 && pid !== process.pid && !alive(pid)) {
      fs.rmSync(path.join(BUNDLES, entry), { recursive: true, force: true });
    }
  }
}

function alive(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return (error as NodeJS.ErrnoException).code === "EPERM";
  }
}
