import { expect, test, type Page } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { fixtureExecutable } from "./platform-fixture";
import { focusPane, focusTab, newTab, runInPane, withControl } from "./daemon-actions";
import { startFixture, type AgentFixture } from "./engine-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { chord } from "./chords";
import { quietFor } from "./wait";
import { countSent, enterWorkspace, screenshot, showTool } from "./wire";

// PRD right-lane and PRD lane-two-columns: one lane beside or over Agent
// Views, two columns (the front Agent tab's desk and one tool), a rail
// of the viewing column's buttons over four tools, one pin. Each test starts
// one stack and asserts one contract.

const screen = (page: Page) => page.locator("[data-workspace-screen]");
const railTool = (page: Page, tool: string) => page.locator(`[data-lane-tool="${tool}"]`);
const front = (page: Page) => page.locator('[data-view-tab-bar] [role="tab"][aria-selected="true"]');
const pane = (page: Page, id: string) => page.locator(`[data-pane-view="${id}"]`);
const documentArea = (page: Page) => page.locator('[data-lane-frame] [data-view-area="true"]');

type Lane = { page: Page; fixture: AgentFixture; daemon: Daemon; second: { tab: string; pane: string }; sent: Map<string, number> };

/** A Workspace with two Agent tabs, three files and a page counting what it sends. */
async function startLane(page: Page, name: string): Promise<Lane> {
  const fixture = await startFixture({ agents: false });
  const folder = path.join(fixture.root, "fixture");
  fs.mkdirSync(path.join(folder, "docs"), { recursive: true });
  for (const file of ["a.md", "b.md", "docs/c.md"]) fs.writeFileSync(path.join(folder, file), `${file}\n`);
  const daemon = await startHided(fixture, name);
  // The second tab is made behind the first, which the page then shows with the keyboard in its first pane, where the agent opens files from.
  const second = await withControl(daemon, async (control) => {
    const made = await newTab(control, fixture, "second");
    await focusTab(control, fixture, fixture.tab);
    await focusPane(control, fixture.panes[0]);
    return made;
  });
  const sent = countSent(page);
  await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
  await enterWorkspace(page, "fixture");
  return { page, fixture, daemon, second, sent };
}

async function openFile(lane: Lane, paneId: string, name: string): Promise<void> {
  const hide = path.resolve("..", "target", "debug", fixtureExecutable("hide"));
  const ran = await runInPane(lane.fixture, lane.daemon, paneId, name, { env: { HIDE_STATE_DIR: lane.daemon.stateDir }, argv: [hide, "file", "open", name] });
  expect(ran.status, ran.stderr).toBe(0);
}

/**
 * Opens `name` from the pane the operator watches: it waits on this tab's
 * desk with the lane closed, counted on the documents button (PRD
 * lane-desk-pair-view B2, D-29), and the button stands the desk beside the
 * panes (D-50).
 */
async function openOnDesk(lane: Lane, name: string): Promise<void> {
  const page = lane.page;
  await openFile(lane, lane.fixture.panes[0]!, name);
  await expect(railTool(page, "documents")).toBeEnabled();
  await expect(screen(page)).toHaveAttribute("data-lane-mode", "closed");
  await railTool(page, "documents").click();
  await expect(screen(page)).toHaveAttribute("data-lane-mode", "docked");
}

const resizes = (lane: Lane) => lane.sent.get("terminal_resize") ?? 0;

