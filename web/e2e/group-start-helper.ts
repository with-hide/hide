import { expect, type Locator, type Page } from "@playwright/test";

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
