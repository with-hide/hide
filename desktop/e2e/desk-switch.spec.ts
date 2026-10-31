// Settings' Desk switch and the app menu (PRD session-docs-and-assistant
// D-76, B41): the View menu carries Desk's command and its chord only while
// Desk is on, on the real Electron build against a private Herdr and hided.

import { expect, type ElectronApplication, type Page } from "@playwright/test";
import { startHerdr } from "../../web/e2e/herdr-fixture";
import { enterWorkspace } from "../../web/e2e/wire";
import { isolate, launch, test } from "./fixture";

const deskItem = (app: ElectronApplication) =>
  app.evaluate(({ Menu }) => {
    const item = Menu.getApplicationMenu()?.getMenuItemById("desk_terminal");
    return item ? { accelerator: item.accelerator ?? null } : null;
  });

async function switchDesk(page: Page, on: boolean) {
  await page.locator("[data-open-settings]").click();
  await page.locator('[data-settings-tab="desk"]').click();
  const use = page.locator("[data-desk-use]");
  if ((await use.getAttribute("data-desk-use")) !== String(on)) await use.click();
  await expect(use).toHaveAttribute("data-desk-use", String(on));
  await page.locator("[data-settings-close]").click();
}

test("the View menu has no Desk command while Desk is off and gains it with its chord when Settings turns Desk on", { tag: "@platform" }, async () => {
  const herdr = await startHerdr({ agents: false });
  const run = isolate(herdr, "desk-switch");
  let app: ElectronApplication | null = null;
  try {
    const launched = await launch(run.env);
    app = launched.app;
    const page = launched.page;
    await enterWorkspace(page, "fixture");
    await expect(page.locator("[data-desk-sidebar-row]")).toHaveCount(0);
    expect(await deskItem(app)).toBeNull();

    await switchDesk(page, true);
    await expect(page.locator("[data-desk-sidebar-row]")).toBeVisible();
    await expect.poll(() => deskItem(app!)).toEqual({ accelerator: process.platform === "darwin" ? "Command+J" : "Control+Shift+J" });

    await switchDesk(page, false);
    await expect(page.locator("[data-desk-sidebar-row]")).toHaveCount(0);
    await expect.poll(() => deskItem(app!)).toBeNull();
  } finally {
    await app?.close().catch(() => undefined);
    run.cleanup();
    herdr.stop();
  }
});
