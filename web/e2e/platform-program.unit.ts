// The portable fixture executable must preserve the caller's data through
// the real program boundary, including the Windows launcher. Nothing here
// waits synchronously: the build and the program run as bounded children this
// worker awaits, so it keeps answering vitest (with-hide/hide-private#48).
import { beforeAll, expect, test } from "vitest";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { BoundedChildError, runBounded, type Bounds } from "./bounded-child";
import { fixtureProgram } from "./platform-fixture";
import { buildFixtureShims, fixtureShimPath } from "./shims/build";

/** Each run's bounds: the 10 s the synchronous calls had, and far more output than the program writes. */
const RUN: Bounds = { timeoutMs: 10_000, outputBytes: 64 * 1024 };

beforeAll(async () => {
  if (process.platform !== "win32") return;
  await buildFixtureShims(fileURLToPath(new URL("..", import.meta.url)), ["launcher", "hide-processes"]);
  RUN.processTool = fixtureShimPath("hide-processes");
}, 120_000);

// Each run keeps its 10 s; the limit adds the 10 s an ended tree's teardown may take.
test("fixture programs preserve Unicode paths, arguments, quoting and the child exit status", { timeout: 30_000 }, async ({ signal }) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "hide-program-"));
  // A program whose tree was not confirmed ended may still hold its folder.
  let held = false;
  try {
    const executable = fixtureProgram(path.join(root, "한국어 폴더 🙂"), "echo-arguments", `
const args = process.argv.slice(2);
if (args[0] === "exit") process.exit(Number(args[1]));
process.stdout.write(JSON.stringify(args));
`);
    const args = ["issue", "create", "--title", "한국어 제목 🙂", "--body", '첫 줄\n"본문"과 C:\\fixture\\path', "", "quote\"inside", "trailing\\"];
    const answer = await runBounded(executable, args, { ...RUN, signal });
    expect(answer.status).toBe(0);
    expect(answer.stderr).toBe("");
    expect(JSON.parse(answer.stdout)).toEqual(args);
    const refused = await runBounded(executable, ["exit", "37"], { ...RUN, signal });
    expect(refused.status).toBe(37);
  } catch (error) {
    held = error instanceof BoundedChildError && !error.treeEnded;
    throw error;
  } finally {
    if (!held) fs.rmSync(root, { recursive: true, force: true });
  }
});
