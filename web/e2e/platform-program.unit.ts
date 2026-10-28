// The portable fixture executable must preserve the caller's data through
// the real program boundary, including the Windows launcher. Nothing here
// waits synchronously: the build and the program run as bounded children this
// worker awaits, so it keeps answering vitest (with-hide/hide-private#48).
import { beforeAll, expect, test } from "vitest";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { runBounded } from "./bounded-child";
import { fixtureProgram } from "./platform-fixture";
import { buildFixtureShims } from "./shims/build";

beforeAll(async () => {
  if (process.platform === "win32") await buildFixtureShims(fileURLToPath(new URL("..", import.meta.url)), ["launcher"]);
}, 120_000);

/** Each run's bounds: the 10 s the synchronous calls had, and far more output than the program writes. */
const RUN = { timeoutMs: 10_000, outputBytes: 64 * 1024 };

// The test's limit holds both runs at their bound and the teardown of the second.
test("fixture programs preserve Unicode paths, arguments, quoting and the child exit status", { timeout: 25_000 }, async ({ signal }) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "hide-program-"));
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
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});
