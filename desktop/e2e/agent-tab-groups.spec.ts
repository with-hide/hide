import { expect, type ElectronApplication } from "@playwright/test";
import { focusTab, newTab, withControl } from "../../web/e2e/daemon-actions";
import { enterWorkspace } from "../../web/e2e/wire";
import { fitWindow, launchShell, screenshot, startStack, test } from "./fixture";
import { withProbe, type Probe } from "./terminal-probe";

test("Agent edge drag splits the desktop column into two live tab groups", async () => {
  const stack = await startStack("agent-groups", { agents: false });
  const { fixture, run, daemon } = stack;
  let app: ElectronApplication | null = null;
  try {
    // The tab is made without being shown, as the first tab stays the one in front.
    const created = await withControl(daemon, async (control) => {
      const made = await newTab(control, fixture, "second");
      await focusTab(control, fixture, fixture.tab);
      return made;
    });
    const launched = await launchShell(run.env);
    app = launched.app;
    const page = launched.page;
    await fitWindow(app, { width: 1024, height: 1080 });
    await withProbe(page);
    await enterWorkspace(page, "fixture");
    const tab = page.locator(`[data-agent-tab-bar] [data-tab="${created.tab}"]`);
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
    await expect(page.locator(`[data-canvas="${fixture.tab}"]`)).toBeVisible();
    await expect(page.locator(`[data-canvas="${created.tab}"]`)).toBeVisible();
    const screen = (pane: string) => page.evaluate((id) => (window as unknown as { __hideProbe: Probe }).__hideProbe.paneText(id), pane);
    const groups = [fixture.panes[0], created.pane];
    for (const pane of groups) {
      const terminal = page.locator(`[data-terminal-host="${pane}"]`);
      await expect(terminal).toBeVisible();
      await terminal.click();
      // The first group's split pane is narrow; keep the output on one row.
      await page.keyboard.type("printf 'group-live\\n'");
      await page.keyboard.press("Enter");
      await expect.poll(() => screen(pane)).toMatch(/(?:^|\n)group-live\r?(?:\n|$)/);
    }
    // Each group ran the line exactly once: Herdr's late answer to the drop's
    // focus once moved the keyboard to the other group mid-line (#413).
    for (const pane of groups) {
      expect((await screen(pane)).match(/(?:^|\n)group-live\r?(?=\n|$)/g) ?? [], `${pane} output`).toHaveLength(1);
    }
    await expect(page.locator('[data-transport="released"]')).toHaveCount(0);
    await screenshot(page, "desktop-agent-groups-split");
  } finally {
    await app?.close().catch(() => undefined);
    stack.stop();
  }
});
