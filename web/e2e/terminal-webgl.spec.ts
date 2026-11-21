// Real WebGL context loss, on the existing private daemon/PTY fixtures, either engine.
// Software WebGL proves browser recovery, not native GPU or physical input.
import { expect, test, type Page } from "@playwright/test";
import fs from "node:fs";
import { startFixture, type AgentFixture } from "./engine-fixture";
import { closePane, withControl } from "./daemon-actions";
import { startHided, type Daemon } from "./hided-fixture";
import { countSent, enterWorkspace } from "./wire";
import { testBrowserArgs } from "./browser-launch.cjs";

declare global {
  interface Window {
    __webglLoss?: {
      canvas: HTMLCanvasElement;
      extension: WEBGL_lose_context;
      lost: boolean;
      restored: boolean;
    };
  }
}

async function start(page: Page) {
  const engine = await startFixture();
  let daemon: Daemon | undefined;
  const stop = () => {
    try { daemon?.stop(); } finally { engine.stop(); }
  };
  try {
    daemon = await startHided(engine, "terminal-webgl");
    const sent = countSent(page);
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    const pane = engine.panes[0];
    const view = page.locator(`[data-pane-view="${pane}"]`);
    await expect(view).toHaveAttribute("data-transport", "controlling");
    await view.locator(".xterm-helper-textarea").focus();
    await expect.poll(() => page.evaluate(() => window.__hideProbe?.paneId())).toBe(pane);
    await typeMarker(page, pane, "BEFORE_GPU_LOSS");
    return { engine, daemon, pane, view, sent, stop };
  } catch (error) {
    stop();
    throw error;
  }
}

async function typeMarker(page: Page, pane: string, marker: string) {
  await page.keyboard.type(`${marker}\n`);
  await expect.poll(() => page.evaluate((id) => window.__hideProbe?.paneText(id), pane)).toContain(marker);
}

async function selectMarker(page: Page, pane: string) {
  const cells = await page.evaluate((id) => {
    const lines = window.__hideProbe!.paneText(id).split("\n");
    const row = lines.findIndex((line) => line.includes("BEFORE_GPU_LOSS"));
    return { row, col: lines[row]!.indexOf("BEFORE_GPU_LOSS"), grid: window.__hideProbe!.paneGrid(id)! };
  }, pane);
  const box = (await page.locator(`[data-terminal="${pane}"] .xterm-screen`).boundingBox())!;
  const cellWidth = box.width / cells.grid.cols;
  const x = box.x + (cells.col + 0.1) * cellWidth;
  const y = box.y + (cells.row + 0.5) * box.height / cells.grid.rows;
  // Start inside the text: the split divider's hit area covers the first column.
  await page.mouse.move(x + "BEFORE_GPU_LOSS".length * cellWidth, y);
  await page.mouse.down();
  await page.mouse.move(x, y, { steps: 4 });
  await page.mouse.up();
  await expect.poll(() => page.evaluate((id) => window.__hideProbe?.paneSelection(id), pane)).toBe("BEFORE_GPU_LOSS");
}

async function loseContext(page: Page, pane: string) {
  await page.locator(`[data-terminal="${pane}"]`).evaluate((element) => {
    const canvas = [...element.querySelectorAll("canvas")].find((candidate) => candidate.getContext("webgl2"));
    if (!canvas) throw new Error("fixture needs an active WebGL2 terminal");
    const extension = canvas.getContext("webgl2")!.getExtension("WEBGL_lose_context");
    if (!extension) throw new Error("fixture needs WEBGL_lose_context");
    const state = window.__webglLoss = { canvas, extension, lost: false, restored: false };
    canvas.addEventListener("webglcontextlost", () => { state.lost = true; }, { once: true });
    canvas.addEventListener("webglcontextrestored", () => { state.restored = true; }, { once: true });
    extension.loseContext();
  });
  await expect.poll(() => page.evaluate(() => window.__webglLoss?.lost)).toBe(true);
}

function received(engine: AgentFixture) {
  return fs.readFileSync(engine.inputLogs[0], "utf8").replaceAll("\r", "\n");
}

test("unavailable WebGL keeps the terminal usable and closes cleanly", async ({ page }) => {
  // The suite's --disable-webgl exercises actual activation failure.
  const flow = await start(page);
  try {
    await expect(flow.view.locator(".xterm-rows")).toHaveCount(1);
    await typeMarker(page, flow.pane, "AFTER_NO_WEBGL");
    await expect.poll(() => received(flow.engine)).toBe("BEFORE_GPU_LOSS\nAFTER_NO_WEBGL\n");
    await withControl(flow.daemon, (control) => closePane(control, flow.pane));
    await expect.poll(() => page.evaluate(() => window.__hideProbe?.liveTerminals())).not.toContain(flow.pane);
  } finally { flow.stop(); }
});

