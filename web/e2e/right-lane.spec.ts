import { expect, test, type Page } from "@playwright/test";
import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fixtureExecutable } from "./platform-fixture";
import { runInPane, startHerdr, type HerdrFixture } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { chord } from "./chords";
import { quietFor } from "./wait";
import { countSent, enterWorkspace, screenshot, setLanePinned, showTool } from "./wire";

// PRD right-lane and PRD lane-two-columns: one lane beside or over Agent
// Views, two columns (the documents or the 보기 slot, and one tool), a rail
// of the viewing column's buttons over four tools, one pin. Each test starts
// one stack and asserts one contract.

const screen = (page: Page) => page.locator("[data-workspace-screen]");
const railTool = (page: Page, tool: string) => page.locator(`[data-lane-tool="${tool}"]`);
const front = (page: Page) => page.locator('[data-view-tab-bar] [role="tab"][aria-selected="true"]');
const pane = (page: Page, id: string) => page.locator(`[data-pane-view="${id}"]`);
const documentArea = (page: Page) => page.locator('[data-lane-frame] [data-view-area="true"]');

type Lane = { page: Page; herdr: HerdrFixture; daemon: Daemon; second: { tab: string; pane: string }; sent: Map<string, number> };

/** A Workspace with two Agent tabs, three files and a page counting what it sends. */
async function startLane(page: Page, name: string): Promise<Lane> {
  const herdr = await startHerdr({ agents: false });
  const fixture = path.join(herdr.root, "fixture");
  fs.mkdirSync(path.join(fixture, "docs"), { recursive: true });
  for (const file of ["a.md", "b.md", "docs/c.md"]) fs.writeFileSync(path.join(fixture, file), `${file}\n`);
  const created = herdr.run(["tab", "create", "--workspace", herdr.workspace, "--cwd", fixture, "--label", "second", "--no-focus"]) as {
    result: { tab: { tab_id: string }; root_pane: { pane_id: string } };
  };
  const daemon = await startHided(herdr, name);
  const sent = countSent(page);
  await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
  await enterWorkspace(page, "fixture");
  return { page, herdr, daemon, second: { tab: created.result.tab.tab_id, pane: created.result.root_pane.pane_id }, sent };
}

async function openFile(lane: Lane, paneId: string, name: string): Promise<void> {
  const hide = path.resolve("..", "target", "debug", fixtureExecutable("hide"));
  const ran = await runInPane(lane.herdr, paneId, name, { env: { HIDE_STATE_DIR: lane.daemon.stateDir }, argv: [hide, "file", "open", name] });
  expect(ran.status, ran.stderr).toBe(0);
}

const resizes = (lane: Lane) => lane.sent.get("terminal_resize") ?? 0;

type SavedRead = { bytes: Buffer } | { read_error: string };
type SavedArea = { id: string; active: string | null; displays: { id: string; path?: string; kind?: string }[] };
type SavedNode = { area: SavedArea } | { split: { first: SavedNode; second: SavedNode } };
type SavedLayout = { active_area: string; root: SavedNode };

/** Only read bytes in the crash window; parsing and evidence I/O happen after exit. */
function readSavedViews(stateDir: string): SavedRead {
  try { return { bytes: fs.readFileSync(path.join(stateDir, "workspace-views.json")) }; }
  catch (error) { return { read_error: (error as NodeJS.ErrnoException).code ?? "unknown_read_error" }; }
}

