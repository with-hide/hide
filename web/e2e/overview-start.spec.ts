// Issue 2514: the Overview's Work map starts new work where it is drawn. Each unfolded project has a
// dashed circle where its next session would stand, a folded project a `+` beside its name, each side
// column row a `+`; all open the start panel on that project. You's `+` opens it on no project, and the
// map's Add project place opens Add project. A start keeps the operator in the Overview, where the new
// session joins the map, and the project's name stays its drag handle.
// End to end because the unit tests cannot see what decides it here: the force map's real placement,
// the Overview layer's modal keeping the pointer and keyboard to itself, and the core's answer to a start.
import { expect, test } from "@playwright/test";
import { startFixture } from "./engine-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { openOverview } from "./overview-entry";
import { enterWorkspace, screenshot } from "./wire";

test.describe.configure({ timeout: 150_000 });

test("the Overview's work map opens the start panel from a project's start circle and row, and the start stays in the Overview", async ({ page }) => {
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
    await screenshot(page, "overview-start-map");

    // The project's start circle, not its name, opens the panel on the project.
    const slot = graph.locator("[data-graph-start-slot]");
    await expect(slot).toHaveCount(1);
    await expect(graph.locator("[data-graph-project] button[aria-label^='Start agent in']")).toHaveCount(0);
    await slot.hover();
    await screenshot(page, "overview-start-slot-hover");
    await slot.click();
    await expect(page.locator("[data-start-text]")).toBeFocused();
    await expect(panel.locator("[data-start-target]")).toHaveAttribute("data-start-target", target);
    await screenshot(page, "overview-start-panel");
    await page.keyboard.press("Escape");
    await expect(panel).toHaveCount(0);
    await expect(page.locator("[data-overview-page]")).toBeVisible();

    // The side column's `+` opens it on the same project, and the start stays in the Overview.
    const row = graph.locator("[data-graph-project-row]").first();
    await row.hover();
    await row.locator("[data-graph-project-row-start]").click();
    await expect(panel.locator("[data-start-target]")).toHaveAttribute("data-start-target", target);
    const sessions = graph.locator("[data-graph-session]");
    const before = await sessions.count();
    await page.locator("[data-start-text]").fill("개요에서 시작");
    await page.keyboard.press("Enter");
    await expect(panel).toHaveCount(0, { timeout: 30_000 }); // wait-ok: the core's answer to the start
    await expect(sessions).toHaveCount(before + 1, { timeout: 30_000 }); // wait-ok: the started agent joins the map
    await expect(page.locator('[data-main-view="work"]')).toBeVisible();
    await expect(slot).toHaveCount(1);
    await screenshot(page, "overview-start-after");
  } finally { daemon?.stop(); engine.stop(); }
});

test("the Overview's work map opens the start panel on no project from You, Add project from its place, and on a folded project from its name", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const engine = await startFixture();
  let daemon: Daemon | undefined;
  try {
    daemon = await startHided(engine, "overview-start-you");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    await openOverview(page, "work");
    const graph = page.locator("[data-overview-graph]");
    await expect(graph).toHaveAttribute("data-graph-phase", "ready", { timeout: 30_000 }); // wait-ok: the first layout of the map
    const panel = page.locator("[data-start-panel]");

    // You's `+` opens the panel with the target it picks by itself, as ⌘K's Start an agent does.
    await graph.locator("[data-graph-operator-start]").click();
    await expect(page.locator("[data-start-text]")).toBeFocused();
    await screenshot(page, "overview-start-you");
    await page.keyboard.press("Escape");
    await expect(panel).toHaveCount(0);

    // The Add project place opens the sidebar's Add project dialog over the Overview.
    await graph.locator("[data-graph-add-project]").click();
    await expect(page.locator("[data-add-project]")).toBeVisible();
    await screenshot(page, "overview-start-add-project");
    await page.keyboard.press("Escape");
    await expect(page.locator("[data-add-project]")).toHaveCount(0);
    await expect(page.locator("[data-overview-page]")).toBeVisible();

    // Folded, the project's circle keeps a `+` beside its name; its start circle goes with the fold.
    const project = graph.locator("[data-graph-project]").first();
    await project.locator(".og-project-name").hover();
    await project.locator(".og-project-fold").click();
    const cluster = graph.locator("[data-graph-cluster]").first();
    await expect(cluster).toBeVisible();
    await expect(graph.locator("[data-graph-start-slot]")).toHaveCount(0);
    await cluster.locator("strong").hover();
    await screenshot(page, "overview-start-folded");
    await cluster.locator("[data-graph-project-start]").click();
    await expect(panel.locator("[data-start-target]")).toHaveAttribute("data-start-target", new RegExp(`^checkout:${daemon.node}:.*/fixture$`));
  } finally { daemon?.stop(); engine.stop(); }
});
