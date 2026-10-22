import { expect, type ElectronApplication } from "@playwright/test";
import { execFileSync } from "node:child_process";
import path from "node:path";
import { startHerdr } from "../../web/e2e/herdr-fixture";
import { enterWorkspace } from "../../web/e2e/wire";
import { fitWindow, isolate, launch, screenshot, test } from "./fixture";

test("Agent edge drag splits the desktop column into two live tab groups", async () => {
  const herdr = await startHerdr({ agents: false });
  const run = isolate(herdr, "agent-groups");
  let app: ElectronApplication | null = null;
  try {
    const created = herdr.run(["tab", "create", "--workspace", herdr.workspace, "--cwd", path.join(herdr.root, "fixture"), "--no-focus"]) as { result: { tab: { tab_id: string }; root_pane: { pane_id: string } } };
    const launched = await launch(run.env);
    app = launched.app;
    const page = launched.page;
    await fitWindow(app, { width: 1024, height: 1080 });
    await enterWorkspace(page, "fixture");
    const tab = page.locator(`[data-agent-tab-bar] [data-tab="${created.result.tab.tab_id}"]`);
    await expect(tab).toBeVisible();
    const start = await tab.boundingBox();
    const body = await page.locator("[data-agent-body]").boundingBox();
    if (!start || !body) throw new Error("Agent drag target is missing");
    await page.mouse.move(start.x + start.width / 2, start.y + start.height / 2);
    await page.mouse.down();
    await page.mouse.move(start.x + start.width / 2 + 12, start.y + start.height / 2, { steps: 3 });
    await page.mouse.move(body.x + body.width * .92, body.y + body.height / 2, { steps: 12 });
    await expect(page.locator('[data-agent-drop="right"]')).toHaveText("Split right");
    await page.mouse.up();
    await expect(page.locator("[data-agent-area-id]")).toHaveCount(2);
    await expect(page.locator("[data-agent-tab-bar]")).toHaveCount(2);
    await expect(page.locator(`[data-canvas="${herdr.tab}"]`)).toBeVisible();
    await expect(page.locator(`[data-canvas="${created.result.tab.tab_id}"]`)).toBeVisible();
    const groups = [herdr.panes[0], created.result.root_pane.pane_id];
    for (const pane of groups) {
      const terminal = page.locator(`[data-terminal-host="${pane}"]`);
      await expect(terminal).toBeVisible();
      await terminal.click();
      await page.keyboard.type("printf 'desktop-group-live\\n'");
      await page.keyboard.press("Enter");
      await expect.poll(() => execFileSync(herdr.bin, ["pane", "read", pane, "--source", "visible", "--format", "text"], { env: herdr.env, encoding: "utf8", timeout: 10_000 })).toMatch(/(?:^|\n)desktop-group-live\r?(?:\n|$)/);
    }
    // Each group ran the line exactly once: Herdr's late answer to the drop's
    // focus once moved the keyboard to the other group mid-line (#413).
    for (const pane of groups) {
      const history = execFileSync(herdr.bin, ["pane", "read", pane, "--source", "recent-unwrapped", "--lines", "200", "--format", "text"], { env: herdr.env, encoding: "utf8", timeout: 10_000 });
      expect(history.match(/(?:^|\n)desktop-group-live\r?(?=\n|$)/g) ?? [], `${pane} output`).toHaveLength(1);
    }
    await expect(page.locator('[data-transport="released"]')).toHaveCount(0);
    await screenshot(page, "desktop-agent-groups-split");
  } finally {
    await app?.close().catch(() => undefined);
    run.cleanup();
    herdr.stop();
  }
});
