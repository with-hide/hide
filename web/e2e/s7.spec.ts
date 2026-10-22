// The S7 View areas on an isolated pinned Herdr and hided (PRD
// workspace-views-layout B1-B22): a click previews a file in the last used
// area and a double click, Keep open or the first edit pins it (B1-B3); Open
// to the side shows one document twice with one buffer (B4, B5, B20); tab
// drags reorder, move and split with a preview and land once (B6-B8);
// splits, dividers, menus and the area commands work from the keyboard (B9, B10,
// B18, B20); the tab menu and the caps say what cannot be done (B11, B19); a
// narrow window changes only what is drawn (B12, B13); a restart brings the
// layout back and a broken or older file is handled (B14-B17); and the
// rail and tools keep working beside several areas (B22; PRD right-lane).

import { expect, test, type Locator, type Page } from "@playwright/test";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { toPage } from "../../desktop/src/main/wirePath";
import { startHerdr, type HerdrFixture } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { bindChordlessCommand, countSent, enterWorkspace, pinState, screenshot, setLanePinned, showDocuments, showExplorer, showProjectRow, showTool } from "./wire";
import { chord } from "./chords";
import { composeKorean } from "./ime";
import { onDisk, quietFor, unchangedForFrames } from "./wait";

/** `--size-rail`: the always-shown device rail takes this much of a window that measured its areas without one. */
const RAIL = 48;

test.describe.configure({ timeout: 180_000 });
// A click or a key that cannot happen fails the flow in seconds, not at the test's end.
test.use({ actionTimeout: 15_000 });

type SentEvent = { kind: string; payload: Record<string, unknown> };

type Stack = {
  herdr: HerdrFixture;
  daemon: Daemon;
  /** The resolved checkout root in the core's wire spelling. */
  root: string;
  sent: Map<string, number>;
  last: Map<string, Record<string, unknown>>;
  /** Every event the page sent, in order. */
  events: SentEvent[];
  /** The `workspace_views.*` diagnostic kinds the page received in any snapshot frame. */
  diagnostics: Set<string>;
};

/** The daemon's state directory, where `workspace-views.json` lives. */
function stateDir(daemon: Daemon): string {
  return path.join(path.dirname(daemon.home), "hide");
}

function viewsFile(daemon: Daemon): string {
  return path.join(stateDir(daemon), "workspace-views.json");
}

/** Records every event the page sends and every Views-file diagnostic it receives. */
function recordWire(page: Page, events: SentEvent[], diagnostics: Set<string>): void {
  page.on("websocket", (ws) => {
    ws.on("framesent", (frame) => {
      try {
        const event = JSON.parse(String(frame.payload)) as { kind?: string; payload?: Record<string, unknown> };
        if (event.kind) events.push({ kind: event.kind, payload: event.payload ?? {} });
      } catch {
        /* the handshake is not an event */
      }
    });
    ws.on("framereceived", (frame) => {
      const text = String(frame.payload);
      if (!text.includes('"diagnostics"')) return;
      for (const match of text.matchAll(/"kind":"(workspace_views\.[a-z_]+)"/g)) {
        if (match[1]) diagnostics.add(match[1]);
      }
    });
  });
}

async function open(page: Page, daemon: Daemon): Promise<void> {
  await page.goto(`${daemon.origin}/#token=${daemon.token}`);
}

/** Pins the lane, so it docks beside Agent Views with a divider (PRD right-lane B5, B6). */
async function pinLane(page: Page): Promise<void> {
  await setLanePinned(page, true);
  await expect(page.locator("[data-workspace-screen]")).toHaveAttribute("data-lane-mode", "docked");
}

/** Drags the docked lane's divider to `x`, where a minimum stops it, and waits for the release to land. */
async function dragLaneTo(page: Page, x: (body: Box) => number): Promise<void> {
  await pinLane(page);
  const divider = page.locator('[data-lane-divider="lane"]');
  const box = await boxOf(divider);
  const body = await boxOf(page.locator("[data-lane-row]"));
  await page.mouse.move(box.x + box.width / 2, box.y + 200);
  await page.mouse.down();
  await page.mouse.move(x(body), box.y + 200, { steps: 6 });
  const guide = await boxOf(page.locator("[data-lane-guide]"));
  await page.mouse.up();
  await expect(page.locator("[data-lane-guide]")).toHaveCount(0);
  // The divider lands where its guide was once the core has the width, so
  // nothing measured after this moves under a drag still landing.
  await expect.poll(async () => Math.abs((await boxOf(divider)).x - guide.x)).toBeLessThanOrEqual(1);
}

/** Narrows the pinned lane to its minimum width. */
async function narrowLane(page: Page): Promise<void> {
  await dragLaneTo(page, (body) => body.x + body.width - 10);
}

/**
 * Gives the pinned lane all the width Agent Views can spare, where their
 * minimum stops it, so several View areas fit side by side.
 */
async function widenLane(page: Page): Promise<void> {
  await dragLaneTo(page, (body) => body.x + 10);
}

/**
 * The isolated stack with `files` written into the fixture checkout, and the
 * page on that Workspace (a first run goes through Main and the Overview)
 * with the files tool and its tree shown in the lane.
 */
async function startStack(
  page: Page,
  label: string,
  files: Record<string, string>,
  options: { beforeOpen?: () => Promise<void>; prepare?: (checkout: string, herdr: HerdrFixture) => void } = {},
): Promise<Stack> {
  const herdr = await startHerdr();
  let daemon: Daemon | null = null;
  try {
    for (const [name, contents] of Object.entries(files)) {
      fs.mkdirSync(path.dirname(path.join(herdr.root, "fixture", name)), { recursive: true });
      fs.writeFileSync(path.join(herdr.root, "fixture", name), contents);
    }
    options.prepare?.(path.join(herdr.root, "fixture"), herdr);
    daemon = await startHided(herdr, label);
    const last = new Map<string, Record<string, unknown>>();
    const sent = countSent(page, last);
    const events: SentEvent[] = [];
    const diagnostics = new Set<string>();
    recordWire(page, events, diagnostics);
    await options.beforeOpen?.();
    await open(page, daemon);
    await enterWorkspace(page, "fixture");
    await showExplorer(page);
    return { herdr, daemon, root: toPage(path.join(fs.realpathSync(herdr.root), "fixture")), sent, last, events, diagnostics };
  } catch (error) {
    daemon?.stop();
    herdr.stop();
    throw error;
  }
}

function stopStack(stack: Stack): void {
  stack.daemon.stop();
  stack.herdr.stop();
}

/**
 * What the View areas show, in tree order: each area's tabs in order, `>`
 * before the area's shown tab, `*` after a preview, and `@` before the area
 * the operator works in. `(b.txt >c.txt*) | @(>d.txt)` is two areas.
 */
async function shape(page: Page): Promise<string> {
  return page.evaluate(() =>
    [...document.querySelectorAll<HTMLElement>("[data-view-area-id]")]
      .map((area) => {
        const tabs = [...area.querySelectorAll<HTMLElement>('[data-view-tab-bar] [role="tab"][data-display]')].map((tab) => {
          const identity = tab.getAttribute("aria-label") ?? "";
          const name = identity.replace(/^[^:]+: /, "").replace(/ · .*$/, "").split("/").pop() ?? "";
          return `${tab.getAttribute("aria-selected") === "true" ? ">" : ""}${name}${tab.dataset.preview === "true" ? "*" : ""}`;
        });
        return `${area.dataset.activeArea === "true" ? "@" : ""}(${tabs.join(" ")})`;
      })
      .join(" | "),
  );
}

/**
 * Double-clicks each row in turn. A file is shown when its read lands, and
 * two reads land in either order, so each file is on screen, kept open and
 * selected in the area in use before the next one is opened.
 */
async function keepOpenInTurn(page: Page, names: string[], row: (name: string) => Locator): Promise<void> {
  for (const name of names) {
    await row(name).dblclick();
    await expect.poll(async () => /@\([^)]*?>(\S+?)[ )]/.exec(await shape(page))?.[1]).toBe(name);
  }
}

function area(page: Page, index: number): Locator {
  return page.locator("[data-view-area-id]").nth(index);
}

/** A View tab by its file name, in one area or anywhere. */
function tab(scope: Page | Locator, name: string): Locator {
  return scope.locator(`[data-view-tab-bar] [role="tab"][data-display][aria-label*="/${name}"]`);
}

/** The shown document's editable text in one area. */
function editor(page: Page, index: number): Locator {
  return area(page, index).locator("[data-editor-body] .cm-content");
}

function explorerRow(page: Page, stack: Stack, name: string): Locator {
  return page.locator(`[data-explorer-row="${stack.root}/${name}"]`);
}

/** Opens a View tab's own menu (right-click) and picks one item. */
async function tabMenu(scope: Page | Locator, page: Page, name: string, item: string): Promise<void> {
  await tab(scope, name).click({ button: "right" });
  if (item.startsWith("split_") || item.startsWith("move_")) await page.locator(`[data-menu-group="${item.split("_")[0]}"]`).hover();
  await page.locator(`[role="menu"] [data-menu-item="${item}"]`).click();
}

/** Every `view_layout` event sent so far. */
function viewEvents(stack: Stack): SentEvent[] {
  return stack.events.filter((event) => event.kind === "view_layout");
}

/** Every View action the page sent named the Workspace in front: this checkout on this machine (contract 4.1). */
function expectFrontWorkspaceOnEveryViewEvent(stack: Stack): void {
  for (const event of viewEvents(stack)) expect(event.payload.workspace).toEqual({ device_id: stack.daemon.node, path: stack.root });
}

