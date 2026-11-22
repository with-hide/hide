// The device Overview as the operator reaches it (PRD overview-device-tasks):
// from a Workspace the sidebar's Overview row opens it as a layer over the
// Workspace; with no Workspace in front it is the page itself. Modal
// lifetime, focus and terminal preservation belong to overview-modal.spec.
import { expect, type Page } from "@playwright/test";

export type OverviewTab = "work" | "projects" | "backlog" | "history" | "usage";

/** The Overview on one of its tabs, opened from the sidebar's Overview row when it is not on screen yet. */
export async function openOverview(page: Page, tab: OverviewTab = "backlog") {
  const overview = page.locator("[data-overview-page]");
  if ((await overview.count()) === 0) await page.locator("[data-sidebar-overview]").click();
  await expect(overview).toBeVisible();
  await page.locator(`[data-main-tab="${tab}"]`).click();
  await expect(page.locator(`[data-main-view="${tab}"]`)).toBeVisible();
}

/** The Overview's Projects tab row of the project labelled `label`. */
export async function projectRow(page: Page, label: string) {
  await openOverview(page, "projects");
  const row = page.locator("[data-projects-row]", { hasText: new RegExp(`^${label}`) });
  // A resting project with nothing to clean up is folded under `Quiet projects N` (B50).
  const quiet = page.locator('[data-projects-quiet][aria-expanded="false"]');
  await expect(row.or(quiet).first()).toBeVisible({ timeout: 20_000 });
  if (!(await row.isVisible())) {
    // The project can leave the quiet group by itself while this looks (a read
    // it waited on lands, such as a merged pull request): the toggle then goes
    // and the row shows without it. The one click races that, so the helper
    // never waits on a toggle that is gone; a click still pending afterwards can
    // only open a quiet fold, which hides no row, and its end is not awaited.
    const opened = quiet.click();
    opened.catch(() => undefined);
    await Promise.race([opened, row.waitFor({ state: "visible", timeout: 20_000 })]);
  }
  await expect(row).toBeVisible({ timeout: 20_000 });
  return row;
}
