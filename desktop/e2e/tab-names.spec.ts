import { expect, type ElectronApplication } from "@playwright/test";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { labelAgentPane } from "../../web/e2e/fixture-agents";
import { enterWorkspace } from "../../web/e2e/wire";
import { launchShell, startStack, test } from "./fixture";
import { animationsFinished, compositorPresents } from "../../web/e2e/wait";

test("native tab composition and Korean inline naming", async () => {
  const stack = await startStack("tab-names");
  const { fixture, run, daemon } = stack;
  let app: ElectronApplication | undefined;
  try {
    const title = "한글 작업 이름 · focused session";
    await labelAgentPane(fixture, daemon, fixture.panes[0], { task: title });
    const launched = await launchShell(run.env);
    app = launched.app;
    const page = launched.page;
    const cdp = await page.context().newCDPSession(page);
    await cdp.send("Emulation.setFocusEmulationEnabled", { enabled: true });
    await expect(page.locator("[data-main-screen], [data-workspace-screen]")).toBeVisible({ timeout: 30_000 });
    await enterWorkspace(page, "fixture");
    await page.locator(`[data-pane-view="${fixture.panes[0]}"]`).click({ position: { x: 30, y: 60 } });
    const tab = page.locator(`[data-tab="${fixture.tab}"]`);
    await expect(tab).toContainText(title);
    await tab.click({ button: "right" });
    await page.getByRole("menuitem", { name: "Copy name", exact: true }).click();
    await expect.poll(async () => (await app!.evaluate(({ clipboard }) => clipboard.readText())) === title).toBe(true);
    await tab.click({ button: "right" });
    await page.getByRole("menuitem", { name: "Rename…", exact: true }).click();
    const input = page.getByRole("textbox", { name: "Tab name", exact: true });
    await expect(input).toBeFocused();
    await input.fill("검토할 탭 이름");
    await input.dispatchEvent("keydown", { key: "Enter", code: "Enter", isComposing: true });
    await expect(input).toBeVisible();
    await input.press("Enter");
    await expect(input).toHaveCount(0);
    await expect(tab).toContainText("검토할 탭 이름");
    const candidate = await app.evaluate(({ BrowserWindow }) => {
      const windows = BrowserWindow.getAllWindows();
      if (windows.length !== 1) throw new Error("Expected exactly one candidate window");
      return { source: windows[0]!.getMediaSourceId(), pid: process.pid };
    });
    const dir = process.env.HIDE_E2E_SCREENSHOT_DIR;
    if (dir) {
      fs.mkdirSync(dir, { recursive: true });
      execFileSync("/usr/sbin/screencapture", ["-x", "-o", "-l", candidate.source.split(":")[1]!, path.join(dir, "tab-names-native.png")]);
      fs.writeFileSync(path.join(dir, "tab-names-native.json"), JSON.stringify({ ...candidate, build: "worktree desktop/dist", isolated: true, occludedPainting: true }));
      for (const theme of ["light", "dark"] as const) {
        await page.locator("[data-open-settings]").click();
        await expect(page.locator('[data-settings="true"]')).toBeVisible();
        await page.locator('[data-settings-tab="general"]').click();
        await page.locator(`[data-theme-option="${theme}"]`).click();
        await expect(page.locator("html")).toHaveClass(new RegExp(`\\b${theme}\\b`));
        await page.keyboard.press("Escape");
        await expect(page.locator('[data-settings="true"]')).toHaveCount(0);
        await expect(tab).toContainText("검토할 탭 이름");
        // Let the existing theme transitions finish before the native capture.
        await animationsFinished(page);
        await compositorPresents(page);
        execFileSync("/usr/sbin/screencapture", ["-x", "-o", "-l", candidate.source.split(":")[1]!, path.join(dir, `tab-names-native-${theme}.png`)]);
      }
    }
  } finally {
    await app?.close();
    stack.stop();
  }
});