test("a click previews in the last used area, a pin keeps a view, and a shown file is focused rather than opened again", async ({ page }) => {
  await page.setViewportSize({ width: 1920, height: 1080 });
  const names = ["a", "b", "c", "d", "e", "f"];
  const stack = await startStack(page, "s7-place", Object.fromEntries(names.map((name) => [`${name}.txt`, `${name} line\n`])));
  try {
    const row = (name: string) => explorerRow(page, stack, name);
    const workspace = page.locator("[data-workspace-screen]");

    // A single click previews in the one area, italic, and the next single
    // click replaces that preview in place (B1).
    await row("a.txt").click();
    await expect(workspace).toHaveAttribute("data-lane-view", "documents");
    await expect.poll(() => shape(page)).toBe("@(>a.txt*)");
    await expect(tab(page, "a.txt").getByText("a.txt")).toHaveCSS("font-style", "italic");
    await expect(tab(page, "a.txt")).toHaveAttribute("aria-label", /· Preview$/);
    await row("b.txt").click();
    await expect.poll(() => shape(page)).toBe("@(>b.txt*)");

    // A double click on the tab pins it (B2).
    await tab(page, "b.txt").dblclick();
    await expect.poll(() => shape(page)).toBe("@(>b.txt)");
    await expect(tab(page, "b.txt").getByText("b.txt")).toHaveCSS("font-style", "normal");
    expect(stack.sent.get("view_layout.keep_open")).toBe(1);

    // Keep open from the tab's menu pins the next preview (B2).
    await row("c.txt").click();
    await expect.poll(() => shape(page)).toBe("@(b.txt >c.txt*)");
    await tabMenu(page, page, "c.txt", "keep_open");
    await expect.poll(() => shape(page)).toBe("@(b.txt >c.txt)");

    // A double click on the Explorer row opens it pinned (B2).
    await row("d.txt").dblclick();
    await expect.poll(() => shape(page)).toBe("@(b.txt c.txt >d.txt)");

    // A second area: the split view goes right and that area is now the one in use.
    await tabMenu(page, page, "c.txt", "split_right");
    await expect.poll(() => shape(page)).toBe("(b.txt >d.txt) | @(>c.txt)");

    // The next click previews in the area used last, and the other area's
    // pinned tabs stay (B1).
    await row("a.txt").click();
    await expect.poll(() => shape(page)).toBe("(b.txt >d.txt) | @(c.txt >a.txt*)");
    await tab(area(page, 0), "b.txt").click();
    await expect.poll(() => shape(page)).toBe("@(>b.txt d.txt) | (c.txt >a.txt*)");
    await row("e.txt").click();
    await expect.poll(() => shape(page)).toBe("@(b.txt d.txt >e.txt*) | (c.txt >a.txt*)");
    await screenshot(page, "s7-place-two-previews");

    // The first edit pins the preview: a dirty document is never a preview,
    // so the next click adds a preview beside it instead of replacing it (B2).
    const opensBefore = stack.sent.get("file_open") ?? 0;
    await editor(page, 0).click();
    await page.keyboard.press("ControlOrMeta+Home");
    await page.keyboard.press("End");
    await page.keyboard.type(" edited");
    await expect.poll(() => shape(page)).toBe("@(b.txt d.txt >e.txt) | (c.txt >a.txt*)");
    await row("f.txt").click();
    await expect.poll(() => shape(page)).toBe("@(b.txt d.txt e.txt >f.txt*) | (c.txt >a.txt*)");
    expect(stack.sent.get("file_open")).toBe(opensBefore + 1);
    await expect.poll(() => onDisk(path.join(stack.root, "e.txt"))).toBe("e line edited\n");

    // Opening a file another area shows focuses that view: no tab is added (B3).
    await row("c.txt").click();
    await expect.poll(() => shape(page)).toBe("(b.txt d.txt e.txt >f.txt*) | @(>c.txt a.txt*)");

    // With two views of one file, reopening it picks the one used last (B3).
    await tab(area(page, 0), "d.txt").click();
    await row("d.txt").click({ button: "right" });
    await page.locator('[data-explorer-menu] [data-menu-item="open-beside"]').click();
    await expect.poll(() => shape(page)).toBe("(b.txt >d.txt e.txt f.txt*) | @(c.txt a.txt* >d.txt)");
    await tab(area(page, 0), "d.txt").click();
    await tab(area(page, 1), "c.txt").click();
    await expect.poll(() => shape(page)).toBe("(b.txt >d.txt e.txt f.txt*) | @(>c.txt a.txt* d.txt)");
    await row("d.txt").click();
    await expect.poll(() => shape(page)).toBe("@(b.txt >d.txt e.txt f.txt*) | (>c.txt a.txt* d.txt)");
    await tab(area(page, 1), "d.txt").click();
    await tab(area(page, 0), "b.txt").click();
    await expect.poll(() => shape(page)).toBe("@(>b.txt d.txt e.txt f.txt*) | (c.txt a.txt* >d.txt)");
    await row("d.txt").click();
    await expect.poll(() => shape(page)).toBe("(>b.txt d.txt e.txt f.txt*) | @(c.txt a.txt* >d.txt)");
    await screenshot(page, "s7-place-reopen-last-focused");
    expectFrontWorkspaceOnEveryViewEvent(stack);
  } finally {
    stopStack(stack);
  }
});

/** Three hundred numbered lines, so a view can scroll and each line is findable. */
const LONG = Array.from({ length: 300 }, (_, index) => `line ${String(index + 1).padStart(3, "0")}`).join("\n") + "\n";

// @platform: Korean composed through the browser's input-method path, and a save refused by file mode.
test("Open to the side shows one document twice: edits and Korean input reach both, each keeps its place, and closing keeps unsaved text", { tag: "@platform" }, async ({ page }) => {
  await page.setViewportSize({ width: 1920, height: 1080 });
  const stack = await startStack(page, "s7-beside", { "shared.txt": LONG, "other.txt": "other\n", "keep.txt": "keep on disk\n" });
  const shared = `${stack.root}/shared.txt`;
  const keep = `${stack.root}/keep.txt`;
  try {
    const row = (name: string) => explorerRow(page, stack, name);
    await row("shared.txt").click();
    await expect.poll(() => shape(page)).toBe("@(>shared.txt*)");

    // Open to the side makes a second, pinned view in a new area on the right (B4).
    await row("shared.txt").click({ button: "right" });
    await page.locator('[data-explorer-menu] [data-menu-item="open-beside"]').click();
    await expect.poll(() => shape(page)).toBe("(>shared.txt*) | @(>shared.txt)");
    expect(stack.last.get("file_open")).toMatchObject({ path: shared, beside: true, preview: false });
    const [left, right] = [editor(page, 0), editor(page, 1)];
    await expect(left).toContainText("line 001");
    await expect(right).toContainText("line 001");

    // An edit in one view shows in the other, and the first edit pins every
    // view of the document (B2, B4).
    await right.click();
    await page.keyboard.press("ControlOrMeta+Home");
    await page.keyboard.press("End");
    await page.keyboard.type("-B");
    await expect(left).toContainText("line 001-B");
    await expect.poll(() => shape(page)).toBe("(>shared.txt) | @(>shared.txt)");

    // Korean text inserted in one view and composed in the other reads the
    // same in both, once, and reaches the file once (B20, D-13).
    await left.click();
    await page.keyboard.press("ControlOrMeta+Home");
    await page.keyboard.press("End");
    await page.keyboard.insertText(" 한글");
    await expect(right).toContainText("line 001-B 한글");
    // The right view's cursor stayed where it was, before the text the left
    // view inserted; End takes it past that text.
    await right.focus();
    await page.keyboard.press("End");
    await composeKorean(page, [
      ["ㅇ", "아", "안"],
      ["ㄴ", "녀", "녕"],
    ]);
    await expect(right).toContainText("line 001-B 한글안녕");
    await expect(left).toContainText("line 001-B 한글안녕");
    await expect.poll(() => onDisk(shared)?.split("\n")[0], { timeout: 10_000 }).toBe("line 001-B 한글안녕");
    await screenshot(page, "s7-beside-korean");

    // Each view keeps its own scroll and cursor: the left one scrolls to the
    // end and types there, the right one types at its own cursor (B4).
    const scrollers = [area(page, 0).locator(".cm-scroller"), area(page, 1).locator(".cm-scroller")];
    await scrollers[0]!.hover();
    await page.mouse.wheel(0, 20_000);
    await expect.poll(() => scrollers[0]!.evaluate((node) => node.scrollTop)).toBeGreaterThan(1000);
    await left.getByText("line 300").click();
    await page.keyboard.press("End");
    await page.keyboard.type("-A");
    await right.focus();
    await page.keyboard.type("-C");
    await expect(right).toContainText("line 001-B 한글안녕-C");
    await expect(left).toContainText("line 300-A");
    expect(await scrollers[1]!.evaluate((node) => node.scrollTop)).toBe(0);
    expect(await scrollers[0]!.evaluate((node) => node.scrollTop)).toBeGreaterThan(1000);
    await expect.poll(() => onDisk(shared), { timeout: 10_000 }).toBe(
      LONG.replace("line 001\n", "line 001-B 한글안녕-C\n").replace("line 300\n", "line 300-A\n"),
    );

    // A save that cannot land keeps the document unsaved in both views.
    // Closing one of them asks nothing and leaves the text and its unsaved
    // mark in the other (B5, D-03).
    await scrollers[0]!.hover();
    await page.mouse.wheel(0, -20_000);
    await expect.poll(() => scrollers[0]!.evaluate((node) => node.scrollTop)).toBe(0);
    fs.chmodSync(shared, 0o444);
    await right.focus();
    await page.keyboard.type("-D");
    await expect(left).toContainText("-C-D");
    await expect(area(page, 0).locator("[data-editor-save-state]")).toBeVisible({ timeout: 10_000 });
    await expect(tab(area(page, 0), "shared.txt")).toContainText("●");
    const closesBefore = stack.sent.get("view_layout.close") ?? 0;
    await tab(area(page, 1), "shared.txt").hover();
    await tab(area(page, 1), "shared.txt").getByRole("button", { name: /Close view/ }).click();
    await expect.poll(() => shape(page)).toBe("@(>shared.txt)");
    expect(stack.sent.get("view_layout.close")).toBe(closesBefore + 1);
    await expect(page.locator("[data-notice]")).toHaveCount(0);
    await expect(editor(page, 0)).toContainText("-C-D");
    await expect(tab(page, "shared.txt")).toContainText("●");
    await expect(area(page, 0).locator("[data-editor-dirty]")).toBeVisible();

    // Closing the last view carries the unsaved text as its save: while the
    // save cannot land the view stays, with its text, and is not called
    // saved; once it can, the save lands and the view closes (B5).
    await tab(page, "shared.txt").hover();
    await tab(page, "shared.txt").getByRole("button", { name: /Close view/ }).click();
    const pending = () => (stack.last.get("view_layout.close")?.pending_save as { contents_utf8?: string } | undefined)?.contents_utf8?.split("\n")[0];
    await expect.poll(pending).toBe("line 001-B 한글안녕-C-D");
    await quietFor(page, 1500, "the unsaved close stays pending while the save cannot land");
    await expect.poll(() => shape(page)).toBe("@(>shared.txt)");
    await expect(editor(page, 0)).toContainText("-C-D");
    await expect(area(page, 0).locator("[data-editor-save-state]")).toBeVisible();
    expect(fs.readFileSync(shared, "utf8").split("\n")[0]).toBe("line 001-B 한글안녕-C");
    fs.chmodSync(shared, 0o644);
    await tab(page, "shared.txt").hover();
    await tab(page, "shared.txt").getByRole("button", { name: /Close view/ }).click();
    // The last view gone, the View areas leave and the panel keeps the Explorer.
    await expect(page.locator("[data-view-area]")).toHaveCount(0, { timeout: 10_000 });
    await expect.poll(() => onDisk(shared)?.split("\n")[0]).toBe("line 001-B 한글안녕-C-D");

    // Unsaved text this page never loaded (a background view after a reload)
    // is closed only by the operator's explicit Don't save, which sends
    // `discard` and leaves the file as it was on disk (B5, contract 4.1).
    await row("keep.txt").dblclick();
    await expect.poll(() => shape(page)).toBe("@(>keep.txt)");
    fs.chmodSync(keep, 0o444);
    await editor(page, 0).click();
    await page.keyboard.press("ControlOrMeta+Home");
    await page.keyboard.type("never saved ");
    await expect(area(page, 0).locator("[data-editor-save-state]")).toBeVisible({ timeout: 10_000 });
    await row("other.txt").dblclick();
    await expect.poll(() => shape(page)).toBe("@(keep.txt >other.txt)");
    // A fresh page, not a fragment navigation of this one.
    await page.goto("about:blank");
    await open(page, stack.daemon);
    await expect.poll(() => shape(page), { timeout: 20_000 }).toBe("@(keep.txt >other.txt)");
    await showDocuments(page);
    await tab(page, "keep.txt").hover();
    await tab(page, "keep.txt").getByRole("button", { name: /Close view/ }).click();
    const dontSave = page.locator("[data-notice-dont-save]");
    await expect(dontSave).toBeVisible();
    await expect(page.locator("[data-notice]")).toContainText("keep.txt has unsaved changes");
    await expect.poll(() => shape(page)).toBe("@(keep.txt >other.txt)");
    await screenshot(page, "s7-beside-dont-save");
    await dontSave.click();
    await expect.poll(() => shape(page)).toBe("@(>other.txt)");
    expect(stack.last.get("view_layout.close")).toMatchObject({ discard: true });
    expect(fs.readFileSync(keep, "utf8")).toBe("keep on disk\n");

    expectFrontWorkspaceOnEveryViewEvent(stack);
  } finally {
    for (const file of [shared, keep]) if (fs.existsSync(file)) fs.chmodSync(file, 0o644);
    stopStack(stack);
  }
});

