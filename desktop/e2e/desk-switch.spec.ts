// Settings' Desk switch and the app menu (PRD session-docs-and-assistant
// D-76, B41): Desk's row follows its switch, while Conversation ⇄ terminal
// stays available (pane-conversation-toggle B10), on a private native stack.

import { expect, type ElectronApplication, type Page } from "@playwright/test";
import { enterWorkspace } from "../../web/e2e/wire";
import { launchShell, startStack, test } from "./fixture";

const deskItem = (app: ElectronApplication) =>
  app.evaluate(({ Menu }) => {
    const item = Menu.getApplicationMenu()?.getMenuItemById("desk_terminal");
    return item ? { accelerator: item.accelerator ?? null } : null;
  });

async function switchDesk(page: Page, on: boolean) {
  await page.locator("[data-open-settings]").click();
  await page.locator('[data-settings-tab="labs"]').click();
  const use = page.locator("[data-desk-use]");
  if ((await use.getAttribute("data-desk-use")) !== String(on)) await use.click();
  await expect(use).toHaveAttribute("data-desk-use", String(on));
  await page.locator("[data-settings-close]").click();
}

test("the View menu keeps Conversation ⇄ terminal and its chord while Settings switches Desk on and off", { tag: "@platform" }, async () => {
  const stack = await startStack("desk-switch", { agents: false });
  let app: ElectronApplication | null = null;
  try {
    const launched = await launchShell(stack.run.env);
    app = launched.app;
    const page = launched.page;
    await enterWorkspace(page, "fixture");
    await expect(page.locator("[data-desk-sidebar-row]")).toHaveCount(0);
    const command = { accelerator: process.platform === "darwin" ? "Command+J" : "Control+Shift+J" };
    expect(await deskItem(app)).toEqual(command);

    await switchDesk(page, true);
    await expect(page.locator("[data-desk-sidebar-row]")).toBeVisible();
    await expect.poll(() => deskItem(app!)).toEqual(command);

    await switchDesk(page, false);
    await expect(page.locator("[data-desk-sidebar-row]")).toHaveCount(0);
    await expect.poll(() => deskItem(app!)).toEqual(command);
  } finally {
    await app?.close().catch(() => undefined);
    stack.stop();
  }
});
