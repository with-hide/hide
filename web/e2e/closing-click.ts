import type { Locator } from "@playwright/test";

/**
 * Clicks a control whose handler closes the page it sits on, and returns once
 * the page has closed.
 *
 * The click is Playwright's own pointer click, so a control that is hidden,
 * covered or disabled fails as it would anywhere. Its reply is sent after the
 * handler ran: when the handler's `window.close()` takes the page down first,
 * the click throws "Target page, context or browser has been closed" although
 * the close is the outcome (with-hide/hide-private#325). That one error is the
 * click's success only because the close it names is awaited next; any other
 * error is the test's failure.
 */
export async function clickClosingItsPage(control: Locator): Promise<void> {
  const page = control.page();
  const closed = page.waitForEvent("close");
  // A click that fails leaves this wait behind; its own end is not the test's.
  closed.catch(() => undefined);
  try {
    await control.click();
  } catch (error) {
    if (!(error instanceof Error) || !error.message.includes("Target page, context or browser has been closed")) throw error;
  }
  await closed;
}