const BESIDE_TOO_NARROW = "This view area is too narrow to open a second view beside it.";

test("Open to the side from the only area is refused with its reason until that area has room to split", async ({ page }) => {
  await page.setViewportSize({ width: 1920, height: 1080 });
  const stack = await startStack(page, "s7-beside-room", { "a.txt": "a\n" }, { prepare: gitCheckout });
  try {
    // In File Views at its narrowest, where the one area is too narrow to halve.
    const row = explorerRow(page, stack, "a.txt");
    await row.click();
    await expect.poll(() => shape(page)).toBe("@(>a.txt*)");
    await narrowLane(page);
    await expect.poll(async () => (await boxOf(area(page, 0))).width).toBeLessThan(450);

    // The one area cannot be halved, so Open to the side stays listed,
    // disabled with the reason, in the Explorer's menu (B4, B9, D-06).
    await row.click({ button: "right" });
    const item = page.locator('[data-explorer-menu] [data-menu-item="open-beside"]');
    await expect(item).toBeDisabled();
    await expect(item).toContainText(BESIDE_TOO_NARROW);
    await page.keyboard.press("Escape");

    // History's row menu opens at the pointer, whole, with the same item
    // disabled and its reason: a menu drawn inside the list's positioned
    // rows landed off the pointer and was clipped out of sight (B4, B9).
    await showTool(page, "changes");
    const historyRow = page.locator('[data-history-group="working"][data-history-path="a.txt"]');
    const rowBox = await boxOf(historyRow);
    const pointer = { x: Math.round(rowBox.x + 16), y: Math.round(rowBox.y + rowBox.height / 2) };
    await page.mouse.click(pointer.x, pointer.y, { button: "right" });
    const besideDiff = page.locator('[data-history-menu="a.txt"] [data-menu-item="open_beside"]');
    await expect(besideDiff).toBeDisabled();
    await expect(besideDiff).toContainText(BESIDE_TOO_NARROW);
    const menuBox = await boxOf(page.locator('[role="menu"][data-history-menu="a.txt"]'));
    // The menu opens at the pointer; near the window's edge it moves left (or
    // up) only as far as it takes to stay whole, so how far depends on the
    // menu's width, which depends on the system's fonts.
    const viewport = page.viewportSize()!;
    expect(Math.abs(menuBox.x - Math.min(pointer.x, viewport.width - menuBox.width))).toBeLessThanOrEqual(2);
    expect(Math.abs(menuBox.y - Math.min(pointer.y, viewport.height - menuBox.height))).toBeLessThanOrEqual(2);
    const itemBox = await boxOf(besideDiff);
    // A disabled item takes no pointer, so the point hits the menu drawn under it.
    const drawn = await page.evaluate(({ x, y }) => document.elementFromPoint(x, y)?.closest('[role="menu"]')?.getAttribute("data-history-menu") ?? null, {
      x: itemBox.x + itemBox.width / 2,
      y: itemBox.y + itemBox.height / 2,
    });
    expect(drawn).toBe("a.txt");
    expect(itemBox.x).toBeGreaterThanOrEqual(menuBox.x);
    expect(itemBox.y + itemBox.height).toBeLessThanOrEqual(menuBox.y + menuBox.height);
    await page.keyboard.press("Escape");
    expect(stack.events.filter((event) => event.kind === "file_open" && event.payload.beside === true)).toHaveLength(0);
    expect(stack.events.filter((event) => event.kind === "changes_select" && event.payload.beside === true)).toHaveLength(0);

    // With room (the documents widened), the same item opens the second view to
    // the right. The documents keep a width of their own, so they are widened.
    await showExplorer(page);
    await widenLane(page);
    await expect.poll(async () => (await boxOf(area(page, 0))).width).toBeGreaterThan(450);
    await row.click({ button: "right" });
    await expect(item).toBeEnabled();
    await item.click();
    await expect.poll(() => shape(page)).toBe("(>a.txt*) | @(>a.txt)");
  } finally {
    stopStack(stack);
  }
});

type Box = { x: number; y: number; width: number; height: number };

async function boxOf(locator: Locator): Promise<Box> {
  const box = await locator.boundingBox();
  if (!box) throw new Error("not on screen");
  return box;
}

/** Every View area's rectangle, to tell whether anything resized. */
async function areaBoxes(page: Page): Promise<Box[]> {
  return page.locator("[data-view-area-id]").evaluateAll((areas) =>
    areas.map((node) => {
      const rect = node.getBoundingClientRect();
      return { x: rect.x, y: rect.y, width: rect.width, height: rect.height };
    }),
  );
}

/**
 * Presses a View tab and drags it with real pointer events past the
 * activation distance to `to`; `during` runs with the pointer still down,
 * and `release: false` leaves the release (or an Escape) to the caller.
 */
async function dragTab(page: Page, from: Locator, to: { x: number; y: number }, during?: () => Promise<void>, release = true): Promise<void> {
  // A tab past the end of a narrow strip is scrolled to first, as the operator would.
  await from.scrollIntoViewIfNeeded();
  const start = await boxOf(from);
  const x = start.x + start.width / 2;
  const y = start.y + start.height / 2;
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x + 12, y + 2, { steps: 3 });
  await page.mouse.move(to.x, to.y, { steps: 10 });
  await during?.();
  if (release) await page.mouse.up();
}

test("dragging a tab reorders, moves or splits once on a valid drop and leaves everything as it was otherwise", async ({ page }) => {
  await page.setViewportSize({ width: 1920 + RAIL, height: 1080 });
  const stack = await startStack(page, "s7-drag", { "a.txt": "a\n", "b.txt": "b\n", "c.txt": "c\n" });
  try {
    // The drop points below are measured in a docked lane wide enough for
    // two areas that can each split again.
    await keepOpenInTurn(page, ["a.txt", "b.txt", "c.txt"], (name) => explorerRow(page, stack, name));
    await expect.poll(() => shape(page)).toBe("@(a.txt b.txt >c.txt)");
    await widenLane(page);
    const moves = () => stack.sent.get("view_layout.move") ?? 0;
    const splits = () => stack.sent.get("view_layout.split") ?? 0;
    const resizes = () => stack.sent.get("terminal_resize") ?? 0;
    const drop = page.locator("[data-view-drop]");
    const floating = page.locator("[data-view-drag-tab]");

    // Within its own tab bar: an insertion line, the floating tab, nothing
    // resized while it lasts, then the order changes once with no copy (B6).
    const before = await areaBoxes(page);
    // The widened lane's terminal resizes go out after its divider lands.
    await unchangedForFrames(page, resizes);
    const terminalResizes = resizes();
    const [bTab, cTab] = [await boxOf(tab(page, "b.txt")), await boxOf(tab(page, "c.txt"))];
    await dragTab(page, tab(page, "a.txt"), { x: (bTab.x + bTab.width / 2 + cTab.x + cTab.width / 2) / 2, y: bTab.y + bTab.height / 2 }, async () => {
      await expect(page.locator('[data-view-drop="bar"]')).toBeVisible();
      await expect(floating).toBeVisible();
      expect(await areaBoxes(page)).toEqual(before);
      expect(moves()).toBe(0);
      await screenshot(page, "s7-drag-insertion-line");
    });
    await expect.poll(() => shape(page)).toBe("@(b.txt >a.txt c.txt)");
    expect(moves()).toBe(1);
    expect(stack.last.get("view_layout.move")).toMatchObject({ action: "move", index: 1 });
    await expect(drop).toHaveCount(0);
    await expect(floating).toHaveCount(0);

    // Onto the right edge of the content: the half the new area would take is
    // washed with one short label before the release; the drop splits once (B7).
    const body = await boxOf(area(page, 0).locator("[data-view-body]"));
    await dragTab(page, tab(page, "c.txt"), { x: body.x + body.width * 0.92, y: body.y + body.height / 2 }, async () => {
      const overlay = page.locator('[data-view-drop="right"]');
      await expect(overlay).toBeVisible();
      await expect(overlay).toHaveText("Split right");
      await expect(drop).toHaveCount(1);
      expect(await areaBoxes(page)).toEqual(before);
      expect(splits()).toBe(0);
      await screenshot(page, "s7-drag-split-overlay");
    });
    await expect.poll(() => shape(page)).toBe("(b.txt >a.txt) | @(>c.txt)");
    expect(splits()).toBe(1);
    expect(stack.last.get("view_layout.split")).toMatchObject({ action: "split", edge: "right" });

    // Into the other area's tab bar: a move into that area, no split, no copy (B6).
    const cRight = await boxOf(tab(area(page, 1), "c.txt"));
    await dragTab(page, tab(area(page, 0), "b.txt"), { x: cRight.x + cRight.width + 20, y: cRight.y + cRight.height / 2 }, async () => {
      await expect(page.locator('[data-view-drop="bar"]')).toBeVisible();
    });
    await expect.poll(() => shape(page)).toBe("(>a.txt) | @(c.txt >b.txt)");
    expect(moves()).toBe(2);
    expect(splits()).toBe(1);

    // Escape ends the drag with nothing sent, and the release after it changes nothing (B8).
    const settled = await areaBoxes(page);
    const aBox = await boxOf(tab(area(page, 0), "a.txt"));
    const bRight = await boxOf(tab(area(page, 1), "b.txt"));
    await dragTab(
      page,
      tab(area(page, 0), "a.txt"),
      { x: bRight.x + bRight.width + 20, y: bRight.y + bRight.height / 2 },
      async () => {
        await expect(page.locator('[data-view-drop="bar"]')).toBeVisible();
        await page.keyboard.press("Escape");
        await expect(drop).toHaveCount(0);
        await expect(floating).toHaveCount(0);
      },
      false,
    );
    await page.mouse.up();
    // A release outside the window lands nothing (B8).
    await dragTab(page, tab(area(page, 1), "c.txt"), { x: 1920 + RAIL + 40, y: 500 }, async () => {
      await expect(floating).toBeVisible();
    });
    await expect(drop).toHaveCount(0);
    await expect(floating).toHaveCount(0);
    // A drop on something that is not a View area (the file tree) lands nothing (B8).
    const explorer = await boxOf(page.locator('[data-lane="explorer"]'));
    await dragTab(page, tab(area(page, 1), "c.txt"), { x: explorer.x + explorer.width / 2, y: explorer.y + explorer.height / 2 }, async () => {
      await expect(drop).toHaveCount(0);
      await expect(floating).toBeVisible();
    });
    // The only view of an area cannot split its own area: no overlay, the
    // forbidden cursor and the reason on the floating tab (B8, B9).
    await dragTab(page, tab(area(page, 0), "a.txt"), { x: aBox.x + 300, y: body.y + body.height / 2 }, async () => {
      await expect(drop).toHaveCount(0);
      await expect(page.locator("html")).toHaveAttribute("data-view-drag", "forbidden");
      await expect(floating).toContainText("This is the only view in its area.");
      await screenshot(page, "s7-drag-forbidden");
    });
    await expect(page.locator("html")).not.toHaveAttribute("data-view-drag", /.*/);
    await expect.poll(() => shape(page)).toBe("(>a.txt) | @(c.txt >b.txt)");
    expect(await areaBoxes(page)).toEqual(settled);
    expect(moves()).toBe(2);
    expect(splits()).toBe(1);
    // No drag resized a terminal: the Agent area kept its size throughout (B7, B19).
    expect(resizes()).toBe(terminalResizes);
    expectFrontWorkspaceOnEveryViewEvent(stack);
  } finally {
    stopStack(stack);
  }
});

