import { expect, type Locator, type Page } from "@playwright/test";
import path from "node:path";
import { startHerdr, type HerdrFixture } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { countSent, enterWorkspace } from "./wire";

export type PaneFact = { pane_id: string; terminal_id: string; tab_id: string };
export function paneFacts(herdr: HerdrFixture): PaneFact[] {
  return (herdr.run(["pane", "list"]) as { result: { panes: PaneFact[] } }).result.panes;
}
export function processFacts(herdr: HerdrFixture, paneId: string) {
  const info = (herdr.run(["pane", "process-info", "--pane", paneId]) as { result: { process_info: { shell_pid: number | null; tty: string | null; foreground_process_group_id: number | null; foreground_processes: { pid: number }[] } } }).result.process_info;
  expect(info.shell_pid).toBeGreaterThan(0);
  return { shell: info.shell_pid, tty: info.tty, group: info.foreground_process_group_id, foreground: info.foreground_processes.map(process => process.pid) };
}
export const paneView = (page: Page, paneId: string) => page.locator(`[data-pane-view="${paneId}"]`);
export const tabTitle = (page: Page, tabId: string) => page.locator(`[data-agent-tab-bar] [role=tab][data-tab="${tabId}"]`);
export async function startMove(page: Page, crossWorkspace = false) {
  const herdr = await startHerdr();
  let daemon: Daemon | undefined;
  const stop = () => { try { daemon?.stop(); } finally { herdr.stop(); } };
  try {
    const args = crossWorkspace ? ["workspace", "create", "--cwd", path.join(herdr.root, "fixture"), "--label", "Move destination", "--env", `PATH=${herdr.fixturePath}`, "--no-focus"] : ["tab", "create", "--workspace", herdr.workspace, "--cwd", path.join(herdr.root, "fixture"), "--label", "Move destination", "--env", `PATH=${herdr.fixturePath}`, "--no-focus"];
    const target = (herdr.run(args) as { result: { tab: { tab_id: string }; root_pane: { pane_id: string } } }).result;
    daemon = await startHided(herdr, "pane-move");
    const payloads = new Map<string, Record<string, unknown>>();
    const sent = countSent(page, payloads);
    const operations = new Map<string, string>();
    page.on("websocket", socket => socket.on("framereceived", ({ payload }) => {
      const frame = JSON.parse(String(payload)) as { payload?: { rest?: { status?: { async_operations?: { id: string; kind: string; phase: string }[] } } } };
      for (const operation of frame.payload?.rest?.status?.async_operations ?? []) if (operation.kind === "pane.move") operations.set(operation.id, operation.phase);
    }));
    // Geometry may already match the desired edge. Await this exact request's
    // authoritative receipt before attempting another gesture on the pane.
    const completed = async (count = sent.get("pane_move") ?? 1) => {
      await expect.poll(() => sent.get("pane_move")).toBe(count);
      await expect.poll(() => operations.get(String(payloads.get("pane_move")?.request_id))).toBe("completed");
    };
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    for (const pane of herdr.panes) await expect(paneView(page, pane)).toHaveAttribute("data-transport", "controlling");
    await expect(tabTitle(page, target.tab.tab_id)).toBeVisible();
    return { herdr, daemon, target, sent, payloads, completed, stop };
  } catch (error) { stop(); throw error; }
}
export async function beginDrag(page: Page, paneId: string, target: Locator, edge: "left" | "right" | "up" | "down" = "right") {
  const title = paneView(page, paneId).locator("[data-pane-move-title]").first();
  await target.scrollIntoViewIfNeeded();
  await title.hover();
  const source = (await title.boundingBox())!;
  const destination = (await target.boundingBox())!;
  await page.mouse.move(source.x + source.width / 2, source.y + source.height / 2);
  await page.mouse.down();
  const x = destination.x + destination.width * (edge === "left" ? 0.1 : edge === "right" ? 0.9 : 0.5);
  const y = destination.y + destination.height * (edge === "up" ? 0.1 : edge === "down" ? 0.9 : 0.5);
  await page.mouse.move(x, y, { steps: 6 });
}
export async function expectDirection(page: Page, source: string, target: string, edge: "left" | "right" | "up" | "down") {
  await expect.poll(async () => {
    const a = await paneView(page, source).boundingBox(), b = await paneView(page, target).boundingBox();
    if (!a || !b) return false;
    return edge === "left" ? a.x < b.x : edge === "right" ? a.x > b.x : edge === "up" ? a.y < b.y : a.y > b.y;
  }).toBe(true);
}
export async function selectMarker(page: Page, paneId: string, marker: string) {
  const cells = await page.evaluate(({ paneId, marker }) => {
    const lines = window.__hideProbe!.paneText(paneId).split("\n");
    const row = lines.findIndex(line => line.includes(marker));
    return { row, col: lines[row]!.indexOf(marker), grid: window.__hideProbe!.paneGrid(paneId)! };
  }, { paneId, marker });
  const box = (await paneView(page, paneId).locator(".xterm-screen").boundingBox())!;
  const width = box.width / cells.grid.cols, y = box.y + (cells.row + 0.5) * box.height / cells.grid.rows;
  await page.mouse.move(box.x + (cells.col + marker.length) * width, y); await page.mouse.down();
  await page.mouse.move(box.x + (cells.col + 0.1) * width, y, { steps: 4 }); await page.mouse.up();
  await expect.poll(() => page.evaluate(id => window.__hideProbe!.paneSelection(id), paneId)).toBe(marker);
}
