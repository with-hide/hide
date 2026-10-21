// PRD overview-main-map B1, B13, B14, B17, B22, B23, B25, B26: the private
// runtime supplies identity and topology; camera, drags, folding and hiding stay
// local to the mounted force map.
import { expect, test, type Locator, type Page } from "@playwright/test";
import { startHerdr } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { openOverview } from "./overview-entry";
import { enterWorkspace } from "./wire";

async function drag(page: Page, handle: Locator, dx: number, dy: number) {
  const box = await handle.boundingBox();
  if (!box) throw new Error("drag handle has no visible bounds");
  const x = box.x + box.width / 2;
  const y = box.y + box.height / 2;
  await page.mouse.move(x, y);
  await page.mouse.down();
  try { await page.mouse.move(x + dx, y + dy, { steps: 8 }); }
  finally { await page.mouse.up(); }
}

test("@platform Overview restores its exact current session and keeps map movement local", async ({ page }) => {
  const herdr = await startHerdr();
  let daemon: Daemon | undefined;
  try {
    daemon = await startHided(herdr, "overview-radial");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    await openOverview(page, "work");
    const graph = page.locator("[data-overview-graph]");
    await expect(graph).toHaveAttribute("data-graph-phase", "ready", { timeout: 30_000 });
    const current = graph.locator('.og-session[data-current="true"]');
    await expect(current).toHaveCount(1);
    await expect(current.locator(".og-session-target")).toHaveAttribute("aria-pressed", "true");
    await expect(graph.locator("[data-graph-preview]")).toHaveCount(0);
    const canvas = await graph.locator(".og-canvas").boundingBox();
    const orb = await current.locator(".og-orb").boundingBox();
    const title = await current.locator(".og-session-title").boundingBox();
    if (!canvas || !orb || !title) throw new Error("current identity has no canvas geometry");
    expect(orb.y).toBeGreaterThanOrEqual(canvas.y);
    expect(title.y + title.height).toBeLessThanOrEqual(canvas.y + canvas.height);
    // B1: the operator sits at the centre and the project's name is drawn.
    await expect(graph.locator(".og-operator")).toHaveCount(1);
    await expect(graph.locator("[data-graph-project]")).toHaveCount(1);
    const positions = () => graph.locator(".react-flow__node-session").evaluateAll(nodes => nodes.map(node => [node.getAttribute("data-id"), (node as HTMLElement).style.transform]));
    const before = await positions();
    // B13/B14: hiding the project removes its sessions; showing it again restores the same places.
    const row = graph.locator("[data-graph-project-row]").first();
    await row.getByRole("button", { name: /^Hide .* on the map$/ }).click();
    await expect(graph.locator("[data-graph-session]")).toHaveCount(0);
    await expect(row).toHaveAttribute("data-hidden", "true");
    await row.getByRole("button", { name: /^Show .* on the map again$/ }).click();
    await expect(current).toHaveCount(1);
    await expect.poll(positions).toEqual(before);
    // B23: operators grab the provider circle itself; it follows the hand and stays where it is dropped.
    const held = await current.locator(".og-orb").boundingBox();
    if (!held) throw new Error("current session has no orb");
    await drag(page, current.locator(".og-orb"), 80, 48);
    await expect.poll(async () => {
      const dropped = await current.locator(".og-orb").boundingBox();
      return dropped ? Math.hypot(dropped.x - held.x - 80, dropped.y - held.y - 48) < 16 : false;
    }).toBe(true);
    await expect(graph.locator("[data-graph-preview]")).toHaveCount(0);
    await current.locator(".og-orb").click();
    await expect(graph.locator("[data-graph-preview]")).toHaveCount(1);
    await page.keyboard.press("Escape");
    await expect(graph.locator("[data-graph-preview]")).toHaveCount(0);
    await graph.getByRole("button", { name: "Reset layout", exact: true }).click();
    await expect.poll(positions).toEqual(before);
    // B22: dragging a project's name moves every one of its sessions.
    const name = graph.locator("[data-graph-project]").first();
    expect(before).toHaveLength(2);
    const grabbed = await name.boundingBox();
    if (!grabbed) throw new Error("project name has no bounds");
    await drag(page, name, 40, 24);
    await expect.poll(async () => (await positions()).filter((row, i) => JSON.stringify(row) !== JSON.stringify(before[i])).length).toBe(2);
    // The name lands where the hand let go, however many steps the drag took.
    await expect.poll(async () => {
      const dropped = await name.boundingBox();
      return dropped ? Math.hypot(dropped.x - grabbed.x - 40, dropped.y - grabbed.y - 24) < 8 : false;
    }).toBe(true);
    await graph.getByRole("button", { name: "Reset layout", exact: true }).click();
    await expect.poll(positions).toEqual(before);
    // B25/B26: folding all turns the project into one circle; unfolding brings every session back to its place.
    await graph.getByRole("button", { name: "Fold all", exact: true }).click();
    await expect(graph.locator("[data-graph-cluster]")).toHaveCount(1);
    await expect(graph.locator("[data-graph-session]")).toHaveCount(0);
    await graph.getByRole("button", { name: "Unfold all", exact: true }).click();
    await expect(graph.locator("[data-graph-cluster]")).toHaveCount(0);
    await expect.poll(positions).toEqual(before);
  } finally { daemon?.stop(); herdr.stop(); }
});
