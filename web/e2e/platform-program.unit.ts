// The portable fixture executable must preserve the caller's data through
// the real program boundary, including the Windows launcher. Nothing here
// waits synchronously: the build and the program run as children this
// worker awaits, so it keeps answering vitest (with-hide/hide-private#48).
import { beforeAll, expect, test } from "vitest";
import { spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { fixtureProgram } from "./platform-fixture";
import { buildFixtureShims } from "./shims/build";

beforeAll(async () => {
  if (process.platform === "win32") await buildFixtureShims(fileURLToPath(new URL("..", import.meta.url)), ["launcher"]);
}, 120_000);

/** Runs `executable` to its end and hands back its exit status and output. */
function run(executable: string, args: string[]): Promise<{ status: number | null; stdout: string; stderr: string }> {
  return new Promise((resolve, reject) => {
    const child = spawn(executable, args, { windowsHide: true, stdio: ["ignore", "pipe", "pipe"] });
    let stdout = "";
    let stderr = "";
    child.stdout.setEncoding("utf8").on("data", (chunk: string) => { stdout += chunk; });
    child.stderr.setEncoding("utf8").on("data", (chunk: string) => { stderr += chunk; });
    child.once("error", reject);
    child.once("close", (status) => resolve({ status, stdout, stderr }));
  });
}

// The two runs keep the 10 s each the synchronous calls were given.
test("fixture programs preserve Unicode paths, arguments, quoting and the child exit status", { timeout: 20_000 }, async () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "hide-program-"));
  try {
    const executable = fixtureProgram(path.join(root, "한국어 폴더 🙂"), "echo-arguments", `
const args = process.argv.slice(2);
if (args[0] === "exit") process.exit(Number(args[1]));
process.stdout.write(JSON.stringify(args));
`);
    const args = ["issue", "create", "--title", "한국어 제목 🙂", "--body", '첫 줄\n"본문"과 C:\\fixture\\path', "", "quote\"inside", "trailing\\"];
    const answer = await run(executable, args);
    expect(answer.status).toBe(0);
    expect(answer.stderr).toBe("");
    expect(JSON.parse(answer.stdout)).toEqual(args);
    const refused = await run(executable, ["exit", "37"]);
    expect(refused.status).toBe(37);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});
