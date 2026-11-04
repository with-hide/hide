// The local measurement CLI uses the existing compiled providers on the real
// pinned Herdr. This is tool plumbing, never authenticated model acceptance.
import { test, expect } from "@playwright/test";
import { runPython } from "./live-check-process";

test("live check guards preserve bytes, refuse aliases, and end detached children", async () => {
  test.skip(process.platform === "win32", "The local measurement runs on Unix hosts");
  const result = await runPython(["-m", "unittest", "discover", "-s", "../scripts/tests", "-p", "test_agent_live_*.py"],
    { timeout: 30_000 });
  expect(result.error, result.stderr).toBeUndefined();
  expect(result.status, result.stdout + result.stderr).toBe(0);
});
