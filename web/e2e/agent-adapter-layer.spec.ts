// The approved Basic-chip journey against a private pinned Herdr and kit HOME.
// Expected membership and words are independent of the production declaration.
import { expect, test } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { startHerdr } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { enterWorkspace, screenshot } from "./wire";
import { afterCleanup } from "./worker-owned";

test.skip(process.platform === "win32", "the kit installs POSIX hooks");

test("Basic chips expose three accessible groups in all four languages and return keyboard focus", async ({ page }) => {
  test.setTimeout(180_000);
  const herdr = await startHerdr();
  let daemon: Daemon | undefined;
  try {
    const home = path.join(fs.mkdtempSync(path.join(herdr.root, "ad-")), "home");
    for (const folder of [".claude", ".codex", ".grok", ".local/bin", ".hide/kit"]) fs.mkdirSync(path.join(home, folder), { recursive: true });
    for (const program of ["claude", "codex", "grok", "opencode", "pi", "omp", "agy"]) fs.writeFileSync(path.join(home, ".local/bin", program), "#!/bin/sh\n", { mode: 0o755 });
    fs.writeFileSync(path.join(home, ".hide/kit/installed.json"), JSON.stringify({ format: 1, installed: [] }));
    daemon = await startHided(herdr, "adapter-groups", home, {}, true);
    await page.setViewportSize({ width: 560, height: 1000 });
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    await page.locator("[data-open-settings]").click();
    await page.locator('[data-settings-tab="agents"]').click();
    const list = page.locator(`[data-agents-machine-list="${daemon.node}"]`);
    await expect(list.locator("[data-agent-row]")).toHaveCount(7, { timeout: 60_000 });
    // OpenCode, Pi and omp take letters through Hide's plugin or extension, and Antigravity through Hide's hook
    // block, and are refused a launch, so they are not Basic (PRD opencode-plugin D-11, pi-omp-extension,
    // antigravity-first-class).
    await expect(list.locator("[data-agent-partial]")).toHaveCount(1);
    for (const id of ["claude-code", "codex", "opencode", "pi", "omp", "antigravity"]) await expect(list.locator(`[data-agent-partial="${id}"]`)).toHaveCount(0);
    for (const [language, word, headings] of [
      ["en", "Basic", ["Herdr basics", "Session reading", "Multi-agent collaboration"]],
      ["ko", "기본", ["Herdr로 되는 기본 기능", "세션 읽기", "여러 에이전트 협업"]],
      ["zh-CN", "基础", ["Herdr 基础功能", "会话读取", "多智能体协作"]],
      ["ja", "基本", ["Herdr の基本機能", "セッションの読み取り", "複数エージェントの連携"]],
    ] as const) {
      for (const theme of ["light", "dark"] as const) {
        await page.locator('[data-settings-tab="general"]').click();
        await page.locator("[data-interface-language]").click();
        await page.locator(`[data-language-option="${language}"]`).click();
        await expect(page.locator("html")).toHaveAttribute("lang", language);
        await page.locator(`[data-theme-option="${theme}"]`).click();
        await expect(page.locator("[data-theme-choice]")).toHaveAttribute("data-theme-choice", theme);
        await page.locator('[data-settings-tab="agents"]').click();
        for (const id of ["grok"]) await expect(list.locator(`[data-agent-partial="${id}"]`)).toHaveText(word);
        const chip = list.locator('[data-agent-partial="grok"]');
        await chip.focus();
        await page.keyboard.press("Enter");
        const popover = page.locator('[data-agent-partial-popover="grok"]');
        await expect(popover).toBeVisible();
        await expect(popover.getByRole("heading")).toHaveText([...headings]);
        await expect(popover.locator("[data-agent-feature]")).toHaveCount(11);
        await expect(popover.locator('[data-agent-feature="herdr_integration:yes"]')).toContainText("✓");
        await expect(popover.locator('[data-agent-feature="letters:no"]')).toContainText("–");
        await expect(popover.locator('[data-agent-feature="sleep:yes"]')).toContainText("✓");
        await expect(popover.locator('[data-agent-feature="fork:yes"]')).toContainText("–");
        await expect(popover.locator('[data-agent-feature="titles:yes"]')).toContainText("✓");
        const bounds = await popover.evaluate((node) => {
          const rect = node.getBoundingClientRect();
          return { overflow: node.scrollWidth - node.clientWidth, left: rect.left, right: rect.right, width: innerWidth };
        });
        expect(bounds.overflow).toBeLessThanOrEqual(0);
        expect(bounds.left).toBeGreaterThanOrEqual(0);
        expect(bounds.right).toBeLessThanOrEqual(bounds.width);
        await screenshot(page, `adapter-groups-${language}-${theme}`);
        await page.keyboard.press("Escape");
        await expect(popover).toHaveCount(0);
        await expect(chip).toBeFocused();
        await page.keyboard.press("Space");
        await expect(popover).toBeVisible();
        await page.keyboard.press("Escape");
        await expect(chip).toBeFocused();
      }
    }
  } catch (error) {
    throw afterCleanup(afterCleanup(error, () => daemon?.stop()), () => herdr.stop());
  }
  try { daemon?.stop(); } catch (error) { throw afterCleanup(error, () => herdr.stop()); }
  herdr.stop();
});
