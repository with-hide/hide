import { expect, type Locator, type Page } from "@playwright/test";
import { execFileSync } from "node:child_process";
import path from "node:path";
import type { HerdrFixture } from "./herdr-fixture";
import { fixtureProgram } from "./platform-fixture";

/** Proves typed input executes in the node's normal shell, independently of its prompt. */
export async function executeTerminalCheck(page: Page, herdr: HerdrFixture, pane: string, timeout = 5_000): Promise<void> {
  const marker = "HIDE_TERMINAL_EXECUTED";
  fixtureProgram(path.join(herdr.root, "bin"), "hide-terminal-check", `console.log(${JSON.stringify(marker)});`);
  const view = page.locator(`[data-pane-view="${pane}"]`);
  await expect(view).toHaveAttribute("data-transport", "controlling");
  await view.locator("textarea").focus();
  await page.keyboard.type("hide-terminal-check");
  await page.keyboard.press("Enter");
  await expect.poll(() => page.evaluate((id) => window.__hideProbe?.paneText(id), pane), { timeout }).toContain(marker);
  await expect.poll(() => execFileSync(herdr.bin, ["pane", "read", pane, "--source", "visible", "--format", "text"], { env: herdr.env, encoding: "utf8", timeout: 10_000 }), { timeout }).toContain(marker);
}

/** Explicitly starts a terminal in the selected draft and keeps its strip slot. */
export async function startDraftTerminal(page: Page | Locator, expectedDraft?: string): Promise<string> {
  const form = page.locator("[data-agent-tab-start]");
  await expect(form).toHaveAttribute("data-agent-tab-start", expectedDraft ?? /^draft:/);
  const draft = (await form.getAttribute("data-agent-tab-start"))!;
  const tabs = page.locator("[data-agent-tab-bar] [role=tab][data-tab]");
  await expect(page.locator(`[data-agent-tab-bar] [role=tab][data-tab="${draft}"]`)).toHaveAttribute("aria-selected", "true");
  const before = await tabs.evaluateAll((nodes) => nodes.map((node) => node.getAttribute("data-tab")!));
  const slot = before.indexOf(draft);
  expect(slot).toBeGreaterThanOrEqual(0);
  await form.locator("[data-group-start-terminal=true]").click();
  await expect(page.locator("[data-agent-tab-start]")).toHaveCount(0, { timeout: 30_000 });
  await expect(tabs).toHaveCount(before.length);
  await expect(page.locator("[data-canvas]")).toBeVisible();
  const tab = (await page.locator("[data-canvas]").getAttribute("data-canvas"))!;
  expect(tab).not.toMatch(/^draft:/);
  const after = [...before];
  after[slot] = tab;
  await expect.poll(() => tabs.evaluateAll((nodes) => nodes.map((node) => node.getAttribute("data-tab")!))).toEqual(after);
  await expect(tabs.nth(slot)).toHaveAttribute("aria-selected", "true");
  return tab;
}