/** Allowlisted fixture facts, never raw saved text, paths, prompts or error messages. */
function savedViewFacts(read: SavedRead, fixture: string): unknown {
  if ("read_error" in read) return { status: "read_error", code: read.read_error };
  const hash = (bytes: Buffer | string) => createHash("sha256").update(bytes).digest("hex");
  const metadata = { bytes: read.bytes.length, sha256: hash(read.bytes) };
  try {
    const saved = JSON.parse(read.bytes.toString("utf8")) as { schema_version: number; workspaces: { path: string; open: boolean; documents: boolean; tool: string | null; pinned: boolean; view_layout: SavedLayout; agent_view_layout: SavedLayout }[] };
    if (!Number.isInteger(saved.schema_version) || !Array.isArray(saved.workspaces)) throw new TypeError();
    const views = saved.workspaces.filter((view) => view.path.replaceAll("\\", "/") === fixture.replaceAll("\\", "/"));
    if (views.length !== 1) return { ...metadata, status: "workspace_match_error", schema_version: saved.schema_version, matches: views.length };
    const view = views[0]!;
    if (typeof view.pinned !== "boolean" || typeof view.open !== "boolean" || typeof view.documents !== "boolean" || ![null, "explorer", "project", "changes"].includes(view.tool)) throw new TypeError();
    const layout = (value: SavedLayout) => {
      const areas: { id: string; active: string | null; displays: { id: string; basename?: string; kind?: string }[] }[] = [];
      const identity = (id: string) => { if (typeof id !== "string") throw new TypeError(); return hash(id); };
      const visit = (node: SavedNode, depth: number) => {
        if (depth > 3) throw new TypeError();
        if ("area" in node) {
          const area = node.area;
          areas.push({ id: identity(area.id), active: area.active === null ? null : identity(area.active), displays: area.displays.map((display) => {
            const facts: { id: string; basename?: string; kind?: string } = { id: identity(display.id) };
            if (display.path !== undefined) {
              const name = path.basename(display.path);
              if (!["a.md", "b.md", "c.md"].includes(name)) throw new TypeError();
              facts.basename = name;
              if (!["file", "diff"].includes(display.kind!)) throw new TypeError();
              facts.kind = display.kind;
            }
            return facts;
          }) });
        } else { visit(node.split.first, depth + 1); visit(node.split.second, depth + 1); }
      };
      visit(value.root, 0);
      const activeArea = areas.find((area) => area.id === identity(value.active_area));
      const active_basename = activeArea?.displays.find((display) => display.id === activeArea.active)?.basename ?? null;
      return { active_area: identity(value.active_area), active_basename, display_count: areas.reduce((count, area) => count + area.displays.length, 0), areas };
    };
    return { ...metadata, status: "read", schema_version: saved.schema_version, open: view.open, documents: view.documents, tool: view.tool, pinned: view.pinned, views: layout(view.view_layout), agents: layout(view.agent_layout) };
  } catch (error) {
    return { ...metadata, status: error instanceof SyntaxError ? "parse_error" : "shape_error" };
  }
}