test("a drag or a tab menu begun on one Workspace ends when another client moves the front to another Workspace", async ({ page }) => {
  await page.setViewportSize({ width: 1920, height: 1080 });
  const stack = await startStack(page, "s7-front-moves", { "a.txt": "a\n", "c.txt": "c\n" }, {
    prepare: (checkout, herdr) => {
      const beta = path.join(path.dirname(checkout), "beta");
      fs.mkdirSync(beta, { recursive: true });
      for (const name of ["b1.txt", "b2.txt"]) fs.writeFileSync(path.join(beta, name), `${name}\n`);
      herdr.run(["workspace", "create", "--cwd", beta, "--label", "beta", "--env", `PATH=${herdr.fixturePath}`, "--no-focus"]);
    },
  });
  const other = await page.context().newPage();
  try {
    const betaRoot = `${path.posix.dirname(stack.root)}/beta`;
    const choose = async (target: Page, project: string) => {
      await (await showProjectRow(target, target.locator("[data-project]", { hasText: project }).locator("[data-checkout]").first())).click();
    };
    // Each Workspace numbers its own views, so both hold a1 with d2 and d3.
    await keepOpenInTurn(page, ["a.txt", "c.txt"], (name) => explorerRow(page, stack, name));
    await expect.poll(() => shape(page)).toBe("@(a.txt >c.txt)");
    await choose(page, "beta");
    // A Workspace chosen for the first time starts with its lane closed.
    await expect(page.locator("[data-workspace-screen]")).toHaveAttribute("data-lane-mode", "closed");
    await showExplorer(page);
    await keepOpenInTurn(page, ["b1.txt", "b2.txt"], (name) => page.locator(`[data-explorer-row="${betaRoot}/${name}"]`));
    await expect.poll(() => shape(page)).toBe("@(b1.txt >b2.txt)");
    await choose(page, "fixture");
    await expect.poll(() => shape(page)).toBe("@(a.txt >c.txt)");
    await showDocuments(page);
    const betaLayout = () => JSON.stringify(storedWorkspace(stack.daemon, betaRoot)?.view_layout ?? null);
    await expect.poll(betaLayout).toContain("b2.txt");
    const stored = betaLayout();
    const changes = () => viewEvents(stack).filter((event) => event.payload.action !== "focus" && event.payload.action !== "focus_area").length;
    const before = changes();
    await open(other, stack.daemon);
    await expect(other.locator("[data-main-screen]").or(other.locator("[data-workspace-screen]"))).toBeVisible({ timeout: 20_000 });

    // A drag toward fixture's right edge while the other client moves the
    // front to beta: the drag ends with the tree it began on, and its release
    // sends nothing that would split beta's d3 (contract 4.1, B8).
    const target = await boxOf(area(page, 0));
    await dragTab(page, tab(page, "c.txt"), { x: target.x + target.width - 12, y: target.y + target.height / 2 }, async () => {
      await expect(page.locator('[data-view-drop="right"]')).toBeVisible();
      await choose(other, "beta");
      await expect.poll(() => shape(page)).toBe("@(b1.txt >b2.txt)");
      await expect(page.locator("[data-view-drop]")).toHaveCount(0);
    });
    await expect.poll(() => shape(page)).toBe("@(b1.txt >b2.txt)");
    expect(changes()).toBe(before);
    expect(betaLayout()).toBe(stored);

    // A tab menu open on fixture's c.txt closes when the front moves the same way.
    await choose(other, "fixture");
    await expect.poll(() => shape(page)).toBe("@(a.txt >c.txt)");
    await showDocuments(page);
    await tab(page, "c.txt").click({ button: "right" });
    await expect(page.locator('[role="menu"]')).toBeVisible();
    await choose(other, "beta");
    await expect.poll(() => shape(page)).toBe("@(b1.txt >b2.txt)");
    await expect(page.locator('[role="menu"]')).toHaveCount(0);
    expect(changes()).toBe(before);
    expect(betaLayout()).toBe(stored);
  } finally {
    await other.close();
    stopStack(stack);
  }
});

/**
 * Moves the keyboard with Tab or Shift+Tab until `target` has it. An editor
 * keeps Tab for indenting; Escape first lets the next Tab leave it, as
 * CodeMirror offers.
 */
async function tabTo(page: Page, target: Locator, key: "Tab" | "Shift+Tab"): Promise<void> {
  for (let step = 0; step < 40; step += 1) {
    if (await target.evaluate((node) => node === document.activeElement)) return;
    if (await page.evaluate(() => Boolean(document.activeElement?.closest(".cm-editor")))) await page.keyboard.press("Escape");
    await page.keyboard.press(key);
  }
  await expect(target).toBeFocused();
}

/** Opens the focused tab's menu with ⇧F10 and chooses `item` with the arrow keys and Enter. */
async function menuByKeyboard(page: Page, item: string): Promise<void> {
  await page.keyboard.press("Shift+F10");
  const focusItem = async (target: Locator) => {
    await expect(target).toBeEnabled();
    for (let step = 0; step < 20 && !(await target.evaluate((node) => node === document.activeElement)); step += 1) {
      const focused = () => page.evaluate(() => document.activeElement?.outerHTML ?? "");
      const before = await focused();
      await page.keyboard.press("ArrowDown");
      await expect.poll(focused).not.toBe(before);
    }
    await expect(target).toBeFocused();
  };
  if (item.startsWith("split_") || item.startsWith("move_")) {
    await focusItem(page.locator(`[data-menu-group="${item.split("_")[0]}"]`));
    await page.keyboard.press("ArrowRight");
  }
  await focusItem(page.locator(`[role="menu"] [data-menu-item="${item}"]`));
  await page.keyboard.press("Enter");
  await expect(page.locator('[role="menu"]')).toHaveCount(0);
}

