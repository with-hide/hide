import { expect, type Locator, type Page } from "@playwright/test";
import path from "node:path";
import { startFixture } from "./engine-fixture";
import { focusTab, newTab, tabOfPane } from "./daemon-actions";
import { connectControl, type HidedControl, type Rest } from "./hided-control.mjs";
import { startHerdr, type HerdrFixture } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { countSent, enterWorkspace } from "./wire";

/** What `pane list` says of a pane, which only Herdr answers. */
export type PaneFact = { pane_id: string; terminal_id: string; tab_id: string };
export function paneFacts(herdr: Pick<HerdrFixture, "run">): PaneFact[] {
  return (herdr.run(["pane", "list"]) as { result: { panes: PaneFact[] } }).result.panes;
}
/** The shell, terminal and foreground processes of a pane as Herdr reports them. */
export function processFacts(herdr: Pick<HerdrFixture, "run">, paneId: string) {
  const info = (herdr.run(["pane", "process-info", "--pane", paneId]) as { result: { process_info: { shell_pid: number | null; tty: string | null; foreground_process_group_id: number | null; foreground_processes: { pid: number }[] } } }).result.process_info;
  expect(info.shell_pid).toBeGreaterThan(0);
  return { shell: info.shell_pid, tty: info.tty, group: info.foreground_process_group_id, foreground: info.foreground_processes.map(process => process.pid) };
}
export const paneView = (page: Page, paneId: string) => page.locator(`[data-pane-view="${paneId}"]`);
export const tabTitle = (page: Page, tabId: string) => page.locator(`[data-agent-tab-bar] [role=tab][data-tab="${tabId}"]`);

/** A move journey: the page on a Workspace of two panes, and a second tab to move one of them to. */
type Drive = {
  daemon: Daemon;
  control: HidedControl;
  panes: string[];
  target: { tab: string; pane: string };
  sent: Map<string, number>;
  payloads: Map<string, Record<string, unknown>>;
  completed: (count?: number) => Promise<void>;
};

/**
 * Opens the page on the daemon and watches the moves it sends. Geometry may
 * already match the desired edge, so `completed` awaits this exact request's
 * authoritative receipt before another gesture is attempted on the pane.
 */
async function drive(page: Page, daemon: Daemon, control: HidedControl, panes: string[], shown: string, target: { tab: string; pane: string }): Promise<Drive> {
  const payloads = new Map<string, Record<string, unknown>>();
  const sent = countSent(page, payloads);
  const operations = new Map<string, string>();
  page.on("websocket", socket => socket.on("framereceived", ({ payload }) => {
    const frame = JSON.parse(String(payload)) as { payload?: { rest?: { status?: { async_operations?: { id: string; kind: string; phase: string }[] } } } };
    for (const operation of frame.payload?.rest?.status?.async_operations ?? []) if (operation.kind === "pane.move") operations.set(operation.id, operation.phase);
  }));
  const completed = async (count = sent.get("pane_move") ?? 1) => {
    await expect.poll(() => sent.get("pane_move")).toBe(count);
    await expect.poll(() => operations.get(String(payloads.get("pane_move")?.request_id))).toBe("completed");
  };
  await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
  await enterWorkspace(page, "fixture");
  for (const pane of panes) await expect(paneView(page, pane)).toHaveAttribute("data-transport", "controlling");
  await expect(tabTitle(page, target.tab)).toBeVisible();
  await expect(tabTitle(page, shown)).toHaveAttribute("aria-selected", "true");
  return { daemon, control, panes, target, sent, payloads, completed };
}

/** The tab the daemon's snapshot puts `pane` in, if it lists the pane. */
export async function tabNow(flow: Pick<Drive, "control">, pane: string): Promise<string | undefined> {
  return flow.control.until((rest: Rest) => tabOfPane(rest, pane), `the tab of ${pane}`, 2_000).catch(() => undefined);
}

/**
 * The fixture's checkout with its two agent panes in one tab, and a second
 * tab, "Move destination", that stays behind the first, on either engine.
 */
export async function startMove(page: Page) {
  const fixture = await startFixture();
  let daemon: Daemon | undefined;
  let control: HidedControl | undefined;
  const stop = () => { control?.close(); try { daemon?.stop(); } finally { fixture.stop(); } };
  try {
    daemon = await startHided(fixture, "pane-move");
    control = await connectControl(daemon);
    const target = await newTab(control, fixture, "Move destination");
    // The new tab came forward as New tab makes it; the pair of panes stays in front.
    await focusTab(control, fixture, fixture.tab);
    return { fixture, ...(await drive(page, daemon, control, fixture.panes, fixture.tab, target)), stop };
  } catch (error) { stop(); throw error; }
}

/**
 * The same journey with the destination tab in a second native Herdr
 * workspace, where Herdr changes the public pane id of a pane moved to it.
 */
export async function startCrossWorkspaceMove(page: Page) {
  const herdr = await startHerdr();
  let daemon: Daemon | undefined;
  let control: HidedControl | undefined;
  const stop = () => { control?.close(); try { daemon?.stop(); } finally { herdr.stop(); } };
  try {
    const made = (herdr.run(["workspace", "create", "--cwd", path.join(herdr.root, "fixture"), "--label", "Move destination", "--env", `PATH=${herdr.fixturePath}`, "--no-focus"]) as { result: { tab: { tab_id: string }; root_pane: { pane_id: string } } }).result;
    daemon = await startHided(herdr, "pane-move");
    control = await connectControl(daemon);
    const target = { tab: made.tab.tab_id, pane: made.root_pane.pane_id };
    return { herdr, ...(await drive(page, daemon, control, herdr.panes, herdr.tab, target)), stop };
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
