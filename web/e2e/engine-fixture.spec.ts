// The engine fixture's own contract: its two agents take their first input
// through the daemon, and stopping it removes its root.
// @platform: the fixture starts two native pane processes together.
import { expect, test } from "@playwright/test";
import fs from "node:fs";
import { startFixture } from "./engine-fixture";
import { sendKeys, withControl } from "./daemon-actions";
import { startHided } from "./hided-fixture";

test("the fixture's two agents accept their first input, and stopping it removes its root", { tag: "@platform" }, async () => {
  const fixture = await startFixture();
  const daemon = await startHided(fixture, "fixture-first-input");
  try {
    for (const [index, pane] of fixture.panes.entries()) {
      const marker = `first-input-${index}`;
      await withControl(daemon, async (control) => sendKeys(control, pane, marker));
      await expect.poll(() => fs.existsSync(fixture.inputLogs[index]!) ? fs.readFileSync(fixture.inputLogs[index]!, "utf8") : "").toBe(marker);
    }
  } finally {
    daemon.stop();
    fixture.stop();
  }
  expect(fs.existsSync(fixture.root)).toBe(false);
});
