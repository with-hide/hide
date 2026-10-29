// The Windows process ending every fixture teardown relies on, against real
// processes: a child its owner started after the owner was listed, left
// running when the owner was killed, is ended with the owner. Windows only.
import { beforeAll, expect, test } from "vitest";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import { endWindowsProcesses, windowsProcessTree } from "../../web/e2e/platform-fixture";
import { buildFixtureShims } from "../../web/e2e/shims/build";

// vitest has no `globalSetup` of Playwright's, so this file builds the
// programs the teardown runs, the same way, before it uses one.
beforeAll(async () => {
  if (process.platform === "win32") await buildFixtureShims(fileURLToPath(new URL("..", import.meta.url)), ["hide-processes"]);
}, 120_000);

test("a child its owner started after the owner was listed is ended once the owner is gone", { timeout: 30_000 }, async (context) => {
  if (process.platform !== "win32") return context.skip();
  const owner = spawn(process.env.ComSpec ?? "cmd.exe", ["/d", "/c", "ping -n 120 127.0.0.1 >nul"], { windowsHide: true, stdio: "ignore" });
  const listed = windowsProcessTree(owner.pid!).filter((process) => process.id === owner.pid);
  expect(listed).toHaveLength(1);
  let child: { id: number; created: string } | undefined;
  try {
    await expect.poll(() => (child = windowsProcessTree(owner.pid!).find((process) => process.id !== owner.pid)), { timeout: 10_000 }).toBeDefined();
    const exited = new Promise((resolve) => owner.once("exit", resolve));
    owner.kill();
    await exited;
    endWindowsProcesses(listed);
    expect(windowsProcessTree(child!.id).filter((process) => process.created === child!.created)).toEqual([]);
  } finally {
    if (child && windowsProcessTree(child.id).some((process) => process.created === child!.created)) process.kill(child.id);
  }
});
