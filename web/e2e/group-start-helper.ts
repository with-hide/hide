import { expect, type Locator, type Page } from "@playwright/test";
import { execFileSync } from "node:child_process";
import path from "node:path";
import type { Daemon } from "./hided-fixture";
import { sendEvent } from "./wire";
import { paneCommandLine, type HerdrFixture } from "./herdr-fixture";
import { fixtureProgram } from "./platform-fixture";

/** Proves typed input executes in the node's normal shell, independently of its prompt. */
export async function executeTerminalCheck(page: Page, herdr: HerdrFixture, pane: string, timeout = 5_000): Promise<void> {
  const marker = "HIDE_TERMINAL_EXECUTED";
  const executable = fixtureProgram(path.join(herdr.root, "bin"), "hide-terminal-check", `console.log(${JSON.stringify(marker)});`);
  const view = page.locator(`[data-pane-view="${pane}"]`);
  await expect(view).toHaveAttribute("data-transport", "controlling");
  await view.locator("textarea").focus();
  // The node's normal shell need not inherit the server's injected PATH.
  // Run the exact fixture executable using the fixture's selected shell.
  const command = paneCommandLine(herdr.shell, [executable]);
  await page.keyboard.type(command);
  await page.keyboard.press("Enter");
  await expect.poll(() => page.evaluate((id) => (window as unknown as { __hideProbe?: { paneText: (id: string) => string } }).__hideProbe?.paneText(id), pane), { timeout }).toContain(marker);
  await expect.poll(() => execFileSync(herdr.bin, ["pane", "read", pane, "--source", "visible", "--format", "text"], { env: herdr.env, encoding: "utf8", timeout: 10_000 }), { timeout }).toContain(marker);
}

/** Observes a newly created terminal after the caller has awaited its tab. */
export async function waitForTerminal(page: Page | Locator, expectedTab?: string): Promise<string> {
  await expect(page.locator("[data-agent-tab-start]")).toHaveCount(0);
  const selected = page.locator('[data-agent-tab-bar] [role=tab][aria-selected=true]').last();
  await expect(selected).not.toHaveAttribute("data-tab", /^draft:/);
  if (expectedTab) await expect(selected).toHaveAttribute("data-tab", expectedTab);
  const tab = (await selected.getAttribute("data-tab"))!;
  await expect(page.locator("[data-canvas]").last()).toHaveAttribute("data-canvas", tab);
  await expect(page.locator("[data-pane-view]").last()).toBeVisible();
  return tab;
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