test("an opened file comes up over Agent Views without resizing a terminal, stays through a terminal click, and Escape puts it away with its document (PRD right-lane B1, B3, B4, B16; lane-two-columns B6, B11, B17, B20) @platform", async ({ page }) => {
  test.setTimeout(120_000);
  const lane = await startLane(page, "lane-overlay");
  try {
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "closed");
    await expect(page.locator("[data-lane-tool]")).toHaveCount(4);
    await expect(railTool(page, "documents")).toBeDisabled();
    await expect(pane(page, lane.herdr.panes[0]!)).toBeVisible();
    await quietFor(page, 500, "the first fit has landed before the lane opens");
    const before = resizes(lane);

    await openFile(lane, lane.herdr.panes[0]!, "a.md");
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "overlay");
    await expect(screen(page)).toHaveAttribute("data-lane-view", "documents");
    await expect(screen(page)).toHaveAttribute("data-lane-shows", "none");
    await expect(front(page)).toHaveAttribute("aria-label", /\/a\.md/);
    await expect(railTool(page, "documents")).toHaveAttribute("aria-pressed", "true");
    await expect(page.locator("[data-lane-header]")).toHaveCount(0);
    await screenshot(page, "right-lane-overlay");

    // The divider at the overlay's left edge widens the documents by what it
    // is dragged, lands once on release, and resizes no terminal (B17).
    const laneBox = (await page.locator('[data-lane-frame="overlay"]').boundingBox())!;
    const grip = page.locator('[data-lane-frame="overlay"] [data-lane-divider="lane"]');
    await expect(grip).toHaveAttribute("aria-valuenow", String(Math.round(laneBox.width)));
    const gripBox = (await grip.boundingBox())!;
    await page.mouse.move(gripBox.x + gripBox.width / 2, gripBox.y + 200);
    await page.mouse.down();
    await page.mouse.move(gripBox.x + gripBox.width / 2 - 120, gripBox.y + 200, { steps: 6 });
    await expect(page.locator("[data-lane-guide]")).toHaveCount(1);
    await page.mouse.up();
    await expect(page.locator("[data-lane-guide]")).toHaveCount(0);
    // The pointer lands on a whole pixel, so the width grows by the drag within a pixel.
    const widened = () => page.locator('[data-lane-frame="overlay"]').boundingBox().then((box) => Math.round(box!.width));
    await expect.poll(async () => Math.abs((await widened()) - (laneBox.width + 120))).toBeLessThanOrEqual(1);
    await expect(grip).toHaveAttribute("aria-valuenow", String(await widened()));
    const wide = await widened();
    await screenshot(page, "right-lane-documents-wide");

    // A tool opens beside the documents at a width of its own: the wide
    // document leaves the Project tool at the default, the column divider
    // trades width between the two, and the tool's × leaves the documents at
    // the width that trade left them (B14, B17).
    await railTool(page, "project").click();
    await expect(page.locator("[data-lane-header]")).toContainText("fixture");
    await expect(screen(page)).toHaveAttribute("data-lane-shows", "project");
    await expect(screen(page)).toHaveAttribute("data-lane-view", "documents");
    const memory = () => page.locator('[data-lane-column="tool"]').boundingBox().then((box) => Math.round(box!.width));
    await expect.poll(memory).toBe(400);
    expect(Math.abs((await documentArea(page).boundingBox())!.width - wide)).toBeLessThanOrEqual(1);
    await screenshot(page, "right-lane-project-beside");
    // The divider between the columns trades width between them; the lane keeps its own.
    const laneWide = await widened();
    const columnGrip = page.locator('[data-lane-divider="column"]');
    const columnBox = (await columnGrip.boundingBox())!;
    await page.mouse.move(columnBox.x + columnBox.width / 2, columnBox.y + 200);
    await page.mouse.down();
    await page.mouse.move(columnBox.x + columnBox.width / 2 - 60, columnBox.y + 200, { steps: 6 });
    await page.mouse.up();
    await expect.poll(memory).toBe(460);
    expect(await widened()).toBe(laneWide);
    await page.locator('[data-lane-tool-close="project"]').click();
    await expect(screen(page)).toHaveAttribute("data-lane-shows", "none");
    await expect.poll(widened).toBe(wide - 60);
    await expect(pane(page, lane.herdr.panes[0]!)).toBeVisible();
    expect(resizes(lane)).toBe(before);

    // A terminal click takes the keyboard and leaves the unpinned lane up:
    // only the operator takes it down (PRD right-lane B4).
    await pane(page, lane.herdr.panes[0]!).click();
    await expect(pane(page, lane.herdr.panes[0]!).locator(".xterm-helper-textarea")).toBeFocused();
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "overlay");
    await expect(front(page)).toHaveAttribute("aria-label", /\/a\.md/);

    // Escape in the lane puts it away; the keyboard goes back to the pane.
    await documentArea(page).locator(".cm-content").click();
    await page.keyboard.press("Escape");
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "hidden");
    await expect(pane(page, lane.herdr.panes[0]!).locator(".xterm-helper-textarea")).toBeFocused();
    await expect(page.locator('[data-lane-badge="1"]')).toBeVisible();
    await quietFor(page, 500, "no terminal was fitted again while the lane came and went");
    expect(resizes(lane)).toBe(before);

    // Escape on the rail puts it away too.
    await railTool(page, "documents").click();
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "overlay");
    await page.keyboard.press("Escape");
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "hidden");

    // The rail's documents button, pressed while they show, folds them; with
    // no tool column the lane has nothing left and closes (B11, B12).
    await railTool(page, "documents").click();
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "overlay");
    await railTool(page, "documents").click();
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "closed");
    await expect(page.locator('[data-lane-badge="1"]')).toBeVisible();
    await railTool(page, "documents").click();
    await expect(screen(page)).toHaveAttribute("data-lane-view", "documents");
    await expect(front(page)).toHaveAttribute("aria-label", /\/a\.md/);
  } finally {
    lane.daemon.stop();
    lane.herdr.stop();
  }
});