test("an opened file stands beside the panes, each terminal resizing once as the desk comes, goes or its divider lands, stays through a terminal click, and Escape puts it away with its document (PRD right-lane B1, B3, B4; lane-two-columns B6, B11, B17; lane-desk-pair-view D-50) @platform", async ({ page }) => {
  test.setTimeout(120_000);
  const lane = await startLane(page, "lane-overlay");
  try {
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "closed");
    await expect(page.locator("[data-lane-tool]")).toHaveCount(5);
    await expect(railTool(page, "documents")).toBeDisabled();
    await expect(pane(page, lane.fixture.panes[0]!)).toBeVisible();
    await quietFor(page, 500, "the first fit has landed before the lane opens");
    const shown = lane.fixture.panes.length;
    let before = resizes(lane);
    // Each terminal follows its new width once, and nothing follows.
    const resizedOnce = async (what: string) => {
      await expect.poll(() => resizes(lane)).toBe(before + shown);
      await quietFor(page, 300, what);
      expect(resizes(lane)).toBe(before + shown);
      before = resizes(lane);
    };

    // The open waits on the desk with the lane closed, and the documents
    // button stands it beside the panes (PRD lane-desk-pair-view B2, D-29, D-50).
    await openOnDesk(lane, "a.md");
    await expect(screen(page)).toHaveAttribute("data-lane-view", "documents");
    await expect(screen(page)).toHaveAttribute("data-lane-shows", "none");
    await expect(front(page)).toHaveAttribute("aria-label", /\/a\.md/);
    await expect(railTool(page, "documents")).toHaveAttribute("aria-pressed", "true");
    await expect(page.locator("[data-lane-header]")).toHaveCount(0);
    await resizedOnce("the desk stood up");
    await screenshot(page, "right-lane-overlay");

    // The divider at the desk's left edge widens it by what it is dragged,
    // moving only a guide meanwhile, and lands once on release (B17, D-50).
    const laneBox = (await page.locator('[data-lane-frame="docked"]').boundingBox())!;
    const grip = page.locator('[data-lane-frame="docked"] [data-lane-divider="lane"]');
    await expect(grip).toHaveAttribute("aria-valuenow", String(Math.round(laneBox.width)));
    const gripBox = (await grip.boundingBox())!;
    await page.mouse.move(gripBox.x + gripBox.width / 2, gripBox.y + 200);
    await page.mouse.down();
    await page.mouse.move(gripBox.x + gripBox.width / 2 - 120, gripBox.y + 200, { steps: 6 });
    await expect(page.locator("[data-lane-guide]")).toHaveCount(1);
    expect(resizes(lane)).toBe(before);
    await page.mouse.up();
    await expect(page.locator("[data-lane-guide]")).toHaveCount(0);
    // The pointer lands on a whole pixel, so the width grows by the drag within a pixel.
    const widened = () => page.locator('[data-lane-frame="docked"]').boundingBox().then((box) => Math.round(box!.width));
    await expect.poll(async () => Math.abs((await widened()) - (laneBox.width + 120))).toBeLessThanOrEqual(1);
    await expect(grip).toHaveAttribute("aria-valuenow", String(await widened()));
    await resizedOnce("the divider landed");
    const wide = await widened();
    await screenshot(page, "right-lane-documents-wide");

    // A tool floats at the right edge at a width of its own with the desk on
    // its left at its width, the two one layer over the panes, so no
    // terminal resizes; the column divider trades width between the two, and
    // the tool's × puts the desk back in its place; the trade's landing
    // narrows the desk's place, each terminal following once (B14, B17, D-50).
    const desk = () => documentArea(page).boundingBox().then((box) => Math.round(box!.width));
    await railTool(page, "project").click();
    await expect(page.locator("[data-lane-header]")).toContainText("fixture");
    await expect(screen(page)).toHaveAttribute("data-lane-shows", "project");
    await expect(screen(page)).toHaveAttribute("data-lane-view", "documents");
    const memory = () => page.locator('[data-lane-column="tool"]').boundingBox().then((box) => Math.round(box!.width));
    await expect.poll(memory).toBe(400);
    expect(Math.abs((await desk()) - wide)).toBeLessThanOrEqual(1);
    await screenshot(page, "right-lane-project-beside");
    const columnGrip = page.locator('[data-lane-divider="column"]');
    const columnBox = (await columnGrip.boundingBox())!;
    await page.mouse.move(columnBox.x + columnBox.width / 2, columnBox.y + 200);
    await page.mouse.down();
    await page.mouse.move(columnBox.x + columnBox.width / 2 - 60, columnBox.y + 200, { steps: 6 });
    await page.mouse.up();
    await expect.poll(memory).toBe(460);
    await page.locator('[data-lane-tool-close="project"]').click();
    await expect(screen(page)).toHaveAttribute("data-lane-shows", "none");
    await expect.poll(widened).toBe(wide - 60);
    await resizedOnce("the trade's landing moved the desk's place once");

    // A terminal click takes the keyboard and leaves the desk up: only the
    // operator takes it down (PRD right-lane B4).
    await pane(page, lane.fixture.panes[0]!).click();
    await expect(pane(page, lane.fixture.panes[0]!).locator(".xterm-helper-textarea")).toBeFocused();
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "docked");
    await expect(front(page)).toHaveAttribute("aria-label", /\/a\.md/);

    // Escape in the lane closes it as ⌘E does; the keyboard goes back to the pane.
    // In the editor the first Escape is the editor's, which lets Tab leave it
    // (PRD lane-desk-pair-view B30), so the lane closes from the tab strip.
    await documentArea(page).locator(".cm-content").click();
    await page.keyboard.press("Escape");
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "docked");
    await front(page).focus();
    await page.keyboard.press("Escape");
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "closed");
    await expect(pane(page, lane.fixture.panes[0]!).locator(".xterm-helper-textarea")).toBeFocused();
    await expect(page.locator('[data-lane-badge="1"]')).toBeVisible();
    await resizedOnce("the desk went away");

    // Escape on the rail closes it too (the press leaves the keyboard on the
    // desk's front tab, whose Escape closes it as well: lane-desk.spec.ts).
    await railTool(page, "documents").click();
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "docked");
    await railTool(page, "documents").focus();
    await page.keyboard.press("Escape");
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "closed");

    // The rail's documents button, pressed while they show, folds them; with
    // no tool column the lane has nothing left and closes (B11, B12).
    await railTool(page, "documents").click();
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "docked");
    await railTool(page, "documents").click();
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "closed");
    await expect(page.locator('[data-lane-badge="1"]')).toBeVisible();
    await railTool(page, "documents").click();
    await expect(screen(page)).toHaveAttribute("data-lane-view", "documents");
    await expect(front(page)).toHaveAttribute("aria-label", /\/a\.md/);
  } finally {
    lane.daemon.stop();
    lane.fixture.stop();
  }
});

