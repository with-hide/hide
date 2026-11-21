// The real candidate's Notifications settings, with private HOME/state only.
// Unit tests cannot observe the served translations, host settings flow and reload together.
// Captures are native exact-window captures, never the operator's app.
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { expect, type ElectronApplication } from "@playwright/test";
import { animationsFinished, compositorPresents } from "../../web/e2e/wait";
import { fitWindow, isolate, launchShell, test } from "./fixture";

test("phone rest explains computer use and keeps the switch across a reload in Korean", async () => {
  const run = isolate({}, "phone-rest");
  let app: ElectronApplication | undefined;
  try {
    const launched = await launchShell(run.env);
    app = launched.app;
    const page = launched.page;
    await fitWindow(app, { width: 1100, height: 800 });
    await page.locator("[data-open-settings]").click();
    await page.locator('[data-settings-tab="general"]').click();
    await page.locator("[data-interface-language]").click();
    await page.locator('[data-language-option="ko"]').click();
    for (const theme of ["light", "dark"] as const) {
      await page.locator('[data-settings-tab="general"]').click();
      await page.locator(`[data-theme-option="${theme}"]`).click();
      await expect(page.locator("html")).toHaveClass(new RegExp(`\\b${theme}\\b`));
      await page.locator('[data-settings-tab="notifications"]').click();
      const group = page.locator('[data-settings-group="phone"]');
      await expect(group).toContainText("컴퓨터를 쓰고 있으면 폰으로 알림을 보내지 않아요. 10분 넘게 자리를 비우거나 화면이 잠기면 폰으로 보내요.");
      await expect(group).toContainText("알림을 받을지는 폰 앱의 설정에서 정해요.");
      await expect(group.locator("[data-phone-push-rest]")).toBeChecked();
      const dir = process.env.HIDE_E2E_SCREENSHOT_DIR;
      if (dir && process.platform === "darwin") {
        fs.mkdirSync(dir, { recursive: true });
        await animationsFinished(page);
        await compositorPresents(page);
        const identity = await app.evaluate(({ BrowserWindow }) => {
          const windows = BrowserWindow.getAllWindows();
          if (windows.length !== 1) throw new Error("capture needs exactly one candidate window");
          return { pid: process.pid, executable: process.execPath, source: windows[0]!.getMediaSourceId() };
        });
        execFileSync("/usr/sbin/screencapture", ["-x", "-o", "-l", identity.source.split(":")[1]!, path.join(dir, `phone-rest-ko-${theme}.png`)]);
        fs.writeFileSync(path.join(dir, `phone-rest-ko-${theme}.json`), JSON.stringify({ ...identity, state: run.env.HIDE_STATE_DIR, theme }, null, 2));
      }
    }
    await page.locator("[data-phone-push-rest]").click();
    await expect(page.locator("[data-phone-push-rest]")).not.toBeChecked();
    await page.reload();
    await page.locator("[data-open-settings]").click();
    await page.locator('[data-settings-tab="notifications"]').click();
    await expect(page.locator("[data-phone-push-rest]")).not.toBeChecked();
  } finally {
    await app?.close();
    run.cleanup();
  }
});