test("the rail draws the viewing column's buttons over a rule and one divider between the Project's tools and the checkout's, and every tool column's header keeps its scope icon (D-37, B2, B3, B54)", async ({ page }) => {
  test.setTimeout(120_000);
  const lane = await startLane(page, "lane-scope");
  try {
    // The documents' button, a rule, then one short rule where the Project's tools end and the checkout's begin.
    const rail = await page.locator("[data-tool-rail] > [data-lane-tool], [data-tool-rail] > [data-tool-rail-divider]").evaluateAll((nodes) => nodes.map((node) => node.getAttribute("data-lane-tool") ?? `divider:${node.getAttribute("data-tool-rail-divider")}`));
    expect(rail).toEqual(["documents", "divider:view", "project", "memory", "divider:true", "explorer", "changes"]);
    // The Project tool's head names the project and where it is, with ⚙ and × (PRD lane-two-columns B26).
    await showTool(page, "project");
    const projectHeader = page.locator('aside[data-lane="project"] [data-lane-header]');
    await expect(projectHeader.locator('[data-lane-scope-kind="project"]')).toContainText("fixture");
    await expect(projectHeader.getByRole("button")).toHaveCount(2);
    await expect(projectHeader.locator('[data-lane-tool-close="project"]')).toBeVisible();
    // Every checkout tool reads `name · scope`, with decoration only, and closes with one ×.
    for (const [tool, name, icon] of [["explorer", "Explorer", "git-branch"], ["changes", "Changes", "git-branch"]] as const) {
      await showTool(page, tool);
      const header = page.locator("[data-lane-header]");
      const scope = header.locator("[data-lane-scope]");
      await expect(scope.locator(`svg.lucide-${icon}[aria-hidden="true"]`)).toHaveCount(1);
      await expect(scope.locator("svg.lucide-folder, svg.lucide-git-branch")).toHaveCount(1);
      const text = (await scope.textContent())!.replace(/\s+/g, " ").trim();
      expect(text, tool).toMatch(new RegExp(`^${name} · \\S`));
      await expect(header.getByRole("button")).toHaveCount(1);
      await expect(header.locator(`[data-lane-tool-close="${tool}"]`)).toBeVisible();
    }
  } finally {
    lane.daemon.stop();
    lane.herdr.stop();
  }
});

