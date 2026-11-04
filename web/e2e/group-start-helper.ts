import { expect, type Locator, type Page } from "@playwright/test";
import path from "node:path";
import type { EngineServer } from "./engine-fixture";
import type { Daemon } from "./hided-fixture";
import { sendEvent } from "./wire";
import { paneCommandLine, type HerdrFixture } from "./herdr-fixture";
import { fixtureProgram } from "./platform-fixture";
import { herdrTabs } from "./failure-dump";

/**
 * Proves typed input executes in the pane's shell, independently of its
 * prompt: the fixture program the shell is told to run prints a marker,
 * which the page's probe reads back. It reads no engine's own CLI, so it
 * holds on either engine. The node's normal shell need not inherit the
 * server's PATH, so the command names the fixture program by path, in the
 * fixture shell's syntax.
 */
export async function executeTerminalCheck(page: Page, fixture: Pick<EngineServer, "root" | "shell">, pane: string, timeout = 5_000): Promise<void> {
  const marker = "HIDE_TERMINAL_EXECUTED";
  const executable = fixtureProgram(path.join(fixture.root, "bin"), "hide-terminal-check", `console.log(${JSON.stringify(marker)});`);
  const view = page.locator(`[data-pane-view="${pane}"]`);
  await expect(view).toHaveAttribute("data-transport", "controlling");
  await view.locator("textarea").focus();
  await page.keyboard.type(paneCommandLine(fixture.shell, [executable]));
  await page.keyboard.press("Enter");
  await expect.poll(() => page.evaluate((id) => (window as unknown as { __hideProbe?: { paneText: (id: string) => string } }).__hideProbe?.paneText(id), pane), { timeout }).toContain(marker);
}

/** Every tab id the fixture's Herdr holds now, across its workspaces. */
export function herdrTabIds(herdr: HerdrFixture): string[] {
  return herdrTabs(herdr).flatMap((workspace) => workspace.tabs);
}

/** The `count` tabs Herdr created since `before` was read, in Herdr's order, once it holds them all. */
export async function createdTabs(herdr: HerdrFixture, before: readonly string[], count = 1): Promise<string[]> {
  const created = () => herdrTabIds(herdr).filter((id) => !before.includes(id));
  await expect.poll(() => created().length, { timeout: 20_000 }).toBe(count);
  return created();
}

/**
 * Waits for the page to select and draw `tab`, a tab Herdr created
 * (`createdTabs`). The page's own selection is never the expectation: the
 * canvas can draw the new tab while the strip still selects the one before.
 */
export async function waitForTerminal(page: Page | Locator, tab: string): Promise<void> {
  await expect(page.locator("[data-agent-tab-start]")).toHaveCount(0);
  await expect(page.locator('[data-agent-tab-bar] [role=tab][aria-selected=true]').last()).toHaveAttribute("data-tab", tab);
  await expect(page.locator("[data-canvas]").last()).toHaveAttribute("data-canvas", tab);
  await expect(page.locator("[data-pane-view]").last()).toBeVisible();
}

/** Compatibility fixture: an older client created a saved, unstarted draft. */
export async function createLegacyDraft(page: Page, daemon: Daemon, checkoutPath: string): Promise<void> {
  const group = await page.locator("[data-agent-group-selector]").getAttribute("data-agent-group-selector");
  await sendEvent(page, daemon, "agent_group", {
    workspace: { device_id: daemon.node, path: checkoutPath },
    group_id: group === "ungrouped" ? null : group, action: "new_tab", request_id: `legacy-${Date.now()}`,
  });
  await expect(page.locator("[data-agent-tab-start]")).toHaveAttribute("data-agent-tab-start", /^draft:/);
}
