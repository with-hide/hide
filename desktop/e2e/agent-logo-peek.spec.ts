// Holding ⌥ peeks at the agent logos while they are hidden (PRD
// agent-logo-view-mode B13-B17): the logos arrive with the number keycaps on
// the same hold, move nothing, leave with the release, with another key or when
// the window loses focus, and a shown mode keeps its logos through the hold.
// A private engine, hided and Electron app.

import { expect, type ElectronApplication, type Locator, type Page } from "@playwright/test";
import "../../web/src/host";
import { enterWorkspace, screenshot } from "../../web/e2e/wire";
import { hostLog, launchShell, startStack, test, type Stack } from "./fixture";

let stack: Stack;
let app: ElectronApplication | null = null;

test.beforeEach(async () => {
  stack = await startStack("logo-peek");
});
test.afterEach(async () => {
  const info = test.info();
  if (info.status !== info.expectedStatus) console.log(hostLog(stack.run.env).map((line) => JSON.stringify(line)).join("\n"));
  await app?.close().catch(() => undefined);
  app = null;
  stack.stop();
});

/** The boxes of what must not move while the logos peek, rounded to the device pixel. */
async function boxes(locators: Locator[]): Promise<string[]> {
  return Promise.all(locators.map(async (locator) => {
    const box = (await locator.boundingBox())!;
    return [box.x, box.y, box.height].map(Math.round).join(",");
  }));
}

test("⌥ shows the logos with the keycaps while hidden, and a key, a blur or the release ends it", async () => {
  const launched = await launchShell(stack.run.env);
  app = launched.app;
  const page: Page = launched.page;
  await page.reload();
  const cdp = await page.context().newCDPSession(page);
  await cdp.send("Emulation.setFocusEmulationEnabled", { enabled: true });
  await enterWorkspace(page, "fixture");
  await page.locator("[data-pane-view] .xterm-helper-textarea").first().focus();
  const rows = page.locator("nav[data-sidebar] li[data-pane]");
  const logos = page.locator("nav[data-sidebar] li[data-pane] [data-agent-mark], [data-tab] [data-agent-mark]");
  await expect(rows).toHaveCount(2);
  await expect(logos).toHaveCount(0);
  const kept = [rows.first(), rows.first().locator("[data-agent-title]"), rows.nth(1)];
  const before = await boxes(kept);
  await screenshot(page, "logo-peek-hidden");

  // B13, B17: the hold shows a logo on every row and the tab with the keycaps, and moves nothing.
  await page.keyboard.down("Alt");
  await expect(page.locator("nav[data-sidebar] [data-keycap]")).toHaveCount(2);
  await expect(rows.locator("[data-agent-mark]")).toHaveCount(2);
  await expect(page.locator("[data-tab] [data-agent-mark]").first()).toBeVisible();
  await screenshot(page, "logo-peek-held");
  const heights = (await boxes(kept)).map((box) => box.split(",")[2]);
  expect(heights).toEqual(before.map((box) => box.split(",")[2]));
  await page.keyboard.up("Alt");
  await expect(logos).toHaveCount(0);
  await expect(page.locator("[data-keycap]")).toHaveCount(0);
  expect(await boxes(kept)).toEqual(before);

  // B14: another key during the hold ends it, and so does losing the window.
  await page.keyboard.down("Alt");
  await expect(logos.first()).toBeVisible();
  await page.keyboard.press("KeyB");
  await expect(logos).toHaveCount(0);
  await page.keyboard.up("Alt");
  await page.keyboard.down("Alt");
  await expect(logos.first()).toBeVisible();
  await page.evaluate(() => window.dispatchEvent(new Event("blur")));
  await expect(logos).toHaveCount(0);
  await page.keyboard.up("Alt");

  // B2, B16: the menu names the held key for this host.
  await page.locator("[data-view-menu-trigger]").click();
  await expect(page.locator("[data-view-peek]")).toHaveAttribute("data-view-peek", process.platform === "darwin" ? "⌥" : "Alt");
  await page.keyboard.press("Escape");

  // B15: shown keeps its logos through the hold, which adds only the keycaps.
  await page.locator("[data-view-menu-trigger]").click();
  await page.locator('[data-view-item="shown"]').click();
  await expect(rows.locator("[data-agent-mark]")).toHaveCount(2);
  await page.keyboard.down("Alt");
  await expect(page.locator("nav[data-sidebar] [data-keycap]")).toHaveCount(2);
  await expect(rows.locator("[data-agent-mark]")).toHaveCount(2);
  await screenshot(page, "logo-shown-held");
  await page.keyboard.up("Alt");
  await expect(page.locator("[data-keycap]")).toHaveCount(0);
  await expect(rows.locator("[data-agent-mark]")).toHaveCount(2);
});