test("the rail draws the viewing column's buttons over a rule and one divider between the Project's tools and the checkout's, and every tool column's header keeps its scope icon (D-37, B2, B3, B54)", async ({ page }) => {
  test.setTimeout(120_000);
  const lane = await startLane(page, "lane-scope");
  try {
    // The documents' button, a rule, then one short rule where the Project's tools end and the checkout's begin.
    const rail = await page.locator("[data-tool-rail] > [data-lane-tool], [data-tool-rail] > [data-tool-rail-divider]").evaluateAll((nodes) => nodes.map((node) => node.getAttribute("data-lane-tool") ?? `divider:${node.getAttribute("data-tool-rail-divider")}`));
    expect(rail).toEqual(["documents", "divider:view", "project", "memory", "divider:true", "explorer", "changes"]);
    // The Project tool's head names the project and where it is, with ⚙ and × (PRD lane-two-columns B26; lane-desk-pair-view B16).
    await showTool(page, "project");
    const projectHeader = page.locator('aside[data-lane="project"] [data-lane-header]');
    await expect(projectHeader.locator('[data-lane-scope-kind="project"]')).toContainText("fixture");
    await expect(projectHeader.getByRole("button")).toHaveCount(2);
    await expect(projectHeader.locator('[data-lane-tool-close="project"]')).toBeVisible();
    // The Hide AI tool's head names the project beside its brain, with the switch for all of Hide AI, ⚙ and × (PRD hide-ai-suggestions B1, B3).
    await showTool(page, "memory");
    const hideAiHeader = page.locator('aside[data-lane="memory"] [data-lane-header]');
    await expect(hideAiHeader.locator('[data-lane-scope-kind="project"]')).toContainText(/Hide AI\s*· fixture/);
    await expect(hideAiHeader.locator("svg.lucide-brain")).toHaveCount(1);
    await expect(hideAiHeader.getByRole("switch")).toHaveCount(1);
    await expect(hideAiHeader.getByRole("button")).toHaveCount(2);
    await expect(hideAiHeader.locator('[data-lane-tool-close="memory"]')).toBeVisible();
    // Every checkout tool reads `name · scope`, with decoration only, and ends with one ×.
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
    lane.fixture.stop();
  }
});

