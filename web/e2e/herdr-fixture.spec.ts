// @platform: The real pinned CLI starts two native pane processes together;
// a partial start failure must release the private server and Windows locks.
import { expect, test } from "@playwright/test";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { herdrBinary, startHerdr } from "./herdr-fixture";
import { fixtureProgram } from "./platform-fixture";

/** A CLI-side rendezvous orders the starts, without substituting any Herdr answer. */
async function withStartBarrier(failFirst: boolean, journey: (root: string) => Promise<void>): Promise<void> {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "hide-start-barrier-"));
  const binary = herdrBinary();
  const previous = process.env.HIDE_E2E_HERDR_BIN;
  try {
    const proxy = fixtureProgram(root, "herdr-start-barrier", `
const fs = require("node:fs"), path = require("node:path");
const { execFileSync } = require("node:child_process");
const root = ${JSON.stringify(root)}, binary = ${JSON.stringify(binary)};
const args = process.argv.slice(2);
if (args[0] === "server" && args.length === 1) {
  fs.writeFileSync(path.join(root, "fixture-root"), process.env.HIDE_E2E_ROOT);
}
if (args[0] === "agent" && args[1] === "start") {
  fs.writeFileSync(path.join(root, args[2] + ".arrived"), "");
  const deadline = Date.now() + 10000;
  while (!["one", "two"].every(name => fs.existsSync(path.join(root, name + ".arrived")))) {
    if (Date.now() >= deadline) throw new Error("second start never arrived while the first was pending");
    Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 25);
  }
  if (${failFirst} && args[2] === "one") args[args.indexOf("--kind") + 1] = "invalid-fixture-kind";
}
try {
  const out = execFileSync(binary, args, { env: process.env, timeout: 30000 });
  if (args[0] === "agent" && args[1] === "start") fs.writeFileSync(path.join(root, args[2] + ".completed"), "");
  process.stdout.write(out);
} catch (error) {
  if (error.stdout) process.stdout.write(error.stdout);
  if (error.stderr) process.stderr.write(error.stderr);
  process.exit(typeof error.status === "number" ? error.status : 1);
}
`);
    process.env.HIDE_E2E_HERDR_BIN = proxy;
    await journey(root);
  } finally {
    if (previous === undefined) delete process.env.HIDE_E2E_HERDR_BIN;
    else process.env.HIDE_E2E_HERDR_BIN = previous;
    fs.rmSync(root, { recursive: true, force: true });
  }
}

test("fixture starts overlap and both panes accept their first input", { tag: "@platform" }, async () => {
  await withStartBarrier(false, async (root) => {
    const fixture = await startHerdr();
    try {
      for (const [index, pane] of fixture.panes.entries()) {
        expect(fs.existsSync(path.join(root, ["one", "two"][index]! + ".completed"))).toBe(true);
        const marker = `first-input-${index}`;
        fixture.run(["pane", "send-text", pane, marker]);
        await expect.poll(() => fs.existsSync(fixture.inputLogs[index]!) ? fs.readFileSync(fixture.inputLogs[index]!, "utf8") : "").toBe(marker);
      }
    } finally {
      fixture.stop();
    }
    expect(fs.existsSync(fixture.root)).toBe(false);
  });
});

test("fixture waits for the sibling start before cleaning up a failed pair", { tag: "@platform" }, async () => {
  await withStartBarrier(true, async (root) => {
    const started = startHerdr();
    // A regression that unexpectedly succeeds must still release its server.
    try {
      await expect(started).rejects.toThrow("fixture agent start failed");
    } finally {
      const result = await Promise.allSettled([started]);
      if (result[0]!.status === "fulfilled") result[0]!.value.stop();
    }
    expect(fs.existsSync(path.join(root, "two.completed"))).toBe(true);
    const fixtureRoot = fs.readFileSync(path.join(root, "fixture-root"), "utf8");
    expect(fs.existsSync(fixtureRoot)).toBe(false);
  });
});