test("splits, moves, resizes, focus and closes run from the tab menu, area commands and chords by keyboard, each landing once", async ({ page }) => {
  await page.setViewportSize({ width: 2560, height: 1080 });
  // The split frame is delivered twice at the transport, as a resend would
  // be; the core must split once (B18).
  let pageSplits = 0;
  let duplicated = 0;
  const duplicateFirstSplit = () =>
    page.routeWebSocket(/\/ws$/, (ws) => {
      const server = ws.connectToServer();
      ws.onMessage((message) => {
        server.send(message);
        if (typeof message !== "string" || !message.includes('"kind":"view_layout"') || !message.includes('"action":"split"')) return;
        pageSplits += 1;
        if (duplicated > 0) return;
        duplicated += 1;
        server.send(message);
      });
    });
  const stack = await startStack(page, "s7-keys", { "a.txt": "a\n", "b.txt": "b\n", "c.txt": "c\n", "d.txt": "d\n" }, { beforeOpen: duplicateFirstSplit });
  try {
    await keepOpenInTurn(page, ["a.txt", "b.txt", "c.txt", "d.txt"], (name) => explorerRow(page, stack, name));
    await widenLane(page);
    const workspace = page.locator("[data-workspace-screen]");
    await expect(workspace).toHaveAttribute("data-lane-view", "documents");
    await expect.poll(() => shape(page)).toBe("@(a.txt b.txt c.txt >d.txt)");
    const count = (key: string) => stack.sent.get(key) ?? 0;
    // The area commands have no default chord (PRD cmdk-navigation D-05): the
    // three this flow runs are bound in Settings first, on keys free on every system (not Alt+Shift+L, Memory's, or Alt+Shift+U, Show task list's).
    await bindChordlessCommand(page, "focus_previous_view_area", "Alt+Shift+KeyJ");
    await bindChordlessCommand(page, "focus_next_view_area", "Alt+Shift+KeyX");
    await bindChordlessCommand(page, "grow_view_area", "Alt+Shift+KeyY");
    await expect.poll(() => shape(page)).toBe("@(a.txt b.txt c.txt >d.txt)");

    // Split right from the active tab's menu: one event, delivered twice, one new area;
    // the keyboard follows the view into it (B9, B18, B20). The tabs come after the
    // lane's divider; before it are Agent Views, whose terminals keep Tab.
    await page.locator('[data-lane-divider="lane"]').focus();
    await tabTo(page, tab(page, "d.txt"), "Tab");
    await menuByKeyboard(page, "split_right");
    await expect.poll(() => shape(page)).toBe("(a.txt b.txt >c.txt) | @(>d.txt)");
    // The page sent one split; the wire carried it to the core twice.
    expect(pageSplits).toBe(1);
    expect(duplicated).toBe(1);
    expect(count("view_layout.split")).toBe(2);
    await quietFor(page, 500, "no further split follows");
    await expect.poll(() => shape(page)).toBe("(a.txt b.txt >c.txt) | @(>d.txt)");
    await expect(editor(page, 1)).toBeFocused();

    // Focus moves between areas from the bound chord, and the keyboard goes along (B20).
    await page.keyboard.press("Alt+Shift+KeyJ");
    await expect.poll(() => shape(page)).toBe("@(a.txt b.txt >c.txt) | (>d.txt)");
    await expect(editor(page, 0)).toBeFocused();
    expect(count("view_layout.focus_area")).toBe(1);

    // The tab menu from the keyboard: Split down, then Move right; the
    // emptied area goes and its neighbour takes its height (B9, B10, B11).
    await tabTo(page, tab(area(page, 0), "b.txt"), "Shift+Tab");
    await menuByKeyboard(page, "split_down");
    await expect.poll(() => shape(page)).toBe("(a.txt >c.txt) | @(>b.txt) | (>d.txt)");
    const [top, bottom] = [await boxOf(area(page, 0)), await boxOf(area(page, 1))];
    expect(bottom.y).toBeGreaterThan(top.y + top.height - 1);
    await tabTo(page, tab(area(page, 1), "b.txt"), "Shift+Tab");
    await menuByKeyboard(page, "move_right");
    await expect.poll(() => shape(page)).toBe("(a.txt >c.txt) | @(d.txt >b.txt)");
    const views = await boxOf(page.locator("[data-view-areas]"));
    await expect.poll(async () => Math.round((await boxOf(area(page, 0))).height)).toBe(Math.round(views.height));
    await screenshot(page, "s7-keys-after-move");

    // A divider drag moves a guide line, resizes nothing while it lasts,
    // and lands one resize on release (B9, B18).
    const divider = page.locator("[data-view-divider]");
    await expect(divider).toHaveCount(1);
    const ratio = Number(await divider.getAttribute("aria-valuenow"));
    const before = await areaBoxes(page);
    const line = await boxOf(divider);
    await page.mouse.move(line.x + line.width / 2, line.y + line.height / 2);
    await page.mouse.down();
    await page.mouse.move(line.x + 60, line.y + line.height / 2, { steps: 4 });
    await page.mouse.move(line.x + 150, line.y + line.height / 2, { steps: 6 });
    await expect(page.locator("[data-view-resize-guide]")).toBeVisible();
    expect(await areaBoxes(page)).toEqual(before);
    expect(count("view_layout.resize")).toBe(0);
    await page.mouse.up();
    await expect(page.locator("[data-view-resize-guide]")).toHaveCount(0);
    await expect.poll(() => count("view_layout.resize")).toBe(1);
    await expect.poll(async () => Number(await divider.getAttribute("aria-valuenow"))).toBeGreaterThan(ratio + 5);
    await expect.poll(async () => Math.round((await boxOf(area(page, 0))).width - before[0]!.width)).toBeGreaterThan(140);
    await quietFor(page, 300, "no second resize commit follows the first");
    expect(count("view_layout.resize")).toBe(1);

    // The focused divider moves one step per arrow key, one event each (B9, B20).
    // The walk starts at the lane's divider, before its areas; an agent
    // terminal ahead of it keeps Tab for its program.
    const dragged = Number(await divider.getAttribute("aria-valuenow"));
    await page.locator('[data-lane-divider="lane"]').focus();
    await tabTo(page, divider, "Tab");
    await page.keyboard.press("ArrowLeft");
    await expect.poll(async () => Number(await divider.getAttribute("aria-valuenow"))).toBe(dragged - 5);
    await page.keyboard.press("ArrowLeft");
    await expect.poll(async () => Number(await divider.getAttribute("aria-valuenow"))).toBe(dragged - 10);
    expect(count("view_layout.resize")).toBe(3);
    // Tabbing through the left area's view on the way made it the area in use (B20).
    await expect.poll(() => shape(page)).toBe("@(a.txt >c.txt) | (d.txt >b.txt)");

    // Grow and focus from the bound chords act on the active area; Close view
    // is the active tab's menu item (B20).
    await page.keyboard.press("Alt+Shift+KeyY");
    await expect.poll(async () => Number(await divider.getAttribute("aria-valuenow"))).toBe(dragged - 5);
    await page.keyboard.press("Alt+Shift+KeyX");
    await expect.poll(() => shape(page)).toBe("(a.txt >c.txt) | @(d.txt >b.txt)");
    await tabTo(page, tab(page, "b.txt"), "Shift+Tab");
    await menuByKeyboard(page, "close_view");
    await expect.poll(() => shape(page)).toBe("(a.txt >c.txt) | @(>d.txt)");
    await tabTo(page, tab(page, "d.txt"), "Shift+Tab");
    await menuByKeyboard(page, "close_view");
    await expect.poll(() => shape(page)).toBe("@(a.txt >c.txt)");
    await expect(divider).toHaveCount(0);
    await expect.poll(async () => Math.round((await boxOf(area(page, 0))).width)).toBe(Math.round(views.width));

    // Closing the last view closes the viewing column and leaves the Explorer
    // in its column, as part of the close (PRD lane-two-columns B12).
    const laneEvents = () => stack.events.filter((event) => event.kind === "workspace_view").length;
    const laneBefore = laneEvents();
    await page.locator('[data-lane-divider="lane"]').focus();
    await tabTo(page, tab(page, "c.txt"), "Tab");
    await menuByKeyboard(page, "close_view");
    await expect.poll(() => shape(page)).toBe("@(>a.txt)");
    await page.locator('[data-lane-divider="lane"]').focus();
    await tabTo(page, tab(page, "a.txt"), "Tab");
    await menuByKeyboard(page, "close_view");
    await expect(page.locator("[data-view-area]")).toHaveCount(0);
    await expect(page.locator('[data-lane="explorer"] [data-explorer]')).toBeVisible();
    await expect(workspace).toHaveAttribute("data-lane-view", "none");
    expect(laneEvents()).toBe(laneBefore);
    await screenshot(page, "s7-keys-closed");
    expectFrontWorkspaceOnEveryViewEvent(stack);
  } finally {
    stopStack(stack);
  }
});

type MenuRow = { id: string; label: string; reason: string | null; disabled: boolean };

/** The open menu's items as drawn: id, label, the reason under a disabled one. */
async function menuRows(page: Page): Promise<MenuRow[]> {
  const read = (locator: Locator) => locator.evaluateAll((items) => items.map((button) => ({
    id: button.getAttribute("data-menu-item") ?? button.getAttribute("data-menu-group") ?? "",
    label: button.querySelector("[data-menu-label]")?.textContent ?? "",
    reason: button.querySelector("[data-menu-reason]")?.textContent ?? null,
    disabled: button.getAttribute("aria-disabled") === "true",
  })));
  const rows = await read(page.locator('[role="menu"] [data-menu-item], [role="menu"] [data-menu-group]'));
  const expanded: MenuRow[] = [];
  for (const row of rows) {
    if ((row.id === "split" || row.id === "move") && !row.disabled) {
      await page.locator(`[data-menu-group="${row.id}"]`).focus();
      await page.keyboard.press("ArrowRight");
      const submenu = page.locator(`[data-menu-submenu="${row.id}"]`);
      await expect(submenu).toBeVisible();
      expanded.push(...await read(submenu.locator("[data-menu-item]")));
      await page.keyboard.press("Escape");
      await expect(submenu).toHaveCount(0);
    } else expanded.push(row);
  }
  return expanded;
}

async function tabMenuRows(page: Page, scope: Page | Locator, name: string): Promise<MenuRow[]> {
  await tab(scope, name).click({ button: "right" });
  await expect(page.locator('[role="menu"]')).toBeVisible();
  const rows = await menuRows(page);
  await page.keyboard.press("Escape");
  await expect(page.locator('[role="menu"]')).toHaveCount(0);
  return rows;
}

const SPLITS = ["split_right", "split_left", "split_up", "split_down"];

/** A drag from one tab to a point near the right edge of another area's content, released in place. */
async function dragToEdge(page: Page, from: Locator, target: Locator, edge: "right" | "down", during?: () => Promise<void>): Promise<void> {
  const body = await boxOf(target.locator("[data-view-body]"));
  const point = edge === "right" ? { x: body.x + body.width * 0.92, y: body.y + body.height / 2 } : { x: body.x + body.width / 2, y: body.y + body.height * 0.92 };
  await dragTab(page, from, point, during);
}

// A Workspace of sixty-five files with the first one previewed in a wide window.
async function startCaps(page: Page, name: string) {
  await page.setViewportSize({ width: 2560 + RAIL, height: 1080 });
  const files = Object.fromEntries(Array.from({ length: 65 }, (_, index) => [`f${String(index + 1).padStart(2, "0")}.txt`, `file ${index + 1}\n`]));
  const stack = await startStack(page, name, files);
  try {
    await explorerRow(page, stack, "f01.txt").click();
    await expect.poll(() => shape(page)).toBe("@(>f01.txt*)");
    await widenLane(page);
    return stack;
  } catch (error) {
    stopStack(stack);
    throw error;
  }
}
const layoutEvents = (stack: Stack) => viewEvents(stack).length;

/** Pins f02 to f10 in one area. The first click of each double click takes the preview's place, so the f01 preview gives way to f02 (B1). */
async function pinNine(page: Page, stack: Stack) {
  const names = Array.from({ length: 9 }, (_, index) => `f${String(index + 2).padStart(2, "0")}.txt`);
  await keepOpenInTurn(page, names, (name) => explorerRow(page, stack, name));
  await expect.poll(() => shape(page)).toBe("@(f02.txt f03.txt f04.txt f05.txt f06.txt f07.txt f08.txt f09.txt >f10.txt)");
}

