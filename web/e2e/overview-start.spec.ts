// Issue 2514: a project's `+` on the Overview's Work map (beside its name, expanded or folded)
// and in the side column opens the start panel on that project, and the start keeps the operator
// in the Overview, where the new session joins the map. The name stays the project's drag handle.
// End to end because the unit tests cannot see what decides it here: the Overview layer's modal keeps
// the pointer and keyboard to itself, and the started agent joins the map only on the core's answer.
import { expect, test } from "@playwright/test";
import { startFixture } from "./engine-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { openOverview } from "./overview-entry";
import { enterWorkspace, screenshot } from "./wire";

test.describe.configure({ timeout: 150_000 });

test("an Overview project's + opens the start panel on that project and the start stays in the Overview", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const engine = await startFixture();
  let daemon: Daemon | undefined;
  try {
    daemon = await startHided(engine, "overview-start");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    await openOverview(page, "work");
    const graph = page.locator("[data-overview-graph]");
    await expect(graph).toHaveAttribute("data-graph-phase", "ready", { timeout: 30_000 }); // wait-ok: the first layout of the map
    const panel = page.locator("[data-start-panel]");
    const target = new RegExp(`^checkout:${daemon.node}:.*/fixture$`);

    // The map: the `+` beside the project's name, not the name itself, opens the panel on the project.
    const project = graph.locator("[data-graph-project]").first();
    await project.locator(".og-project-name").hover();
    await screenshot(page, "overview-start-name-hover");
    await project.locator("[data-graph-project-start]").click();
    await expect(panel).toBeVisible();
    await expect(page.locator("[data-start-text]")).toBeFocused();
    await expect(panel.locator("[data-start-target]")).toHaveAttribute("data-start-target", target);
    await screenshot(page, "overview-start-panel");
    await page.keyboard.press("Escape");
    await expect(panel).toHaveCount(0);
    await expect(page.locator("[data-overview-page]")).toBeVisible();

    // The side column: the row's `+` opens it on the same project, and the start stays in the Overview.
    const row = graph.locator("[data-graph-project-row]").first();
    await row.hover();
    await screenshot(page, "overview-start-row-hover");
    await row.locator("[data-graph-project-row-start]").click();
    await expect(panel.locator("[data-start-target]")).toHaveAttribute("data-start-target", target);
    const sessions = graph.locator("[data-graph-session]");
    const before = await sessions.count();
    await page.locator("[data-start-text]").fill("개요에서 시작");
    await page.keyboard.press("Enter");
    await expect(panel).toHaveCount(0, { timeout: 30_000 }); // wait-ok: the core's answer to the start
    await expect(sessions).toHaveCount(before + 1, { timeout: 30_000 }); // wait-ok: the started agent joins the map
    await expect(page.locator("[data-overview-page]")).toBeVisible();
    await expect(page.locator('[data-main-view="work"]')).toBeVisible();
    await screenshot(page, "overview-start-after");

    // Folded, the project's circle keeps the `+` beside its name.
    await project.locator(".og-project-name").hover();
    await project.locator(".og-project-fold").click();
    const cluster = graph.locator("[data-graph-cluster]").first();
    await expect(cluster).toBeVisible();
    await cluster.locator("strong").hover();
    await screenshot(page, "overview-start-folded");
    await cluster.locator("[data-graph-project-start]").click();
    await expect(panel.locator("[data-start-target]")).toHaveAttribute("data-start-target", target);
  } finally { daemon?.stop(); engine.stop(); }
});