test("the pin docks the lane once, keeps it through a terminal click, a tool and an Agent tab, and a restart brings it back docked (PRD right-lane B5, B7, B15, B22, B24; lane-two-columns B21, B23) @platform", async ({ page }) => {
  test.setTimeout(150_000);
  const lane = await startLane(page, "lane-pin");
  try {
    await openFile(lane, lane.herdr.panes[0]!, "a.md");
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "overlay");
    await quietFor(page, 500, "the overlay has settled");
    const before = resizes(lane);
    const agents = await page.locator("[data-agent-area]").boundingBox();

    await setLanePinned(page, true);
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "docked");
    // One fit per attached terminal, the background tab's included.
    const attached = await page.locator("[data-pane-view]").count();
    await expect.poll(() => resizes(lane)).toBe(before + attached);
    await quietFor(page, 500, "the pin fitted each terminal once");
    expect(resizes(lane)).toBe(before + attached);
    const docked = await page.locator("[data-agent-area]").boundingBox();
    expect(docked!.width).toBeLessThan(agents!.width);
    await screenshot(page, "right-lane-docked");

    await pane(page, lane.herdr.panes[0]!).click();
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "docked");
    // A tool opens beside the documents and fits each terminal once more (B21).
    await railTool(page, "project").click();
    await expect(page.locator("[data-lane]")).toHaveAttribute("data-lane", "project");
    await expect(screen(page)).toHaveAttribute("data-lane-view", "documents");
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "docked");
    await expect.poll(() => resizes(lane)).toBe(before + 2 * attached);

    // Another Agent tab keeps the lane and adds its View to the checkout's
    // one list; returning to the first Agent keeps the View just selected.
    const tab = (id: string) => page.locator(`[data-agent-tab-bar] [data-tab="${id}"]`);
    await tab(lane.second.tab).click();
    await openFile(lane, lane.second.pane, "b.md");
    await expect(front(page)).toHaveAttribute("aria-label", /\/b\.md/);
    await tab(lane.herdr.tab).click();
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "docked");
    await expect(front(page)).toHaveAttribute("aria-label", /\/b\.md/);
    await expect(page.locator('[data-view-tab-bar] [role="tab"]')).toHaveCount(2);

    // Windows stops the fixture forcibly. A rendered selection does not
    // prove the asynchronous save the next process will restore has landed.
    const fixture = path.join(lane.herdr.root, "fixture");
    const stored = () => savedViewFacts(readSavedViews(lane.daemon.stateDir), fixture);
    await expect.poll(stored).toMatchObject({ status: "read", open: true, documents: true, tool: "project", pinned: true, views: { display_count: 2, active_basename: "b.md" } });
    const savedBeforeExit = readSavedViews(lane.daemon.stateDir);
    let savedAfterExit: SavedRead | undefined;
    try {
      lane.daemon = await lane.daemon.restart((stateDir) => { savedAfterExit = readSavedViews(stateDir); });
    } finally {
      if (savedAfterExit) {
        await test.info().attach("first-restart-saved-views", { contentType: "application/json", body: JSON.stringify({ before_exit: savedViewFacts(savedBeforeExit, fixture), after_exit: savedViewFacts(savedAfterExit, fixture) }, null, 2) });
      }
    }
    await page.goto("about:blank");
    await page.goto(`${lane.daemon.origin}/?probe=1#token=${lane.daemon.token}`);
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "docked");
    await expect(screen(page)).toHaveAttribute("data-lane-shows", "project");
    await expect(front(page)).toHaveAttribute("aria-label", /\/b\.md/);

    // Unpinned, the lane goes back over Agent Views with both columns, and a
    // restart brings it back there: the operator never took it down.
    await setLanePinned(page, false);
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "overlay");
    await expect.poll(stored).toMatchObject({ status: "read", open: true, documents: true, tool: "project", pinned: false, views: { display_count: 2, active_basename: "b.md" } });
    lane.daemon = await lane.daemon.restart();
    await page.goto("about:blank");
    await page.goto(`${lane.daemon.origin}/?probe=1#token=${lane.daemon.token}`);
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "overlay");
    await expect(railTool(page, "documents")).toHaveAttribute("aria-pressed", "true");
    await expect(railTool(page, "project")).toHaveAttribute("aria-pressed", "true");
  } finally {
    lane.daemon.stop();
    lane.herdr.stop();
  }
});

test("closing the last view closes the lane when no tool shows, hands the keyboard to the pane, and the lane chord brings the Explorer back (B12, B15)", async ({ page }) => {
  test.setTimeout(120_000);
  const lane = await startLane(page, "lane-last-view");
  try {
    await openFile(lane, lane.herdr.panes[0]!, "a.md");
    await expect(front(page)).toHaveAttribute("aria-label", /\/a\.md/);
    await expect(screen(page)).toHaveAttribute("data-lane-shows", "none");

    await documentArea(page).locator(".cm-content").click();
    await page.keyboard.press(chord("close_tab"));
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "closed");
    await expect(page.locator('[data-lane="explorer"]')).toHaveCount(0);
    await expect(pane(page, lane.herdr.panes[0]!).locator(".xterm-helper-textarea")).toBeFocused();
    await expect(railTool(page, "documents")).toBeDisabled();
    await railTool(page, "documents").hover();
    await expect(page.locator('[role="tooltip"]')).toHaveText(/No open document/);

    // A lane with no column and no document comes back on the Explorer.
    await page.keyboard.press(chord("toggle_lane"));
    await expect(screen(page)).toHaveAttribute("data-lane-shows", "explorer");
    await expect(screen(page)).toHaveAttribute("data-lane-view", "none");
    await expect(page.locator('[data-lane="explorer"] [data-explorer]')).toBeVisible();
  } finally {
    lane.daemon.stop();
    lane.herdr.stop();
  }
});

