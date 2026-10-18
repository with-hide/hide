import { expect, test, type Page } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { fixtureExecutable } from "./platform-fixture";
import { runInPane, startHerdr, type HerdrFixture } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { chord } from "./chords";
import { quietFor } from "./wait";
import { countSent, enterWorkspace, screenshot, showTool } from "./wire";

// PRD right-lane: one lane beside or over Agent Views, a rail of four tools,
// one pin, the file tree as a column of the files tool. Each test starts one
// stack and asserts one contract.

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

test("an opened file comes up over Agent Views without resizing a terminal, stays through a terminal click, and Escape puts it away with its document (B1, B3, B4, B16) @platform", async ({ page }) => {
  test.setTimeout(120_000);
  const lane = await startLane(page, "lane-overlay");
  try {
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "closed");
    await expect(page.locator("[data-lane-tool]")).toHaveCount(4);
    await expect(railTool(page, "files")).toHaveAttribute("aria-pressed", "false");
    await expect(pane(page, lane.herdr.panes[0]!)).toBeVisible();
    await quietFor(page, 500, "the first fit has landed before the lane opens");
    const before = resizes(lane);

    await openFile(lane, lane.herdr.panes[0]!, "a.md");
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "overlay");
    await expect(front(page)).toHaveAttribute("aria-label", /\/a\.md/);
    await expect(railTool(page, "files")).toHaveAttribute("aria-pressed", "true");
    await expect(page.locator("[data-lane-header]")).toContainText("Files");
    await screenshot(page, "right-lane-overlay");

    // The divider at the overlay's left edge widens the lane by what it is
    // dragged, lands once on release, and resizes no terminal (B6).
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
    await screenshot(page, "right-lane-files-wide");

    // Every other tool keeps a width of its own: the wide document leaves
    // Memory at the default, and Files comes back at its width (B6).
    await railTool(page, "memory").click();
    await expect(page.locator("[data-lane-header]")).toContainText("Memory");
    await expect.poll(widened).toBe(Math.round(laneBox.width));
    await screenshot(page, "right-lane-memory-own-width");
    // Memory's divider moves Memory's width alone.
    const memoryGrip = (await grip.boundingBox())!;
    await page.mouse.move(memoryGrip.x + memoryGrip.width / 2, memoryGrip.y + 200);
    await page.mouse.down();
    await page.mouse.move(memoryGrip.x + memoryGrip.width / 2 - 60, memoryGrip.y + 200, { steps: 6 });
    await page.mouse.up();
    await expect.poll(async () => Math.abs((await widened()) - (laneBox.width + 60))).toBeLessThanOrEqual(1);
    await railTool(page, "files").click();
    await expect(page.locator("[data-lane-header]")).toContainText("Files");
    await expect.poll(widened).toBe(wide);
    await expect(pane(page, lane.herdr.panes[0]!)).toBeVisible();
    expect(resizes(lane)).toBe(before);

    // A terminal click takes the keyboard and leaves the unpinned lane up:
    // only the operator takes it down (B4).
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
    await railTool(page, "files").click();
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "overlay");
    await page.keyboard.press("Escape");
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "hidden");

    // The rail's icon on the tool it shows closes the lane.
    await railTool(page, "files").click();
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "overlay");
    await railTool(page, "files").click();
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "closed");
  } finally {
    lane.daemon.stop();
    lane.herdr.stop();
  }
});

