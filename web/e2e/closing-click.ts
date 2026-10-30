import { expect, type Locator } from "@playwright/test";

/**
 * Clicks a control whose handler closes the page it sits on, and returns once
 * the page has closed.
 *
 * Playwright's own click waits for the page to answer the pointer release,
 * and that answer is sent after the handler ran: when the handler's
 * `window.close()` takes the page down first, the click throws "Target page,
 * context or browser has been closed" although the close is the outcome
 * (with-hide/hide-private#325). Here the page answers the evaluate before the
 * click runs on its next task, so nothing the test sent is waiting on the page
 * when it closes, and the close itself is what the test waits for.
 */
export async function clickClosingItsPage(control: Locator): Promise<void> {
  const page = control.page();
  await expect(control).toBeVisible();
  const closed = page.waitForEvent("close");
  await control.evaluate((element: HTMLElement) => {
    setTimeout(() => element.click(), 0);
  });
  await closed;
}