/** Two areas: f10 on its own at the right, the rest at the left. */
async function pinNineAndSplit(page: Page, stack: Stack) {
  await pinNine(page, stack);
  await tabMenu(page, page, "f10.txt", "split_right");
  await expect.poll(() => shape(page)).toBe("(f02.txt f03.txt f04.txt f05.txt f06.txt f07.txt f08.txt >f09.txt) | @(>f10.txt)");
}

const areaWith = (page: Page, name: string) => page.locator("[data-view-area-id]").filter({ has: tab(page, name) });
const areaCount = (page: Page) => page.locator("[data-view-area-id]").count();

/** Four areas, three splits deep at the left. */
async function splitToDepth(page: Page, stack: Stack) {
  await pinNineAndSplit(page, stack);
  await tabMenu(area(page, 0), page, "f09.txt", "split_down");
  await tabMenu(area(page, 0), page, "f08.txt", "split_right");
  await expect.poll(() => areaCount(page)).toBe(4);
}

/** Six areas, by drops on other areas' edges. */
async function splitToSix(page: Page, stack: Stack) {
  await splitToDepth(page, stack);
  await dragToEdge(page, tab(area(page, 0), "f07.txt"), areaWith(page, "f10.txt"), "down");
  await expect.poll(() => areaCount(page)).toBe(5);
  await dragToEdge(page, tab(area(page, 0), "f06.txt"), areaWith(page, "f09.txt"), "right");
  await expect.poll(() => areaCount(page)).toBe(6);
}

test("a preview alone in its area offers its menu without the splits it cannot make, and opening it changes nothing", async ({ page }) => {
  const stack = await startCaps(page, "s7-caps-menu");
  try {
    // New tab, Keep open, four splits that cannot land (with the reason), the
    // path, Select in File Tree and Close view (a browser tab has no OS file
    // manager to reveal in); no Move without a neighbour and never a file
    // deletion (B11).
    const sentBefore = layoutEvents(stack);
    const alone = await tabMenuRows(page, page, "f01.txt");
    expect(alone.map((row) => row.id)).toEqual(["new_tab", "keep_open", "split", "copy_path", "select_in_tree", "close_view"]);
    expect(alone.map((row) => row.label)).toEqual(["New tab", "Keep open", "Split", "Copy path", "Select in File Tree", "Close view"]);
    expect(alone.find((row) => row.id === "split")).toMatchObject({ disabled: true, reason: "This is the only view in its area." });
    expect(layoutEvents(stack)).toBe(sentBefore);
    await tab(page, "f01.txt").click({ button: "right" });
    await expect(page.locator('[role="menu"]')).not.toContainText(/Trash|Delete/);
    await screenshot(page, "s7-caps-menu-preview");
    await page.keyboard.press("Escape");
  } finally { stopStack(stack); }
});

test("a pinned view has no Keep open and, beside another area, can move there", async ({ page }) => {
  const stack = await startCaps(page, "s7-caps-pinned");
  try {
    // This menu needs two pinned views beside a single pinned view. Confirm
    // each preview and Keep open action before preparing the next file.
    for (const name of ["f02.txt", "f03.txt", "f04.txt"]) {
      await explorerRow(page, stack, name).click();
      await expect(tab(page, name)).toHaveAttribute("data-preview", "true");
      await tabMenu(page, page, name, "keep_open");
      await expect(tab(page, name)).toHaveAttribute("data-preview", "false");
    }
    await tabMenu(page, page, "f04.txt", "split_right");
    await expect.poll(() => shape(page)).toBe("(f02.txt >f03.txt) | @(>f04.txt)");
    const left = await tabMenuRows(page, area(page, 0), "f02.txt");
    expect(left.map((row) => row.id)).toEqual(["new_tab", ...SPLITS, "move_right", "copy_path", "select_in_tree", "close_view"]);
    expect(left.every((row) => !row.disabled)).toBe(true);
    const right = await tabMenuRows(page, area(page, 1), "f04.txt");
    expect(right.map((row) => row.id)).toEqual(["new_tab", "split", "move_left", "copy_path", "select_in_tree", "close_view"]);
    expect(right.find((row) => row.id === "split")).toMatchObject({ disabled: true, reason: "This is the only view in its area." });
  } finally { stopStack(stack); }
});

test("a window too narrow to halve two side by side areas again refuses the split across and still offers the split down", async ({ page }) => {
  const stack = await startCaps(page, "s7-caps-room");
  try {
    // Room is a reason too (B9, D-06).
    await pinNineAndSplit(page, stack);
    await page.setViewportSize({ width: 1280, height: 1080 });
    await expect.poll(async () => (await boxOf(area(page, 0))).width).toBeLessThan(450);
    const cramped = await tabMenuRows(page, area(page, 0), "f02.txt");
    expect(cramped.find((row) => row.id === "split_right")).toMatchObject({ disabled: true, reason: "This view area is too narrow to split." });
    expect(cramped.find((row) => row.id === "split_down")).toMatchObject({ disabled: false });
  } finally { stopStack(stack); }
});

test("an area three splits deep cannot split again and says why", async ({ page }) => {
  const stack = await startCaps(page, "s7-caps-depth");
  try {
    // B9, B19
    await splitToDepth(page, stack);
    const deep = await tabMenuRows(page, area(page, 0), "f02.txt");
    expect(deep.find((row) => row.id === "split")).toMatchObject({ disabled: true, reason: "View areas can be split only 3 levels deep." });
  } finally { stopStack(stack); }
});

test("at six view areas every split is refused with its reason, in the menu and on a drag, and no view moves", async ({ page }) => {
  const stack = await startCaps(page, "s7-caps-areas");
  try {
    // B7, B9, B19
    await splitToSix(page, stack);
    const six = await shape(page);
    const splitsAtSix = stack.sent.get("view_layout.split") ?? 0;
    const capped = await tabMenuRows(page, area(page, 0), "f02.txt");
    expect(capped.find((row) => row.id === "split")).toMatchObject({ disabled: true, reason: "This Workspace already shows 6 view areas, the most it can." });
    await tab(area(page, 0), "f02.txt").click({ button: "right" });
    await screenshot(page, "s7-caps-area-limit-menu");
    await page.keyboard.press("Escape");
    await dragToEdge(page, tab(area(page, 0), "f05.txt"), areaWith(page, "f10.txt"), "down", async () => {
      await expect(page.locator("[data-view-drop]")).toHaveCount(0);
      await expect(page.locator("html")).toHaveAttribute("data-view-drag", "forbidden");
      await expect(page.locator("[data-view-drag-tab]")).toContainText("This Workspace already shows 6 view areas, the most it can.");
    });
    // The refused menu and drag changed nothing.
    const sixFocused = await shape(page);
    expect(stack.sent.get("view_layout.split") ?? 0).toBe(splitsAtSix);
    expect(sixFocused.replace(/[>@]/g, "")).toBe(six.replace(/[>@]/g, ""));
    expectFrontWorkspaceOnEveryViewEvent(stack);
  } finally { stopStack(stack); }
});

test("a checkout holds at most thirty-two views: opening one more closes its view looked at longest ago, and opening a shown file still moves to it", async ({ page }) => {
  await page.setViewportSize({ width: 2560 + RAIL, height: 1080 });
  const files = Object.fromEntries(Array.from({ length: 33 }, (_, index) => [`f${String(index + 1).padStart(2, "0")}.txt`, `file ${index + 1}\n`]));
  const stack = await startStack(page, "s7-caps-views", files);
  try {
    // B19, contract 2. Seeded as a schema 2 file, the File Views column
    // on with the Explorer: the core reads it through the lane migration,
    // so the cap holds across that path too.
    const seeded = {
      schema_version: 2,
      workspaces: [
        {
          device_id: stack.daemon.node,
          path: stack.root,
          views: true,
          tool: "explorer",
          tools: true,
          last_used_unix_ms: Date.now(),
          layout: {
            root: {
              area: {
                id: "a1",
                active: "d32",
                displays: Array.from({ length: 32 }, (_, index) => ({
                  id: `d${index + 1}`,
                  path: `${stack.root}/f${String(index + 1).padStart(2, "0")}.txt`,
                  kind: "file",
                  committed: null,
                  preview: false,
                  // f10 was looked at longest ago.
                  last_focused_unix_ms: index === 9 ? 1 : 1000 + index,
                })),
              },
            },
            active_area: "a1",
            next_id: 33,
          },
        },
      ],
    };
    stack.daemon = await stack.daemon.restart((dir) => fs.writeFileSync(path.join(dir, "workspace-views.json"), JSON.stringify(seeded)));
    await page.goto("about:blank");
    await open(page, stack.daemon);
    await expect(page.locator('[data-view-area-id] [role="tab"][data-view-state="open"]')).toHaveCount(32, { timeout: 30_000 });
    await showExplorer(page);
    // Explorer virtualizes this long list: reveal its last rows before clicking one.
    await page.getByRole("tree", { name: "Checkout files" }).hover();
    await page.mouse.wheel(0, 2000);
    await explorerRow(page, stack, "f33.txt").click();
    await expect(tab(page, "f33.txt")).toHaveAttribute("aria-selected", "true");
    await expect(tab(page, "f10.txt")).toHaveCount(0);
    await expect(page.locator('[data-view-area-id] [role="tab"]')).toHaveCount(32);
    await expect(page.locator("[data-notice]")).toHaveCount(0);
    await screenshot(page, "s7-caps-display-limit");
    await explorerRow(page, stack, "f12.txt").click();
    await expect(tab(page, "f12.txt")).toHaveAttribute("aria-selected", "true");
    await expect(page.locator("[data-notice]")).toHaveCount(0);
    await expect(page.locator('[data-view-area-id] [role="tab"]')).toHaveCount(32);
    expectFrontWorkspaceOnEveryViewEvent(stack);
  } finally {
    stopStack(stack);
  }
});

type StoredWorkspace = { device_id: string; path: string; open: boolean; documents: boolean; tool: string | null; pinned: boolean; document_width?: number; explorer_width?: number; view_layout: unknown };

/** This Workspace's entry in `workspace-views.json`, without the clock stamps a focus writes. */
function storedWorkspace(daemon: Daemon, root: string): StoredWorkspace | null {
  let text: string;
  try {
    text = fs.readFileSync(viewsFile(daemon), "utf8");
  } catch {
    return null;
  }
  const stored = JSON.parse(text, (key, value: unknown) => (key === "last_used_unix_ms" || key === "last_focused_unix_ms" ? undefined : value)) as {
    workspaces: StoredWorkspace[];
  };
  return stored.workspaces.find((entry) => entry.device_id === daemon.node && entry.path === root) ?? null;
}

