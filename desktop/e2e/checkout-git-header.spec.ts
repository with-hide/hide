import { expect, type ElectronApplication } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { startHerdr } from "../../web/e2e/herdr-fixture";
import { checkoutGitFixture } from "../../web/e2e/checkout-git-fixture";
import { enterWorkspace } from "../../web/e2e/wire";
import { git } from "../../web/e2e/work-prs-fixture";
import { fitWindow, isolate, launchShell, test } from "./fixture";
import { captureNativeWindow } from "./native-window";

test("checkout Git header fits the isolated native candidate at 720 by 480", async () => {
  const herdr = await startHerdr();
  const fixture = checkoutGitFixture(herdr);
  git(fixture.root, ["branch", "-m", "feature/매우-긴-한글과-English-checkout-name"]);
  const run = isolate(herdr, "checkout-header");
  let app: ElectronApplication | undefined;
  try {
    const opened = await launchShell(run.env);
    app = opened.app;
    const page = opened.page;
    await fitWindow(app, { width: 720, height: 480 });
    await enterWorkspace(page, "fixture");
    fs.writeFileSync(path.join(fixture.root, "new.txt"), "new\n");
    await expect(page.locator("[data-checkout-files]")).toHaveText("1", { timeout: 15_000 });
    await expect(page.locator('[data-checkout-ci="failed"]')).toBeVisible({ timeout: 15_000 });
    for (const theme of ["light", "dark"]) {
      await page.getByRole("button", { name: /^Settings \(/ }).click();
      await page.locator('[data-settings-tab="general"]').click();
      await page.locator(`[data-theme-option="${theme}"]`).click();
      const size = page.locator('[data-font-size="true"] [role="slider"]');
      await size.focus();
      await page.keyboard.press("End");
      await expect(size).toHaveAttribute("aria-valuenow", "17");
      await page.keyboard.press("Escape");
      const toolbar = page.locator("[data-workspace-toolbar]");
      expect(await toolbar.evaluate(node => node.scrollWidth > node.clientWidth)).toBe(false);
      // The 32px owning row includes its 1px border; the toolbar fills its content box.
      expect(await page.locator('[data-title-row]').evaluate(node => node.getBoundingClientRect().height)).toBe(32);
      await captureNativeWindow(app, `checkout-header-native-720-${theme}`, { sha: fixture.sha, provider: "private synthetic check-runs and commit statuses", home: run.env.HOME, socket: herdr.socket, state: run.env.HIDE_STATE_DIR });
    }
  } finally {
    await app?.close(); run.cleanup(); herdr.stop();
  }
});