test("closing the last view closes the lane when no tool shows, hands the keyboard to the pane, and the lane chord brings the Explorer back (B12, B15)", async ({ page }) => {
  test.setTimeout(120_000);
  const lane = await startLane(page, "lane-last-view");
  try {
    await openOnDesk(lane, "a.md");
    await expect(front(page)).toHaveAttribute("aria-label", /\/a\.md/);
    await expect(screen(page)).toHaveAttribute("data-lane-shows", "none");

    await documentArea(page).locator(".cm-content").click();
    await page.keyboard.press(chord("close_tab"));
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "closed");
    await expect(page.locator('[data-lane="explorer"]')).toHaveCount(0);
    await expect(pane(page, lane.fixture.panes[0]!).locator(".xterm-helper-textarea")).toBeFocused();
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
    lane.fixture.stop();
  }
});

test("the Explorer floats beside the standing desk without narrowing it, stays through a tool switch and the last view closing, and its chord closes the column alone (B4, B5, B7, B8, B12, B14; D-50) @platform", async ({ page }) => {
  test.setTimeout(120_000);
  const lane = await startLane(page, "lane-explorer");
  try {
    await openOnDesk(lane, "a.md");
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

    // Picking a file opens it floating beside the same Explorer (B6, D-50);
    // the chord closes the Explorer column alone, and the desk stands (B14).
    await page.locator('[data-lane="explorer"]').evaluate((column) => { (column as HTMLElement).dataset.probe = "kept"; });
    await page.locator('[data-explorer-row$="/b.md"]').dblclick();
    await expect(front(page)).toHaveAttribute("aria-label", /\/b\.md/);
    await expect(page.locator('[data-lane="explorer"][data-probe="kept"]')).toBeVisible();
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "overlay");
    await page.locator('[data-lane-frame] [data-view-area="true"] .cm-content').click();
    await page.keyboard.press(chord("toggle_lane_explorer"));
    await expect(page.locator('[data-lane="explorer"]')).toHaveCount(0);
    await expect(screen(page)).toHaveAttribute("data-lane-view", "documents");
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "docked");
    await expect(front(page)).toHaveAttribute("aria-label", /\/b\.md/);
    // The chord opens it again beside the document.
    await page.keyboard.press(chord("toggle_lane_explorer"));
    await expect(page.locator('[data-lane="explorer"] [data-explorer]')).toBeVisible();
    await expect(screen(page)).toHaveAttribute("data-lane-view", "documents");
  } finally {
    lane.daemon.stop();
    lane.fixture.stop();
  }
});

test("expand takes the whole body over the standing desk and returns on Escape without resizing a terminal (PRD right-lane B8; lane-two-columns B15; D-50) @platform", async ({ page }) => {
  test.setTimeout(120_000);
  const lane = await startLane(page, "lane-expand");
  try {
    await openOnDesk(lane, "a.md");
    await quietFor(page, 500, "the standing desk has settled");
    const before = resizes(lane);
    await page.locator("[data-lane-more]").click();
    await page.locator('[data-lane-menu-item="expand"]').click();
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "expanded");
    const row = (await page.locator("[data-lane-row]").boundingBox())!;
    expect(Math.abs((await page.locator("[data-lane-frame]").boundingBox())!.width - row.width)).toBeLessThanOrEqual(1);
    await screenshot(page, "right-lane-expanded");
    await page.locator('[data-lane-frame] [data-view-area="true"] .cm-content').click();
    await page.keyboard.press("Escape");
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "docked");
    // ⇧⌘↩, pressed in the document, expands the lane and back; no pane enters focus mode (D-57).
    await page.keyboard.press(chord("toggle_zoom"));
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "expanded");
    await page.keyboard.press(chord("toggle_zoom"));
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "docked");
    await quietFor(page, 500, "no terminal was fitted again while the lane expanded");
    expect(resizes(lane)).toBe(before);
    expect(lane.sent.get("pair_view") ?? 0).toBe(0);
  } finally {
    lane.daemon.stop();
    lane.fixture.stop();
  }
});

test("the rail shows icons alone and names each tool in its hint in four languages and both themes, and Tab reaches each tool with its pressed state (B23)", async ({ page }) => {
  test.setTimeout(120_000);
  const lane = await startLane(page, "lane-rail");
  try {
    await openOnDesk(lane, "a.md");
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
    lane.fixture.stop();
  }
});