test("a window too narrow to dock opens the pinned lane over the agents with one area and a way to the others, and stores none of it", async ({ page }) => {
  await page.setViewportSize({ width: 1920, height: 1080 });
  const stack = await startStack(page, "s7-narrow", { "a.txt": "a\n", "b.txt": "b\n" });
  try {
    // Two areas in the pinned lane beside the agents.
    await keepOpenInTurn(page, ["a.txt", "b.txt"], (name) => explorerRow(page, stack, name));
    await widenLane(page);
    await tabMenu(page, page, "b.txt", "split_right");
    await expect.poll(() => shape(page)).toBe("(>a.txt) | @(>b.txt)");
    const divider = page.locator("[data-view-divider]");
    await divider.focus();
    await page.keyboard.press("ArrowLeft");
    await expect(divider).toHaveAttribute("aria-valuenow", "45");
    const workspace = page.locator("[data-workspace-screen]");
    await expect(workspace).toHaveAttribute("data-lane-view", "documents");
    await expect.poll(() => (storedWorkspace(stack.daemon, stack.root)?.view_layout as { root?: { split?: { ratio?: number } } } | undefined)?.root?.split?.ratio).toBeCloseTo(0.45);
    const sentBefore = stack.events.filter((event) => event.kind === "workspace_view").length;
    const sidebar = (await boxOf(page.locator("[data-lane-row]"))).x;

    // A window with no room to dock opens the pinned lane over the agents,
    // its two columns stacked with the documents in front; View areas that
    // cannot all fit show the active one with a switcher to the others (PRD
    // right-lane B19; lane-two-columns B19; S7 B13).
    await page.setViewportSize({ width: Math.round(sidebar + 440), height: 1080 });
    await expect(workspace).toHaveAttribute("data-lane-mode", "overlay");
    await expect(workspace).toHaveAttribute("data-lane-stacked", "true");
    expect(await pinState(page)).toBe("narrow");
    await expect(page.locator('[data-view-areas="single"]')).toBeVisible();
    await expect(page.locator("[data-view-area-id]")).toHaveCount(1);
    await expect.poll(() => shape(page)).toBe("@(>b.txt)");
    const switcher = page.locator("[data-view-area-switch]");
    await expect(switcher).toHaveText("2/2");
    await screenshot(page, "s7-narrow-single-area");

    // The switcher shows another area; choosing one is an ordinary focus (B13).
    await switcher.click();
    await page.locator('[role="menu"] [data-menu-item]').first().click();
    await expect.poll(() => shape(page)).toBe("@(>a.txt)");
    await expect(switcher).toHaveText("1/2");
    await switcher.click();
    await page.locator('[role="menu"] [data-menu-item]').nth(1).click();
    await expect.poll(() => shape(page)).toBe("@(>b.txt)");

    // The close chord with the keyboard in the View area closes that view and
    // leaves the agents under the lane alone.
    await editor(page, 0).click();
    const herdrBefore = herdrIds(stack.herdr);
    const closes = () => viewEvents(stack).filter((event) => event.payload.action === "close").length;
    const closesBefore = closes();
    await page.keyboard.press(chord("close_tab"));
    await expect.poll(closes).toBe(closesBefore + 1);
    await expect.poll(() => shape(page)).toBe("@(>a.txt)");
    expect(stack.events.filter((event) => event.kind === "close_tab" || event.kind === "close_pane")).toHaveLength(0);
    expect(herdrIds(stack.herdr)).toEqual(herdrBefore);

    // An agent chosen from the sidebar keeps the pinned lane up, and nothing
    // narrow was sent or stored (B19).
    const agent = page.locator("nav[data-sidebar] [data-agent-open]").first();
    const pane = await agent.getAttribute("data-agent-open");
    await agent.click();
    await expect(page.locator(`[data-pane-view="${pane}"]`)).toHaveAttribute("data-focused", "true", { timeout: 15_000 });
    await expect(workspace).toHaveAttribute("data-lane-mode", "overlay");
    expect(stack.events.filter((event) => event.kind === "workspace_view").length).toBe(sentBefore);

    // Widening docks the lane again with what the core stores (B19, B22).
    await page.setViewportSize({ width: 1920, height: 1080 });
    await expect(workspace).toHaveAttribute("data-lane-mode", "docked");
    await expect(workspace).not.toHaveAttribute("data-lane-stacked", "true");
    await expect(workspace).toHaveAttribute("data-lane-view", "documents");
    await expect(page.locator('[data-lane="explorer"] [data-explorer]')).toBeVisible();
    const stored = storedWorkspace(stack.daemon, stack.root);
    expect([stored?.open, stored?.documents, stored?.tool, stored?.pinned]).toEqual([true, true, "explorer", true]);
    expectFrontWorkspaceOnEveryViewEvent(stack);
  } finally {
    stopStack(stack);
  }
});

