// The C programs the e2e fixtures run in place of a real provider, opener and
// process programs. They are compiled once, by the e2e entry point's
// `globalSetup`, into `web/.e2e-shims` (ignored by git), each named with a
// hash of its source: a test only copies a finished program, so no test
// holds a compiler and a stale program cannot be mistaken for the current one.
// A vitest file builds the programs it runs in its `beforeAll` instead; the
// compiler runs as a bounded child the build awaits, so the worker that runs
// that file keeps answering vitest while clang runs: a synchronous build of
// every program on a Windows runner held one for 81 s, and vitest failed the
// suite on the worker's missed report (with-hide/hide-private#48).
import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { BoundedChildError, runBounded } from "../bounded-child";

const windows = process.platform === "win32";
const ext = windows ? ".exe" : "";

/** A program the fixtures copy, by the name of its source in `web/e2e/shims`. */
export type FixtureShim = "claude-shim" | "spawn-provider" | "noop" | "launcher" | "hide-children" | "hide-processes";

/** The programs this system builds: the launcher and the two process programs only exist for Windows. */
const BUILT: FixtureShim[] = windows ? ["claude-shim", "spawn-provider", "noop", "launcher", "hide-children", "hide-processes"] : ["claude-shim", "spawn-provider", "noop"];

// The programs a run built, by name, handed to the tests through the environment:
// Playwright starts its workers after `globalSetup`, so they inherit it.
const PROGRAMS = "HIDE_E2E_SHIMS";

const hashed = (source: string, name: FixtureShim): string => {
  const digest = createHash("sha256").update(fs.readFileSync(source));
  if (name === "claude-shim") digest.update(fs.readFileSync(path.join(path.dirname(source), "live-check.h")));
  return `${name}-${digest.digest("hex").slice(0, 12)}${ext}`;
};

/**
 * One compile's bounds: with the 30 s the tree's end may take, inside the
 * 120 s a vitest `beforeAll` gives its build, so a compiler that hangs is
 * ended before the hook is abandoned; the output cap is `execFileSync`'s
 * default.
 */
const COMPILE = { timeoutMs: 60_000, outputBytes: 1024 * 1024 };

/**
 * Compiles each of `programs` (every program this system builds unless
 * named) whose current source has no finished build, removes older builds of
 * it, and records the finished programs for the tests. `packageDir` is the
 * folder of the config that is running (`web` or `desktop`, siblings): the
 * sources and the build folder are found from it, not from the working
 * directory.
 */
export async function buildFixtureShims(packageDir: string, programs: readonly FixtureShim[] = BUILT): Promise<void> {
  const unbuilt = programs.filter((name) => !BUILT.includes(name));
  if (unbuilt.length > 0) throw new Error(`fixture programs ${unbuilt.join(", ")} are not built on ${process.platform}`);
  const sourceDir = path.resolve(packageDir, "..", "web", "e2e", "shims");
  const outputDir = path.resolve(packageDir, "..", "web", ".e2e-shims");
  fs.mkdirSync(outputDir, { recursive: true });
  const compiler = windows ? "clang.exe" : "cc";
  const built: Record<string, string> = {};
  // On Windows `hide-processes` is built first and follows every later
  // compile's tree; its own compile has nothing to follow it with, so a
  // failure there that is not the compiler's own exit keeps its partial file.
  const order: FixtureShim[] = windows ? ["hide-processes", ...programs.filter((name) => name !== "hide-processes")] : [...programs];
  for (const name of order) {
    const source = path.join(sourceDir, `${name}.c`);
    const program = path.join(outputDir, hashed(source, name));
    built[name] = program;
    if (fs.existsSync(program)) continue;
    const partial = `${program.slice(0, program.length - ext.length)}.${process.pid}.partial${ext}`;
    let failure: unknown;
    let output = "";
    try {
      const done = await runBounded(compiler, ["-O1", "-o", partial, source], { ...COMPILE, processTool: built["hide-processes"] });
      if (done.status !== 0) failure = new Error(`exit status ${done.status}, signal ${done.signal}`);
      output = done.stdout + done.stderr;
    } catch (error) {
      if (!(error instanceof BoundedChildError)) throw error;
      // A compiler whose tree may still run may still write the partial file.
      if (!error.treeEnded) throw new Error(`fixture C compiler ${compiler} failed on ${source}, and ${partial} is left in place: ${error.message}`, { cause: error });
      // A compiler that cannot start has no output.
      failure = error;
      output = error.stdout + error.stderr;
    }
    if (failure) {
      fs.rmSync(partial, { force: true });
      throw new Error(`fixture C compiler ${compiler} failed on ${source}:\n${output || String(failure)}`, { cause: failure });
    }
    // Another build of the same source may have finished first; keep its program.
    if (fs.existsSync(program)) {
      fs.rmSync(partial, { force: true });
      continue;
    }
    fs.renameSync(partial, program);
    // Older finished builds of this program; another process's partial file is not one.
    const finished = new RegExp(`^${name}-[0-9a-f]{12}${windows ? "\\.exe" : ""}$`);
    for (const old of fs.readdirSync(outputDir)) {
      if (finished.test(old) && path.join(outputDir, old) !== program) fs.rmSync(path.join(outputDir, old), { force: true });
    }
  }
  process.env[PROGRAMS] = JSON.stringify(built);
}

/** The finished program `name`, run where it is built; a program that was not built is a failure that says how to build it. */
export function fixtureShimPath(name: FixtureShim): string {
  const programs = JSON.parse(process.env[PROGRAMS] ?? "{}") as Record<string, string>;
  const program = programs[name];
  if (!program || !fs.existsSync(program)) {
    throw new Error(`fixture program ${name} is not built (${program ?? `no ${PROGRAMS} in the environment`}); Playwright's globalSetup builds it, so run the e2e through \`playwright test\` (scripts/verify-web.sh web e2e)`);
  }
  return program;
}

/** Copies the finished `name` to `executable`; a program that was not built is a failure that says how to build it. */
export function copyFixtureShim(name: FixtureShim, executable: string): void {
  fs.copyFileSync(fixtureShimPath(name), executable);
}