// A separate fixture pool replaces the suite's --disable-webgl for these cases.
// SwiftShader supplies a repeatable real context without requiring host GPU hardware.
const webglTest = test.extend({
  launchOptions: { args: testBrowserArgs(["--use-gl=angle", "--use-angle=swiftshader", "--enable-unsafe-swiftshader"]) },
});

webglTest.describe("WebGL recovery", () => {
  const test = webglTest;
  test("transient restoration stays on WebGL; persistent loss preserves the same terminal", async ({ page }) => {
    const flow = await start(page);
    try {
      const textarea = await flow.view.locator(".xterm-helper-textarea").elementHandle();
      const grid = await page.evaluate((id) => window.__hideProbe!.paneGrid(id), flow.pane);
      await selectMarker(page, flow.pane);
      // Own the addon timer so a loaded runner cannot turn a transient loss into persistent loss.
      await page.clock.install();
      await page.clock.pauseAt(await page.evaluate(() => Date.now() + 1000));
      await loseContext(page, flow.pane);
      await page.evaluate(() => window.__webglLoss!.extension.restoreContext());
      await expect.poll(() => page.evaluate(() => window.__webglLoss?.restored)).toBe(true);
      await page.clock.runFor(3100);
      expect(await page.evaluate(() => window.__webglLoss!.canvas.isConnected)).toBe(true);
      await expect(flow.view.locator(".xterm-rows")).toHaveCount(0);
      await page.clock.resume();
      await typeMarker(page, flow.pane, "AFTER_RESTORE");
      await selectMarker(page, flow.pane);
      const viewports = flow.sent.get("terminal_viewport") ?? 0;
      await page.clock.pauseAt(await page.evaluate(() => Date.now() + 1000));
      await loseContext(page, flow.pane);
      await page.clock.runFor(3100);
      await page.clock.resume();
      await expect(flow.view.locator(".xterm-rows")).toHaveCount(1);
      expect(await page.evaluate(() => window.__webglLoss!.canvas.isConnected)).toBe(false);
      expect(await textarea!.evaluate((element) => element === document.querySelector(`[data-terminal="${window.__hideProbe!.paneId()}"] textarea`))).toBe(true);
      expect(await page.evaluate((id) => window.__hideProbe!.paneGrid(id), flow.pane)).toEqual(grid);
      expect(await page.evaluate((id) => window.__hideProbe!.paneSelection(id), flow.pane)).toBe("BEFORE_GPU_LOSS");
      await expect(flow.view.locator(".xterm-selection div").first()).toBeVisible();
      await typeMarker(page, flow.pane, "AFTER_FALLBACK");
      await expect.poll(() => received(flow.engine)).toBe("BEFORE_GPU_LOSS\nAFTER_RESTORE\nAFTER_FALLBACK\n");
      expect(flow.sent.get("terminal_viewport") ?? 0).toBe(viewports);
      await expect.poll(() => page.evaluate((id) => window.__hideProbe?.paneText(id), flow.pane)).toContain("BEFORE_GPU_LOSS");
      await test.info().attach("dom-after-context-loss", { body: await flow.view.screenshot(), contentType: "image/png" });
    } finally { flow.stop(); }
  });

  test("closing a pane during context loss releases its renderer before the loss deadline", async ({ page }) => {
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    const flow = await start(page);
    try {
      await page.clock.install();
      await page.clock.pauseAt(await page.evaluate(() => Date.now() + 1000));
      await loseContext(page, flow.pane);
      await withControl(flow.daemon, (control) => closePane(control, flow.pane));
      await expect.poll(() => page.evaluate(() => window.__hideProbe?.liveTerminals())).not.toContain(flow.pane);
      await page.clock.runFor(3100);
      await page.clock.resume();
      expect(await page.evaluate(() => window.__webglLoss!.canvas.isConnected)).toBe(false);
      expect(await page.evaluate(() => window.__hideProbe!.diagnostics())).not.toContain(`terminal.renderer pane=${flow.pane} mode=dom reason=context_loss`);
      expect(errors).toEqual([]);
    } finally { flow.stop(); }
  });
});
