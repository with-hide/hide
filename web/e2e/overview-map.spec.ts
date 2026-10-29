// PRD overview-main-map B1, B13, B14, B17, B21, B22, B23, B25, B26: the private
// runtime supplies identity and topology; the camera stays local to the mounted
// force map, while hiding, folding and dropped places are the core's.
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
  const herdr = await startHerdr({ readiness: "registered" });
  let daemon: Daemon | undefined;
  try {
    daemon = await startHided(herdr, "overview-map");
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
    await row.getByRole("button", { name: /^Hide .* from the graph$/ }).click();
    await expect(graph.locator("[data-graph-session]")).toHaveCount(0);
    await expect(row).toHaveAttribute("data-hidden", "true");
    await row.getByRole("button", { name: /^Show .* on the graph again$/ }).click();
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
    const name = graph.locator("[data-graph-project] .og-project-name").first();
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
    // B21: pointing at the name makes the project's glow stand out, and it returns when the pointer leaves.
    const glow = graph.locator(".og-glow").first();
    await name.hover();
    await expect(glow).toHaveAttribute("data-emphasized", "true");
    await page.mouse.move(1, 1);
    await expect(glow).toHaveAttribute("data-emphasized", "false");
    // So does pointing at its row in the side column, and choosing one of its sessions.
    await graph.locator("[data-graph-project-row]").first().hover();
    await expect(glow).toHaveAttribute("data-emphasized", "true");
    await page.mouse.move(1, 1);
    await current.locator(".og-orb").click();
    await expect(glow).toHaveAttribute("data-emphasized", "true");
    await page.keyboard.press("Escape");
    await expect(glow).toHaveAttribute("data-emphasized", "false");
    // B25: the fold button beside the name folds the project in the map; choosing the circle unfolds it in place.
    await name.hover();
    await graph.locator("[data-graph-project] .og-project-fold").first().click();
    await expect(graph.locator("[data-graph-cluster]")).toHaveCount(1);
    await expect(graph.locator("[data-graph-session]")).toHaveCount(0);
    // The fold replaced the name under the pointer; the folded circle is not left lit.
    await expect(graph.locator("[data-graph-cluster]")).toHaveAttribute("data-emphasized", "false");
    await graph.locator("[data-graph-cluster] .og-cluster-orb").click();
    await expect(graph.locator("[data-graph-cluster]")).toHaveCount(0);
    await expect(glow).toHaveAttribute("data-emphasized", "false");
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

// PRD overview-graph-remembered B1, B3, B5, B9: the core keeps the map's arrangement, so a
// reloaded shell opens it as it was left, and Reset layout's fresh map is what reopens.
test("Overview's work map keeps a hidden project and a dropped session across a reload until Reset layout", async ({ page }) => {
  const herdr = await startHerdr({ readiness: "registered" });
  let daemon: Daemon | undefined;
  try {
    daemon = await startHided(herdr, "overview-map-remembered");
    const graph = page.locator("[data-overview-graph]");
    const open = async (reload: boolean) => {
      if (reload) await page.reload();
      else await page.goto(`${daemon!.origin}/#token=${daemon!.token}`);
      await enterWorkspace(page, "fixture");
      await openOverview(page, "work");
      await expect(graph).toHaveAttribute("data-graph-phase", "ready", { timeout: 30_000 });
    };
    await open(false);
    const current = graph.locator('.og-session[data-current="true"]');
    const place = () => current.evaluate((node) => (node.closest(".react-flow__node") as HTMLElement).style.transform);
    const fresh = await place();
    await drag(page, current.locator(".og-orb"), 80, 48);
    await expect.poll(place).not.toBe(fresh);
    const dropped = await place();
    const row = graph.locator("[data-graph-project-row]").first();
    await row.getByRole("button", { name: /^Hide .* from the graph$/ }).click();
    await expect(row).toHaveAttribute("data-hidden", "true");
    await open(true);
    await expect(row).toHaveAttribute("data-hidden", "true");
    await expect(graph.locator("[data-graph-session]")).toHaveCount(0);
    await row.getByRole("button", { name: /^Show .* on the graph again$/ }).click();
    await expect(current).toHaveCount(1);
    await expect.poll(place).toBe(dropped);
    await graph.getByRole("button", { name: "Reset layout", exact: true }).click();
    await expect.poll(place).toBe(fresh);
    await open(true);
    await expect.poll(place).toBe(fresh);
  } finally { daemon?.stop(); herdr.stop(); }
});

// PRD pane-open-in-overview B4-B7, B12: an agent pane's menu opens the Overview's Work map with
// that pane's session chosen, its hidden project shown again, framed, and Escape steps back to the pane.
test("a pane's Show in Overview opens Work with its session chosen and gives the keyboard back", async ({ page }) => {
  const herdr = await startHerdr({ readiness: "registered" });
  let daemon: Daemon | undefined;
  try {
    daemon = await startHided(herdr, "overview-map-pane");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    await openOverview(page, "work");
    const graph = page.locator("[data-overview-graph]");
    await expect(graph).toHaveAttribute("data-graph-phase", "ready", { timeout: 30_000 });
    const current = graph.locator('.og-session[data-current="true"]');
    const pane = (await current.getAttribute("data-graph-session"))!;
    // The project is hidden and another tab was last; the button opens Work and shows the project again (B4, B7).
    const row = graph.locator("[data-graph-project-row]").first();
    await row.getByRole("button", { name: /^Hide .* from the graph$/ }).click();
    await expect(row).toHaveAttribute("data-hidden", "true");
    await openOverview(page, "projects");
    await page.keyboard.press("Escape");
    await expect(page.locator("[data-overview-page]")).toHaveCount(0);
    // The pane menu's first item opens it; the header keeps no button of its own.
    await page.locator(`[data-pane-menu="${pane}"]`).click();
    await expect(page.locator('[data-menu-item]').first()).toHaveAttribute("data-menu-item", "open_in_overview");
    await page.locator('[data-menu-item="open_in_overview"]').click();
    await expect(page.locator('[data-main-view="work"]')).toBeVisible();
    await expect(graph).toHaveAttribute("data-graph-phase", "ready", { timeout: 30_000 });
    // B5: chosen, with its details open and the camera on it.
    await expect(graph.locator("[data-graph-preview]")).toHaveCount(1);
    await expect(current.locator(".og-session-target")).toHaveAttribute("aria-pressed", "true");
    await expect.poll(async () => {
      const canvas = await graph.locator(".og-canvas").boundingBox();
      const orb = await current.locator(".og-orb").boundingBox();
      return canvas && orb ? Math.hypot(orb.x + orb.width / 2 - canvas.x - canvas.width / 2, orb.y + orb.height / 2 - canvas.y - canvas.height / 2) < 24 : false;
    }).toBe(true);
    // B6: Escape closes the details back to the projects, then the Overview, and the pane has the keyboard.
    await page.keyboard.press("Escape");
    await expect(graph.locator("[data-graph-preview]")).toHaveCount(0);
    await expect(graph.locator("[data-graph-projects]")).toBeVisible();
    await expect(row).toHaveAttribute("data-hidden", "false");
    await page.keyboard.press("Escape");
    await expect(page.locator("[data-overview-page]")).toHaveCount(0);
    await expect(page.locator(`[data-pane-view="${pane}"] .xterm-helper-textarea`)).toBeFocused();
    // B12: the sidebar's Overview only highlights the current pane's session.
    await openOverview(page, "work");
    await expect(graph).toHaveAttribute("data-graph-phase", "ready", { timeout: 30_000 });
    await expect(current.locator(".og-session-target")).toHaveAttribute("aria-pressed", "true");
    await expect(graph.locator("[data-graph-preview]")).toHaveCount(0);
  } finally { daemon?.stop(); herdr.stop(); }
});

// The side column's search finds a session on the map: what it does not find dims in place,
// and Enter shows the first found session with its details, as choosing it on the map does.
test("Overview's work map search dims what it does not find and Enter shows the found session", async ({ page }) => {
  const herdr = await startHerdr({ readiness: "registered" });
  let daemon: Daemon | undefined;
  try {
    daemon = await startHided(herdr, "overview-map-search");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    await openOverview(page, "work");
    const graph = page.locator("[data-overview-graph]");
    await expect(graph).toHaveAttribute("data-graph-phase", "ready", { timeout: 30_000 });
    const current = graph.locator('.og-session[data-current="true"]');
    const title = (await current.locator("[data-agent-title]").textContent())?.trim();
    if (!title) throw new Error("current session has no title");
    const search = graph.getByRole("textbox", { name: "Find project or session" });
    const place = () => current.evaluate((node) => (node.closest(".react-flow__node") as HTMLElement).style.transform);
    const before = await place();
    await search.fill("no session is called this");
    await expect(graph.locator(".og-no-match")).toHaveText("Nothing matches");
    await expect(current).toHaveAttribute("data-muted", "true");
    await expect(graph.locator("[data-graph-project]")).toHaveAttribute("data-muted", "true");
    expect(await place()).toBe(before);
    await search.fill(title);
    await expect(current).toHaveAttribute("data-muted", "false");
    await expect(graph.locator(`[data-graph-found-session="${await current.getAttribute("data-graph-session")}"]`)).toHaveCount(1);
    await search.press("Enter");
    await expect(graph.locator("[data-graph-preview] .og-details-head")).toContainText(title);
    await expect(current.locator(".og-session-target")).toHaveAttribute("aria-pressed", "true");
    await page.keyboard.press("Escape");
    await expect(graph.locator("[data-graph-preview]")).toHaveCount(0);
    await expect(search).toHaveValue(title);
  } finally { daemon?.stop(); herdr.stop(); }
});
