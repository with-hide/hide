import { expect, test } from "@playwright/test";
import { startFixture } from "./engine-fixture";
import { paneShellTabName } from "./pane-fixture";
import { labelAgentPane, startAgentIn } from "./fixture-agents";
import { startHided, type Daemon } from "./hided-fixture";
import { enterWorkspace, screenshot } from "./wire";

test("focused agent titles, inline rename, clear and reconnect", async ({ page }) => {
  test.setTimeout(120_000);
  const fixture = await startFixture({ agents: false });
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(fixture, "tab-names");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    const tab = page.locator(`[data-tab="${fixture.tab}"]`);
    await expect(tab).toBeVisible();
    await expect(tab).toContainText(paneShellTabName(fixture.shell));
    const [first, second] = fixture.panes;
    for (const [pane, title] of [[first, "첫 번째 작업 진행"], [second, "두 번째 작업 진행"]]) {
      await startAgentIn(fixture, daemon, pane!);
      await labelAgentPane(fixture, daemon, pane!, { task: title! });
    }
    await page.locator(`[data-pane-view="${first}"]`).click({ position: { x: 30, y: 60 } });
    await expect(tab).toContainText("첫 번째 작업 진행", { timeout: 15_000 });
    await expect(tab.locator("[data-tab-status]")).toHaveCount(1);
    await expect(tab.locator("img")).toHaveCount(0);
    // The logo stays on the pane's own header and names its product (agent-mark-quiet B4, B6).
    await expect(page.locator(`[data-pane-view="${first}"] header [aria-label="Claude Code"] [data-agent-mark="claude"]`)).toHaveCount(1);
    await page.locator(`[data-pane-view="${second}"]`).click({ position: { x: 30, y: 60 } });
    await expect(tab).toContainText("두 번째 작업 진행");
    await screenshot(page, "tab-names-automatic");
    const rename = async () => {
      await tab.click({ button: "right" });
      const menu = page.getByRole("menu");
      await expect(menu.getByRole("menuitem")).toHaveText([
        "New tab",
        "Move to new group…",
        "Move to group",
        "Rename…",
        /^Split/,
        "Copy name",
        "Close tab…",
      ]);
      await menu.getByRole("menuitem", { name: "Rename…", exact: true }).click();
      const input = page.getByRole("textbox", { name: "Tab name", exact: true });
      await expect(input).toBeFocused();
      return input;
    };
    let input = await rename();
    await expect(input).toHaveValue("두 번째 작업 진행");
    expect(await input.evaluate((node: HTMLInputElement) => [node.selectionStart, node.selectionEnd])).toEqual([0, "두 번째 작업 진행".length]);
    await input.fill("취소할 이름");
    await input.press("Escape");
    await expect(tab).toContainText("두 번째 작업 진행");
    input = await rename();
    await input.fill("고정 이름");
    await input.press("Enter");
    await expect(input).toHaveCount(0);
    await expect(tab).toContainText("고정 이름");
    await page.locator(`[data-pane-view="${first}"]`).click({ position: { x: 30, y: 60 } });
    await expect(tab).toContainText("고정 이름");
    await page.reload();
    await expect(tab).toContainText("고정 이름");
    await screenshot(page, "tab-names-custom");
    input = await rename();
    await input.fill("");
    await input.press("Enter");
    await expect(input).toHaveCount(0);
    await expect(tab).toContainText("첫 번째 작업 진행");
    input = await rename();
    await input.fill("blur 취소");
    await page.locator("[data-workspace-toolbar]").click();
    await expect(input).toHaveCount(0);
    await expect(tab).toContainText("첫 번째 작업 진행");
  } finally {
    daemon?.stop();
    fixture.stop();
  }
});
