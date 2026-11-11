// A desktop pause choice is persisted by the private daemon, while receiving
// remains the phone's setting. A component test cannot observe host launch and
// daemon-backed persistence. Capture only this fixture's candidate window.
import { expect, type ElectronApplication } from "@playwright/test";
import { startEngine } from "../../web/e2e/engine-fixture";
import { captureNativeWindow } from "./native-window";
import { animationsFinished, compositorPresents } from "../../web/e2e/wait";
import { connectHome, isolate, launchShell, screenshot, test } from "./fixture";

test("phone pause defaults On and keeps Off after reopening Settings", async () => {
  const engine = await startEngine();
  const run = isolate(engine, "phone-push-settings");
  run.env.HIDE_DESKTOP_SYSTEM_LANGUAGE = "ko-KR";
  let app: ElectronApplication | null = null;
  try {
    await connectHome(run);
    const launched = await launchShell(run.env); app = launched.app;
    const page = launched.page;
    await page.locator("[data-open-settings]").click();
    await page.locator("[data-interface-language]").click();
    await page.locator('[data-language-option="ko"]').click();
    await page.locator('[data-settings-tab="notifications"]').click();
    const rest = page.getByRole("switch", { name: "컴퓨터를 쓰는 동안 폰 알림 쉬기", exact: true });
    await expect(rest).toHaveAttribute("aria-checked", "true");
    await expect(page.getByText("데스크톱 Hide가 열려 있으면 폰으로 알림을 보내지 않아요.", { exact: true })).toBeVisible();
    await expect(page.getByText("알림을 받을지는 폰 앱의 설정에서 정해요.", { exact: true })).toBeVisible();
    const capture = async (name: string) => {
      await animationsFinished(page); await compositorPresents(page);
      await screenshot(page, name);
      await captureNativeWindow(app!, `${name}-native`, { state: run.env.HIDE_STATE_DIR, home: run.env.HOME, daemonPid: run.daemonPid(), build: "worktree debug" });
    };
    await capture("desktop-phone-pause-on");
    await rest.click();
    await expect(rest).toHaveAttribute("aria-checked", "false");
    await capture("desktop-phone-pause-off");
    await page.reload();
    await page.locator("[data-open-settings]").click();
    await page.locator('[data-settings-tab="notifications"]').click();
    await expect(rest).toHaveAttribute("aria-checked", "false");
  } finally {
    await app?.close(); run.cleanup(); engine.stop();
  }
});
