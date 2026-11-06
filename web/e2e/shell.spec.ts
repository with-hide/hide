import { expect, test, type Page } from "@playwright/test";
import path from "node:path";
import { startEngine, startFixture } from "./engine-fixture";
import { startHided } from "./hided-fixture";
import { enterWorkspace } from "./wire";

test("a bad token shows the refused connection state", async ({ page }) => {
  const engine = await startEngine();
  let daemon: Awaited<ReturnType<typeof startHided>> | null = null;
  try {
    daemon = await startHided(engine, "shell-bad-token");
    await page.goto(`${daemon.origin}/#token=${"aa".repeat(32)}`);
    await expect(page.getByText("connection refused - run hide again")).toBeVisible();
  } finally {
    daemon?.stop();
    engine.stop();
  }
});

async function typedTextEchoes(page: Page, marker: string): Promise<void> {
  await page.locator('[data-pane-view][data-focused="true"] .xterm-helper-textarea').focus();
  await page.keyboard.type(marker);
  await expect
    .poll(() => page.evaluate(() => window.__hideProbe?.screenText() ?? ""), { timeout: 10_000 })
    .toContain(marker);
}

// @platform: Real PTY input and echo through the platform's engine and its shell.
test("a sidebar row click switches the pane and typed text echoes there", { tag: "@platform" }, async ({ page }) => {
  const herdr = await startFixture();
  let daemon: Awaited<ReturnType<typeof startHided>> | null = null;
  try {
    daemon = await startHided(herdr, "shell");
    const [first, second] = herdr.panes;
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
    await enterWorkspace(page);
    // The sidebar opens on Projects; the rows this test clicks are the Agents list's.
    await expect(page.locator(`[data-pane="${first}"]`)).toContainText("Agent one");
    await expect(page.locator(`[data-pane="${second}"]`)).toContainText("Agent two");
    await expect(page.locator('[data-pane-view][data-focused="true"]')).toHaveAttribute("data-pane-view", first);

    await page.locator(`[data-pane="${second}"]`).click();
    await expect(page.locator('[data-pane-view][data-focused="true"]')).toHaveAttribute("data-pane-view", second);
    await expect
      .poll(() => page.evaluate(() => window.__hideProbe?.screenText() ?? ""), { timeout: 10_000 })
      .toContain("claude");
    await typedTextEchoes(page, "echo-two-9f3a");

    await page.locator(`[data-pane="${first}"]`).click();
    await expect(page.locator('[data-pane-view][data-focused="true"]')).toHaveAttribute("data-pane-view", first);
    await expect
      .poll(() => page.evaluate(() => window.__hideProbe?.screenText() ?? ""), { timeout: 10_000 })
      .toContain("claude");
    await typedTextEchoes(page, "echo-one-7c1d 한글");
    // Run evidence for a reviewer; CI sets no directory and takes none.
    const screenshots = process.env.HIDE_E2E_SCREENSHOT_DIR;
    if (screenshots) {
      await page.screenshot({ path: path.join(screenshots, "s1-click-flow.png") });
    }
    await expect(page.evaluate(() => window.__hideProbe?.screenText() ?? "")).resolves.not.toContain(
      "echo-two-9f3a",
    );
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});