test("the Explorer opens beside the document without narrowing it, stays through a tool switch and the last view closing, and its chord closes the column alone (B4, B5, B7, B8, B12, B14) @platform", async ({ page }) => {
  test.setTimeout(120_000);
  const lane = await startLane(page, "lane-explorer");
  try {
    await openFile(lane, lane.herdr.panes[0]!, "a.md");
    await expect(front(page)).toHaveAttribute("aria-label", /\/a\.md/);
    const width = (await documentArea(page).boundingBox())!.width;
    await expect(railTool(page, "explorer")).toHaveAttribute("aria-pressed", "false");

    await railTool(page, "explorer").click();
    await expect(page.locator('[data-lane="explorer"] [data-explorer]')).toBeVisible();
    await expect(railTool(page, "explorer")).toHaveAttribute("aria-pressed", "true");
    await expect(railTool(page, "documents")).toHaveAttribute("aria-pressed", "true");
    await expect(railTool(page, "documents").locator("[data-lane-badge]")).toHaveText("1");
    expect(Math.abs((await documentArea(page).boundingBox())!.width - width)).toBeLessThanOrEqual(1);
    // The Explorer has nothing of its own to turn off: no toggle, no hint about a tree alone (B5).
    await expect(page.locator("[data-lane-tree-toggle], [data-lane-tree-footer]")).toHaveCount(0);
    await screenshot(page, "right-lane-explorer");

    // Another tool replaces the Explorer in the tool column; the documents stay.
    await railTool(page, "changes").click();
    await expect(screen(page)).toHaveAttribute("data-lane-shows", "changes");
    await expect(screen(page)).toHaveAttribute("data-lane-view", "documents");
    await railTool(page, "explorer").click();
    await expect(page.locator('[data-lane="explorer"] [data-explorer]')).toBeVisible();

    // The last view closed closes the viewing column; the Explorer stays (B12).
    await page.getByRole("button", { name: "Close view a.md" }).click();
    await expect(screen(page)).toHaveAttribute("data-lane-view", "none");
    await expect(screen(page)).toHaveAttribute("data-lane-shows", "explorer");
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "overlay");
    await expect(railTool(page, "documents")).toBeDisabled();
    await screenshot(page, "right-lane-explorer-alone");

    // Picking a file opens it beside the same Explorer (B6); the chord
    // closes the Explorer column alone, the document staying (B14).
    await page.locator('[data-lane="explorer"]').evaluate((column) => { (column as HTMLElement).dataset.probe = "kept"; });
    await page.locator('[data-explorer-row$="/b.md"]').dblclick();
    await expect(front(page)).toHaveAttribute("aria-label", /\/b\.md/);
    await expect(page.locator('[data-lane="explorer"][data-probe="kept"]')).toBeVisible();
    await page.locator('[data-lane-frame] [data-view-area="true"] .cm-content').click();
    await page.keyboard.press(chord("toggle_lane_explorer"));
    await expect(page.locator('[data-lane="explorer"]')).toHaveCount(0);
    await expect(screen(page)).toHaveAttribute("data-lane-view", "documents");
    await expect(front(page)).toHaveAttribute("aria-label", /\/b\.md/);
    // The chord opens it again beside the document.
    await page.keyboard.press(chord("toggle_lane_explorer"));
    await expect(page.locator('[data-lane="explorer"] [data-explorer]')).toBeVisible();
    await expect(screen(page)).toHaveAttribute("data-lane-view", "documents");
  } finally {
    lane.daemon.stop();
    lane.herdr.stop();
  }
});

