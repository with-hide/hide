import { expect, type ElectronApplication } from "@playwright/test";
import { startHerdr } from "../../web/e2e/herdr-fixture";
import { enterWorkspace } from "../../web/e2e/wire";
import { executeTerminalCheck } from "../../web/e2e/group-start-helper";
import { captureElectronWindow } from "./electron-window";
import { fitWindow, isolate, launch, test } from "./fixture";

// Operator request: New tab immediately opens a usable terminal, without a task form.
test("New tab button and native menu open terminals directly in the selected group", { tag: "@platform" }, async () => {
  const herdr = await startHerdr({ agents: false });
  const run = isolate(herdr, "new-terminal");
  let app: ElectronApplication | null = null;
  try {
    const launched = await launch(run.env);
    app = launched.app;
    const page = launched.page;
    await fitWindow(app, { width: 1440, height: 900 });
    await enterWorkspace(page, "fixture");
    const url = new URL(page.url());
    url.searchParams.set("probe", "1");
    await page.goto(url.toString());
    await enterWorkspace(page, "fixture");
    await expect.poll(() => page.evaluate(() => typeof (window as unknown as { __hideProbe?: unknown }).__hideProbe)).toBe("object");
    const tabs = page.locator('[data-agent-tab-bar] [role=tab]');
    await expect(tabs).toHaveCount(1);
    await page.locator("[data-agent-group-selector]").click();
    await page.getByRole("menuitem", { name: "New group…", exact: true }).click();
    await page.getByRole("textbox", { name: "Group name", exact: true }).fill("터미널 작업");
    await page.getByRole("button", { name: "Save", exact: true }).click();
    await expect(tabs).toHaveCount(0);
    await page.locator("[data-new-agent-tab=true]").click();
    await expect(tabs).toHaveCount(1);
    await expect(page.locator("[data-agent-tab-start]")).toHaveCount(0);
    const first = (await page.locator("[data-pane-view]").getAttribute("data-pane-view"))!;
    expect(herdr.panes).not.toContain(first);
    await executeTerminalCheck(page, herdr, first, 15_000);
    await app.evaluate(({ Menu }) => Menu.getApplicationMenu()!.getMenuItemById("new_tab")!.click());
    await expect(tabs).toHaveCount(2);
    await expect(page.locator("[data-agent-tab-start]")).toHaveCount(0);
    const second = (await page.locator("[data-pane-view]").getAttribute("data-pane-view"))!;
    expect(second).not.toBe(first);
    await executeTerminalCheck(page, herdr, second, 15_000);
    await expect(page.locator("[data-agent-group-selector]")).toContainText("터미널 작업");
    await captureElectronWindow(app, "new-tab-terminal", { first, second, group: "터미널 작업" });
  } finally {
    await app?.close().catch(() => undefined);
    run.cleanup();
    herdr.stop();
  }
});
