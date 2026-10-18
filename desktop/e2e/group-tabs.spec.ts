// Candidate-window evidence for group-tabs. Renderer input is addressed
// through Playwright; captures identify this app's sole exact native window.
import { expect, type ElectronApplication } from "@playwright/test";
import { startHerdr } from "../../web/e2e/herdr-fixture";
import { enterWorkspace } from "../../web/e2e/wire";
import { chord } from "../../web/e2e/chords";
import { compositorPresents } from "../../web/e2e/wait";
import { fitWindow, isolate, launchShell, test } from "./fixture";
import { captureNativeWindow } from "./native-window";

test("named groups and the independent terminal action render in the candidate window", async () => {
  const herdr = await startHerdr();
  const run = isolate(herdr, "group-tabs");
  let app: ElectronApplication | undefined;
  try {
    const opened = await launchShell(run.env);
    app = opened.app;
    const page = opened.page;
    await fitWindow(app, { width: 1440, height: 900 });
    await enterWorkspace(page, "fixture");
    const selector = page.locator("[data-agent-group-selector]");
    await expect(selector).toBeVisible();
    const facts = { build: "worktree debug", daemonPid: run.daemonPid(), home: run.env.HOME, state: run.env.HIDE_STATE_DIR, socket: herdr.socket, input: "Playwright renderer", externalProvider: "controlled fixture CLI" };
    await compositorPresents(page);
    await captureNativeWindow(app, "group-tabs-work-native", facts);
    await selector.click();
    await page.getByRole("menuitem", { name: "New group…", exact: true }).click();
    await page.getByRole("textbox", { name: "Group name", exact: true }).fill("문서 검토와 다음 작업");
    await page.getByRole("button", { name: "Save", exact: true }).click();
    await expect(page.getByRole("dialog")).toHaveCount(0);
    const group = await selector.getAttribute("data-agent-group-selector");
    const row = page.locator(`[data-sidebar-group-select="${group}"]`);
    if (!(await row.isVisible())) await page.locator("[data-checkout-toggle][aria-expanded=false]").click();
    await expect(row).toBeVisible();
    await page.locator("[data-new-agent-tab=true]").click();
    await expect(page.locator("[data-agent-tab-start]")).toHaveAttribute("data-agent-tab-start", /^draft:/);
    await expect(row).toHaveAttribute("aria-current", "true");
    await expect(page.locator('[data-agent-open][aria-current="true"]')).toHaveCount(0);
    await expect(page.locator("[data-group-start-terminal=true]")).toBeEnabled();
    await expect(page.locator("[data-new-agent-tab=true]")).toBeVisible();
    await compositorPresents(page);
    await captureNativeWindow(app, "group-tabs-blank-native", facts);
    await page.keyboard.press(chord("settings"));
    await page.locator('[data-settings-tab="general"]').click();
    await page.locator('[data-theme-option="light"]').click();
    await expect(page.locator("html")).toHaveClass(/(^|\s)light(\s|$)/);
    await page.locator("[data-interface-language]").click();
    await page.locator('[data-language-option="ko"]').click();
    await expect(page.locator("html")).toHaveAttribute("lang", "ko");
    await page.keyboard.press("Escape");
    await fitWindow(app, { width: 920, height: 760 });
    await expect(page.locator("[data-group-start-terminal=true]")).toBeVisible();
    await expect(selector).toContainText("문서 검토와 다음 작업");
    await compositorPresents(page);
    await captureNativeWindow(app, "group-tabs-narrow-light-native", { ...facts, requestedWidth: 920 });
  } finally {
    try { await app?.close(); }
    finally { try { run.cleanup(); } finally { herdr.stop(); } }
  }
});