test("expand takes the whole body and returns on Escape without resizing a terminal (PRD right-lane B8; lane-two-columns B15) @platform", async ({ page }) => {
  test.setTimeout(120_000);
  const lane = await startLane(page, "lane-expand");
  try {
    await openFile(lane, lane.herdr.panes[0]!, "a.md");
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "overlay");
    await quietFor(page, 500, "the overlay has settled");
    const before = resizes(lane);
    await page.locator("[data-lane-more]").click();
    await page.locator('[data-lane-menu-item="expand"]').click();
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "expanded");
    const row = (await page.locator("[data-lane-row]").boundingBox())!;
    expect(Math.abs((await page.locator("[data-lane-frame]").boundingBox())!.width - row.width)).toBeLessThanOrEqual(1);
    await screenshot(page, "right-lane-expanded");
    await page.locator('[data-lane-frame] [data-view-area="true"] .cm-content').click();
    await page.keyboard.press("Escape");
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "overlay");
    // The pane zoom's chord, pressed in the document, expands the lane and back; no pane zooms.
    await page.keyboard.press(chord("toggle_zoom"));
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "expanded");
    await page.keyboard.press(chord("toggle_zoom"));
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "overlay");
    await quietFor(page, 500, "no terminal was fitted again while the lane expanded");
    expect(resizes(lane)).toBe(before);
    expect(lane.sent.get("toggle_zoom") ?? 0).toBe(0);
  } finally {
    lane.daemon.stop();
    lane.herdr.stop();
  }
});

test("the rail shows icons alone and names each tool in its hint in four languages and both themes, and Tab reaches each tool with its pressed state (B23)", async ({ page }) => {
  test.setTimeout(120_000);
  const lane = await startLane(page, "lane-rail");
  try {
    await openFile(lane, lane.herdr.panes[0]!, "a.md");
    await expect(railTool(page, "documents")).toHaveAttribute("aria-pressed", "true");
    const rail = page.locator("[data-tool-rail]");
    // The rail draws no text of its own but the documents count: a tool's name
    // is its accessible name and the hint that opens on hover, with the tool's
    // scope and chord.
    await expect(rail).toHaveText("1");
    for (const [language, theme] of [
      ["en", "light"],
      ["ko", "dark"],
      ["zh-CN", "light"],
      ["ja", "dark"],
    ] as const) {
      await page.locator("[data-open-settings]").click();
      await page.locator('[data-settings-tab="general"]').click();
      await page.locator("[data-interface-language]").click();
      await page.locator(`[data-language-option="${language}"]`).click();
      await expect(page.locator("html")).toHaveAttribute("lang", language);
      await page.locator(`[data-theme-option="${theme}"]`).click();
      await page.keyboard.press("Escape");
      await expect(page.locator('[data-settings="true"]')).toHaveCount(0);
      // The hint reads `<name> · <scope>`, and the name is the button's accessible name.
      await railTool(page, "changes").hover();
      const hint = page.locator('[data-slot="tooltip-content"]');
      await expect(hint).toContainText(" · ");
      await expect(railTool(page, "changes")).toHaveAttribute("aria-label", (await hint.innerText()).split(" · ")[0]!.trim());
      await screenshot(page, `right-lane-rail-${language}-${theme}`);
      await page.mouse.move(400, 400);
      await expect(page.locator('[data-slot="tooltip-content"]')).toHaveCount(0);
    }

    // The keyboard reaches the rail; each tool says whether it shows.
    await railTool(page, "explorer").focus();
    await page.keyboard.press("Tab");
    await expect(railTool(page, "changes")).toBeFocused();
    await expect(railTool(page, "changes")).toHaveAttribute("aria-pressed", "false");
    await page.keyboard.press("Enter");
    await expect(railTool(page, "changes")).toHaveAttribute("aria-pressed", "true");
    await expect(railTool(page, "explorer")).toHaveAttribute("aria-pressed", "false");
    await expect(railTool(page, "documents")).toHaveAttribute("aria-pressed", "true");
  } finally {
    lane.daemon.stop();
    lane.herdr.stop();
  }
});
