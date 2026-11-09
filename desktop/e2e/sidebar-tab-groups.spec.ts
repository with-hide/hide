import { expect, type ElectronApplication } from "@playwright/test";
import { enterWorkspace } from "../../web/e2e/wire";
import { launchShell, startStack, test } from "./fixture";
import { captureElectronWindow } from "./electron-window";
import { captureNativeWindow } from "./native-window";

test("a tab with agents draws its guide and rows in the candidate window", { tag: "@platform" }, async () => {
  // A private desktop host, local stream and state file on each system.
  const stack = await startStack("sidebar-tab-groups");
  const { fixture, run } = stack;
  let app: ElectronApplication | undefined;
  try {
    const launched = await launchShell(run.env);
    app = launched.app;
    const page = launched.page;
    await page.reload();
    await enterWorkspace(page, "fixture");
    const rows = page.locator(`[data-sidebar-tab-group="${fixture.tab}"] [data-agent-open]`);
    // The fixture closes Add project's shell; a tab with agents draws its guide and its rows and no line.
    await expect(rows).toHaveCount(2);
    await expect(page.locator(`[data-sidebar-tab-guide="${fixture.tab}"]`)).toHaveCount(1);
    await expect(page.locator(`[data-sidebar-tab="${fixture.tab}"]`)).toHaveCount(0);
    await captureElectronWindow(app, "sidebar-native-segments", { build: "worktree debug candidate", daemonPid: run.daemonPid() });
    await captureNativeWindow(app, "sidebar-compositor-segments", { method: "screencapture exact candidate window without activation", build: "worktree debug candidate", daemonPid: run.daemonPid(), limits: "native keyboard and IME not exercised" });
    await rows.first().click();
    await expect(page.locator(`[data-tab="${fixture.tab}"]`).first()).toHaveAttribute("aria-selected", "true");
  } finally {
    if (app) await app.close();
    stack.stop();
  }
});