/** The pane and tab ids the private Herdr holds, to show a restart used them as they were (B15). */
function herdrIds(herdr: HerdrFixture): string[] {
  const text = JSON.stringify(herdr.run(["api", "snapshot"]));
  return [...new Set([...text.matchAll(/"(?:pane_id|tab_id)":"([^"]+)"/g)].map((match) => match[1] ?? ""))].sort();
}

/** Reopens the app as a fresh page on the daemon, as relaunching it does. */
async function reopen(page: Page, daemon: Daemon): Promise<void> {
  await page.goto("about:blank");
  await open(page, daemon);
}

test("a restart brings the View areas back as they were, and a file gone meanwhile is unavailable on its own view until Retry", async ({ page }) => {
  await page.setViewportSize({ width: 1920, height: 1080 });
  const stack = await startStack(page, "s7-restore", { "a.txt": "a text\n", "b.txt": "b text\n", "c.txt": "c text\n", "d.txt": "d text\n", "e.txt": "e text\n" });
  try {
    const row = (name: string) => explorerRow(page, stack, name);
    // Three areas, a preview, two ratios, in the pinned lane beside its tree.
    await keepOpenInTurn(page, ["a.txt", "b.txt"], row);
    await widenLane(page);
    await row("c.txt").click();
    await tabMenu(page, page, "b.txt", "split_right");
    await expect.poll(() => shape(page)).toBe("(a.txt >c.txt*) | @(>b.txt)");
    await row("d.txt").dblclick();
    await tabMenu(page, page, "d.txt", "split_down");
    await row("e.txt").dblclick();
    await expect.poll(() => shape(page)).toBe("(a.txt >c.txt*) | (>b.txt) | @(d.txt >e.txt)");
    const dividers = page.locator("[data-view-divider]");
    await expect(dividers).toHaveCount(2);
    await dividers.nth(0).focus();
    await page.keyboard.press("ArrowLeft");
    await dividers.nth(1).focus();
    await page.keyboard.press("ArrowDown");
    await expect(dividers.nth(0)).toHaveAttribute("aria-valuenow", "45");
    await expect(dividers.nth(1)).toHaveAttribute("aria-valuenow", "55");
    await tab(area(page, 2), "e.txt").click();
    const before = await shape(page);
    expect(before).toBe("(a.txt >c.txt*) | (>b.txt) | @(d.txt >e.txt)");
    await expect.poll(() => storedWorkspace(stack.daemon, stack.root)).toMatchObject({ open: true, documents: true, tool: "explorer", pinned: true });
    await expect
      .poll(() => JSON.stringify(storedWorkspace(stack.daemon, stack.root)?.view_layout))
      .toMatch(/"ratio":0\.45.*"ratio":0\.55/);
    const panes = herdrIds(stack.herdr);
    await screenshot(page, "s7-restore-before");

    // The file goes while Hide is stopped, and Hide comes back on the same state.
    stack.daemon = await stack.daemon.restart(() => fs.rmSync(path.join(stack.root, "e.txt")));
    await reopen(page, stack.daemon);

    // The same Workspace, not Main: the tree, ratios, order, preview, each
    // area's view, the area in use, and the pinned lane with the Explorer
    // beside the documents (B14; PRD right-lane B22; lane-two-columns B17).
    const workspace = page.locator("[data-workspace-screen]");
    await expect(workspace).toHaveAttribute("data-lane-mode", "docked", { timeout: 20_000 });
    await expect(workspace).toHaveAttribute("data-lane-view", "documents");
    await expect(workspace).toHaveAttribute("data-lane-shows", "explorer");
    await expect(page.locator("[data-main-screen]")).toHaveCount(0);
    await expect.poll(() => shape(page), { timeout: 15_000 }).toBe(before);
    await expect(dividers.nth(0)).toHaveAttribute("aria-valuenow", "45");
    await expect(dividers.nth(1)).toHaveAttribute("aria-valuenow", "55");
    await expect(page.locator('[data-lane="explorer"] [data-explorer]')).toBeVisible();
    await expect(tab(page, "c.txt").getByText("c.txt")).toHaveCSS("font-style", "italic");
    await expect(editor(page, 0)).toContainText("c text");
    await expect(editor(page, 1)).toContainText("b text");

    // Only the gone file's view is unavailable, with Close view and Retry;
    // the others and their areas stay (B16).
    await expect(tab(page, "e.txt")).toHaveAttribute("data-unavailable", "true", { timeout: 15_000 });
    await expect(page.locator('[data-view-area-id] [role="tab"][data-unavailable="true"]')).toHaveCount(1);
    await expect(area(page, 2).locator("[data-close-unavailable]")).toBeVisible();
    await expect(area(page, 2).locator("[data-retry-unavailable]")).toBeVisible();
    await expect(page.locator("[data-close-unavailable]")).toHaveCount(1);
    await expect(page.locator("[data-retry-unavailable]")).toHaveCount(1);
    await screenshot(page, "s7-restore-unavailable");

    // Herdr's panes and tabs are used as they are: nothing was started or rewritten (B15).
    expect(herdrIds(stack.herdr)).toEqual(panes);

    // Retry reads the file again once it is back (B16).
    fs.writeFileSync(path.join(stack.root, "e.txt"), "e is back\n");
    await area(page, 2).locator("[data-retry-unavailable]").click();
    await expect(editor(page, 2)).toContainText("e is back");
    await expect(tab(page, "e.txt")).toHaveAttribute("data-unavailable", "false");
    expect(stack.last.get("view_layout.retry")).toMatchObject({ action: "retry" });
    await expect.poll(() => shape(page)).toBe(before);
    expectFrontWorkspaceOnEveryViewEvent(stack);
  } finally {
    stopStack(stack);
  }
});

test("a broken or unknown Views file is kept aside for Main and a fresh layout, and an S6 file becomes one area", async ({ page }) => {
  await page.setViewportSize({ width: 1920, height: 1080 });
  const stack = await startStack(page, "s7-recover", { "a.txt": "a text\n", "b.txt": "b text\n", "c.txt": "c text\n" });
  try {
    await explorerRow(page, stack, "a.txt").dblclick();
    await expect.poll(() => storedWorkspace(stack.daemon, stack.root)).not.toBeNull();
    const main = page.locator("[data-main-screen]");
    const workspace = page.locator("[data-workspace-screen]");
    const aside = () => fs.readdirSync(stateDir(stack.daemon)).filter((name) => name.startsWith("workspace-views.json.unreadable-"));

    // Unparseable, then a version this build does not know: each is moved
    // aside byte for byte, the app starts on Main, and a Workspace chosen
    // there starts a new layout (B17).
    for (const [index, broken] of ["{ not json", '{"schema_version": 4, "workspaces": [{"device_id": "local"}]}\n'].entries()) {
      stack.diagnostics.clear();
      stack.daemon = await stack.daemon.restart((dir) => fs.writeFileSync(path.join(dir, "workspace-views.json"), broken));
      await reopen(page, stack.daemon);
      await expect(main).toBeVisible({ timeout: 20_000 });
      await expect(workspace).toHaveCount(0);
      const kept = aside().map((name) => fs.readFileSync(path.join(stateDir(stack.daemon), name), "utf8"));
      expect(kept).toHaveLength(index + 1);
      expect(kept).toContain(broken);
      await expect.poll(() => stack.diagnostics.has("workspace_views.unreadable")).toBe(true);
      if (index === 0) await screenshot(page, "s7-recover-main");
      await enterWorkspace(page, "fixture");
      await expect(workspace).toHaveAttribute("data-lane-mode", "closed");
      await showExplorer(page);
      await explorerRow(page, stack, "b.txt").click();
      await expect.poll(() => shape(page)).toBe("@(>b.txt*)");
      await expect.poll(() => JSON.parse(fs.readFileSync(viewsFile(stack.daemon), "utf8")).schema_version as number).toBe(4);
    }

    // An S6 (schema 1) file: its tabs become one area in their order, with
    // its active tab and preview, and its lane starts closed until the
    // documents are asked for; the next save writes schema 4 (B17, contract
    // 1, PRD lane-two-columns B23). It names this machine `local`, as a
    // build of that time did, and the start converts that to the node id
    // (core-host-node D-21).
    const v1 = {
      schema_version: 1,
      workspaces: [
        {
          device_id: "local",
          path: stack.root,
          mode: "views",
          explorer: false,
          changes: true,
          agent_share: 0.4,
          tabs: ["a.txt", "b.txt", "c.txt"].map((name) => ({ path: `${stack.root}/${name}`, kind: "file", preview: name === "c.txt" })),
          active: { path: `${stack.root}/b.txt`, kind: "file" },
          last_used_unix_ms: Date.now(),
        },
      ],
    };
    stack.diagnostics.clear();
    stack.daemon = await stack.daemon.restart((dir) => fs.writeFileSync(path.join(dir, "workspace-views.json"), JSON.stringify(v1)));
    await reopen(page, stack.daemon);
    await expect(workspace).toHaveAttribute("data-lane-mode", "closed", { timeout: 20_000 });
    await expect(page.locator('[data-lane-tool="documents"] [data-lane-badge]')).toHaveText("3");
    await showDocuments(page);
    await expect(workspace).toHaveAttribute("data-lane-mode", "overlay");
    await expect.poll(() => shape(page), { timeout: 15_000 }).toBe("@(a.txt >b.txt c.txt*)");
    await expect(editor(page, 0)).toContainText("b text");
    await expect.poll(() => stack.diagnostics.has("workspace_views.migrated")).toBe(true);
    await screenshot(page, "s7-recover-migrated");
    await tab(page, "a.txt").click();
    await expect.poll(() => {
      const stored = JSON.parse(fs.readFileSync(viewsFile(stack.daemon), "utf8")) as { schema_version: number; workspaces: { path: string; view_layout?: { root?: { area?: { displays?: { path: string; preview: boolean }[] } } } }[] };
      const entry = stored.workspaces.find((candidate) => candidate.path === stack.root);
      return { version: stored.schema_version, displays: entry?.view_layout?.root?.area?.displays?.map((display) => `${path.basename(display.path)}${display.preview ? "*" : ""}`) };
    }).toEqual({ version: 3, displays: ["a.txt", "b.txt", "c.txt*"] });
    expectFrontWorkspaceOnEveryViewEvent(stack);
  } finally {
    stopStack(stack);
  }
});

/** A committed checkout whose `a.txt` then changes, so History lists a working diff. */
function gitCheckout(checkout: string): void {
  const git = (...args: string[]) => execFileSync("git", args, { cwd: checkout });
  git("init", "-q", "-b", "main");
  git("add", "-A");
  git("-c", "user.email=e2e@example.com", "-c", "user.name=e2e", "-c", "commit.gpgsign=false", "commit", "-qm", "base");
  fs.appendFileSync(path.join(checkout, "a.txt"), "changed\n");
}

test("the rail, tools, kind marks and an open while the lane is closed keep working beside several View areas", async ({ page }) => {
  await page.setViewportSize({ width: 1920, height: 1080 });
  const stack = await startStack(page, "s7-s6", { "a.txt": "a text\n", "b.txt": "b text\n", "c.txt": "c text\n" }, { prepare: gitCheckout });
  try {
    const workspace = page.locator("[data-workspace-screen]");
    const laneSent = () => stack.events.filter((event) => event.kind === "workspace_view").length;

    // The rail carries the documents' button and the four tools, and the
    // top row none of them (PRD right-lane B1; lane-two-columns B2).
    await expect(page.locator("[data-tool-rail] [data-lane-tool]")).toHaveCount(5);
    await expect(page.locator("[data-workspace-toolbar] [data-lane-tool]")).toHaveCount(0);

    // The first file opens one View area beside the Explorer (B2, B12).
    await expect(workspace).toHaveAttribute("data-lane-shows", "explorer");
    await expect(workspace).toHaveAttribute("data-lane-view", "none");
    await expect(page.locator("[data-view-area]")).toHaveCount(0);
    await explorerRow(page, stack, "a.txt").dblclick();
    await expect(page.locator("[data-view-area]")).toBeVisible();
    await expect(workspace).toHaveAttribute("data-lane-view", "documents");
    await expect.poll(() => shape(page)).toBe("@(>a.txt)");
    await expect(page.locator("[data-view-divider]")).toHaveCount(0);
    expect(viewEvents(stack)).toHaveLength(0);

    // Kind marks, never colour alone: a file's type mark, a diff's comparison
    // mark, and each agent tab's status mark (S6 D-15; the provider logo is on the pane header only).
    await showTool(page, "changes");
    await page.locator('[data-history-group="working"][data-history-path="a.txt"]').click();
    await expect.poll(() => shape(page)).toBe("@(a.txt >a.txt*)");
    await expect(page.locator('[data-view-area-id] [data-tab-kind="file"] [data-view-mark="file"]')).toHaveCount(1);
    await expect(page.locator('[data-view-area-id] [data-tab-kind="diff"] [data-view-mark="diff"]')).toHaveCount(1);
    await expect(page.locator('[data-tab-kind="diff"]')).toHaveAttribute("aria-label", /^Working diff: /);
    await expect(page.locator("[data-agent-tab-bar] [data-tab-status]").first()).toBeVisible();
    await expect(page.locator("[data-agent-tab-bar] img")).toHaveCount(0);
    await screenshot(page, "s7-s6-kind-marks");

    // Two areas, the right one used last. The diff is pinned first, since
    // the next click would take its preview slot (B1).
    await page.locator('[data-tab-kind="diff"]').dblclick();
    await expect.poll(() => shape(page)).toBe("@(a.txt >a.txt)");
    await showExplorer(page);
    await explorerRow(page, stack, "b.txt").dblclick();
    await widenLane(page);
    await tabMenu(page, page, "b.txt", "split_right");
    await expect.poll(() => shape(page)).toBe("(a.txt >a.txt) | @(>b.txt)");

    // The rail swaps the tool column's tool, closes the column and opens it
    // again beside several areas, one event each, and the areas stay as they
    // are (PRD lane-two-columns B7, B14).
    const toolsBefore = laneSent();
    const viewsBefore = viewEvents(stack).length;
    await page.locator('[data-lane-tool="changes"]').click();
    await expect(page.locator("[data-explorer]")).toHaveCount(0);
    await expect(page.locator('[data-lane="changes"]')).toBeVisible();
    await page.locator('[data-lane-tool="changes"]').click();
    await expect(workspace).toHaveAttribute("data-lane-shows", "none");
    await expect(workspace).toHaveAttribute("data-lane-mode", "docked");
    await page.locator('[data-lane-tool="explorer"]').click();
    await expect(page.locator('[data-lane="explorer"]')).toBeVisible();
    expect(laneSent()).toBe(toolsBefore + 3);
    expect(viewEvents(stack).length).toBe(viewsBefore);
    await expect.poll(() => shape(page)).toBe("(a.txt >a.txt) | @(>b.txt)");

    // The closed lane keeps its views and the documents button counts them; a
    // file opened then opens the lane through the open itself, the areas
    // whole, and lands in the area used last (PRD right-lane B16, B17).
    await page.keyboard.press(chord("toggle_lane"));
    await expect(workspace).toHaveAttribute("data-lane-mode", "closed");
    await expect(page.locator("[data-view-area-id]")).toHaveCount(0);
    await expect(page.locator('[data-lane-tool="documents"] [data-lane-badge]')).toHaveText("3");
    const sentBefore = laneSent();
    await page.keyboard.press(chord("open_file"));
    const input = page.locator('[data-palette="Open file"] [data-palette-input]');
    await expect(input).toBeVisible();
    await input.fill("c.txt");
    const row = page.locator(`[data-palette-row="${stack.root}/c.txt"]`);
    await expect(row).toBeVisible({ timeout: 20_000 });
    await row.click();
    await expect(workspace).toHaveAttribute("data-lane-mode", "docked");
    await expect.poll(() => shape(page)).toBe("(a.txt >a.txt) | @(b.txt >c.txt*)");
    await expect(area(page, 1)).toHaveAttribute("data-active-area", "true");
    expect(laneSent()).toBe(sentBefore);
    // The diff in the lane is read like any diff on screen.
    await expect(page.locator('[data-lane-frame] .cm-content', { hasText: "+changed" })).toBeVisible({ timeout: 10_000 });
    await screenshot(page, "s7-s6-open-from-off");
    expectFrontWorkspaceOnEveryViewEvent(stack);
  } finally {
    stopStack(stack);
  }
});
