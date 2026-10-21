// Approved radial-map B4-B11/B17/B24 and PRD overview-main-map B1, B13, B14, B17:
// the private runtime supplies identity and topology; camera, drags and hiding
// stay local to the mounted graph and never move another circle.
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
    // Team outlines follow their members; every other node keeps its own place.
    const positions = () => graph.locator(".react-flow__node").evaluateAll(nodes => nodes.filter(node => !node.getAttribute("data-id")?.startsWith("team:")).map(node => [node.getAttribute("data-id"), (node as HTMLElement).style.transform]));
    const before = await positions();
    // B13/B14: hiding the project removes its sessions; showing it again restores the same places.
    const row = graph.locator("[data-graph-project-row]").first();
    await row.getByRole("button", { name: /^Hide .* on the map$/ }).click();
    await expect(graph.locator("[data-graph-session]")).toHaveCount(0);
    await expect(row).toHaveAttribute("data-hidden", "true");
    await row.getByRole("button", { name: /^Show .* on the map again$/ }).click();
    await expect(current).toHaveCount(1);
    await expect.poll(positions).toEqual(before);
    const session = current.locator("xpath=..");
    const originalSession = await session.getAttribute("style");
    // Operators grab the provider circle itself; no separate grip control.
    await drag(page, current.locator(".og-orb"), 48, 32);
    await expect.poll(() => session.getAttribute("style")).not.toBe(originalSession);
    const afterSession = await positions();
    expect(afterSession.filter((row, i) => JSON.stringify(row) !== JSON.stringify(before[i]))).toHaveLength(1);
    await expect(graph.locator("[data-graph-preview]")).toHaveCount(0);
    await current.locator(".og-orb").click();
    await expect(graph.locator("[data-graph-preview]")).toHaveCount(1);
    await page.keyboard.press("Escape");
    await expect(graph.locator("[data-graph-preview]")).toHaveCount(0);
    await graph.getByRole("button", { name: "Reset layout", exact: true }).click();
    await expect.poll(positions).toEqual(before);
    // React Flow renders absolute transforms even though our node data is relative.
    // Dragging a project's name moves exactly that name and its sessions.
    const group = graph.locator(".react-flow__node-project").first();
    const projectId = await group.getAttribute("data-id");
    const members = await graph.locator(".react-flow__node-session").evaluateAll(nodes => nodes.map(node => node.getAttribute("data-id")));
    expect(members).toHaveLength(2);
    const originalGroup = await group.getAttribute("style");
    await drag(page, group.locator(".og-project-drag"), 40, 24);
    await expect.poll(() => group.getAttribute("style")).not.toBe(originalGroup);
    const afterGroup = await positions();
    expect(afterGroup.filter((row, i) => JSON.stringify(row) !== JSON.stringify(before[i])).map(row => row[0]).sort()).toEqual([projectId, ...members].sort());
    await graph.getByRole("button", { name: "Reset layout", exact: true }).click();
    await expect.poll(positions).toEqual(before);
  } finally { daemon?.stop(); herdr.stop(); }
});