test("the rail draws one divider between the Project's tools and the checkout's, and each header says its scope behind an icon (D-37, B54)", async ({ page }) => {
  test.setTimeout(120_000);
  const lane = await startLane(page, "lane-scope");
  try {
    // One short rule where the Project's tools end and the checkout's begin.
    const rail = await page.locator("[data-tool-rail] > [data-lane-tool], [data-tool-rail] > [data-tool-rail-divider]").evaluateAll((nodes) => nodes.map((node) => node.getAttribute("data-lane-tool") ?? "divider"));
    expect(rail).toEqual(["memory", "prs", "divider", "files", "changes"]);
    // The header reads `name · scope` as text, with the icon as decoration only.
    for (const [tool, name, icon] of [["memory", "Memory", "folder"], ["prs", "PRs", "folder"], ["files", "Files", "git-branch"], ["changes", "Changes", "git-branch"]] as const) {
      await showTool(page, tool);
      const header = page.locator("[data-lane-header]");
      await expect(header.locator(`svg.lucide-${icon}[aria-hidden="true"]`)).toHaveCount(1);
      await expect(header.locator("svg.lucide-folder, svg.lucide-git-branch")).toHaveCount(1);
      const text = (await header.locator("[data-lane-scope]").textContent())!.replace(/\s+/g, " ").trim();
      expect(text, tool).toMatch(new RegExp(`^${name} · \\S`));
    }
  } finally {
    lane.daemon.stop();
    lane.herdr.stop();
  }
});

test("the pin docks the lane once, keeps it through a terminal click, a tool and an Agent tab, and a restart brings it back docked (B5, B7, B15, B22, B24) @platform", async ({ page }) => {
  test.setTimeout(150_000);
  const lane = await startLane(page, "lane-pin");
  try {
    await openFile(lane, lane.herdr.panes[0]!, "a.md");
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "overlay");
    await quietFor(page, 500, "the overlay has settled");
    const before = resizes(lane);
    const agents = await page.locator("[data-agent-area]").boundingBox();

    await page.locator("[data-lane-pin]").click();
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "docked");
    await expect(page.locator("[data-lane-pin]")).toHaveAttribute("aria-pressed", "true");
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
    await railTool(page, "prs").click();
    await expect(page.locator("[data-lane]")).toHaveAttribute("data-lane", "prs");
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "docked");

    // Another Agent tab keeps the lane and adds its view to the checkout's one list.
    await railTool(page, "files").click();
    const tab = (id: string) => page.locator(`[data-agent-tab-bar] [data-tab="${id}"]`);
    await tab(lane.second.tab).click();
    await openFile(lane, lane.second.pane, "b.md");
    await expect(front(page)).toHaveAttribute("aria-label", /\/b\.md/);
    await tab(lane.herdr.tab).click();
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "docked");
    await expect(front(page)).toHaveAttribute("aria-label", /\/a\.md/);
    await expect(page.locator('[data-view-tab-bar] [role="tab"]')).toHaveCount(2);

    lane.daemon = await lane.daemon.restart();
    await page.goto("about:blank");
    await page.goto(`${lane.daemon.origin}/?probe=1#token=${lane.daemon.token}`);
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "docked");

    // Unpinned, the lane goes back over Agent Views, and a restart brings it
    // back there: the operator never took it down.
    await page.locator("[data-lane-pin]").click();
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "overlay");
    lane.daemon = await lane.daemon.restart();
    await page.goto("about:blank");
    await page.goto(`${lane.daemon.origin}/?probe=1#token=${lane.daemon.token}`);
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "overlay");
    await expect(railTool(page, "files")).toHaveAttribute("aria-pressed", "true");
  } finally {
    lane.daemon.stop();
    lane.herdr.stop();
  }
});

test("closing the last view with the tree off closes the lane, hands the keyboard to the pane, and the lane chord brings Files back (B12)", async ({ page }) => {
  test.setTimeout(120_000);
  const lane = await startLane(page, "lane-last-view");
  try {
    await openFile(lane, lane.herdr.panes[0]!, "a.md");
    await expect(front(page)).toHaveAttribute("aria-label", /\/a\.md/);
    await expect(page.locator("[data-lane-tree-toggle]")).toHaveAttribute("aria-pressed", "false");

    await documentArea(page).locator(".cm-content").click();
    await page.keyboard.press(chord("close_tab"));
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "closed");
    await expect(page.locator("[data-lane-tree]")).toHaveCount(0);
    await expect(pane(page, lane.herdr.panes[0]!).locator(".xterm-helper-textarea")).toBeFocused();

    await page.keyboard.press(chord("toggle_lane"));
    await expect(screen(page)).toHaveAttribute("data-lane-shows", "files");
    await expect(page.locator('[data-lane-tree="only"]')).toBeVisible();
  } finally {
    lane.daemon.stop();
    lane.herdr.stop();
  }
});

test("the tree opens beside the document without narrowing it, stays through a tool switch, and fills the lane once the last view closes (B9-B12) @platform", async ({ page }) => {
  test.setTimeout(120_000);
  const lane = await startLane(page, "lane-tree");
  try {
    await openFile(lane, lane.herdr.panes[0]!, "a.md");
    await expect(front(page)).toHaveAttribute("aria-label", /\/a\.md/);
    const width = (await documentArea(page).boundingBox())!.width;
    const toggle = page.locator("[data-lane-tree-toggle]");
    await expect(toggle).toHaveAttribute("aria-pressed", "false");

    await toggle.click();
    await expect(page.locator('[data-lane-tree="beside"]')).toBeVisible();
    await expect(page.locator("[data-lane-tree-toggle]")).toHaveAttribute("aria-pressed", "true");
    expect(Math.abs((await documentArea(page).boundingBox())!.width - width)).toBeLessThanOrEqual(1);
    await screenshot(page, "right-lane-tree");

    await railTool(page, "changes").click();
    await railTool(page, "files").click();
    await expect(page.locator('[data-lane-tree="beside"]')).toBeVisible();

    // The last view closed leaves the lane on the files tool, the tree alone.
    await page.getByRole("button", { name: "Close view a.md" }).click();
    await expect(page.locator('[data-lane-tree="only"]')).toBeVisible();
    await expect(page.locator("[data-lane-tree-hint]")).toBeVisible();
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "overlay");
    await screenshot(page, "right-lane-tree-only");
    // The chord has nothing to turn off while the tree is all the lane shows.
    const views = lane.sent.get("workspace_view") ?? 0;
    await page.keyboard.press(chord("toggle_file_tree"));
    await quietFor(page, 300, "the chord sent nothing");
    expect(lane.sent.get("workspace_view") ?? 0).toBe(views);

    // Picking a file opens it beside the same tree; the chord turns it off.
    await page.locator("[data-lane-tree]").evaluate((tree) => { (tree as HTMLElement).dataset.probe = "kept"; });
    await page.locator('[data-explorer-row$="/b.md"]').dblclick();
    await expect(front(page)).toHaveAttribute("aria-label", /\/b\.md/);
    await expect(page.locator('[data-lane-tree="beside"][data-probe="kept"]')).toBeVisible();
    await page.locator('[data-lane-frame] [data-view-area="true"] .cm-content').click();
    await page.keyboard.press(chord("toggle_file_tree"));
    await expect(page.locator("[data-lane-tree]")).toHaveCount(0);
  } finally {
    lane.daemon.stop();
    lane.herdr.stop();
  }
});

test("expand takes the whole body and returns on Escape without resizing a terminal (B8) @platform", async ({ page }) => {
  test.setTimeout(120_000);
  const lane = await startLane(page, "lane-expand");
  try {
    await openFile(lane, lane.herdr.panes[0]!, "a.md");
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "overlay");
    await quietFor(page, 500, "the overlay has settled");
    const before = resizes(lane);
    await page.locator("[data-lane-expand]").click();
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
    await expect(railTool(page, "files")).toHaveAttribute("aria-pressed", "true");
    const rail = page.locator("[data-tool-rail]");
    // The rail draws no text of its own: a tool's name is its accessible name
    // and the hint that opens on hover, with the tool's scope and chord.
    await expect(rail).toHaveText("");
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
    await railTool(page, "files").focus();
    await page.keyboard.press("Tab");
    await expect(railTool(page, "changes")).toBeFocused();
    await expect(railTool(page, "changes")).toHaveAttribute("aria-pressed", "false");
    await page.keyboard.press("Enter");
    await expect(railTool(page, "changes")).toHaveAttribute("aria-pressed", "true");
    await expect(railTool(page, "files")).toHaveAttribute("aria-pressed", "false");
  } finally {
    lane.daemon.stop();
    lane.herdr.stop();
  }
});
