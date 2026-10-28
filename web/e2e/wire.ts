// Shared e2e helpers: the client-event counter both flow specs assert with,
// the screenshot that writes only when a run directory was named, and the
// way into a Workspace from the Main a first run opens on.

import { expect, type Locator, type Page, type WebSocket } from "@playwright/test";
import path from "node:path";
import { chord } from "./chords";
import { dumpOnFailure, keepFailedRequests, pageSide } from "./failure-dump";
import { animationsFinished } from "./wait";

/** What the English shell names the core's own machine on the system the fixture runs on (`systemMachineKey` in `web/src/screenMachine.ts`). */
export const THIS_MACHINE = process.platform === "darwin" ? "This Mac" : process.platform === "win32" ? "This PC" : "This computer";

/**
 * Counts client events by kind as the page sends them; one action must be
 * one event. `last` keeps the newest payload per kind for shape assertions.
 * An event whose payload names an `action` (`view_layout`) is counted and
 * kept under `kind.action` as well, so one action can be told from another.
 */
export function countSent(page: Page, last: Map<string, Record<string, unknown>> = new Map()): Map<string, number> {
  const counts = new Map<string, number>();
  // With HIDE_E2E_TRACE the wire is narrated with timestamps, so a failed
  // flow shows which side moved and when; Playwright prints it for a failure.
  const trace = process.env.HIDE_E2E_TRACE ? (line: string) => console.log(`[${Date.now()}] ${line}`) : null;
  page.on("pageerror", (error) => console.log(`[pageerror] ${error.message}`));
  keepFailedRequests(page);
  page.on("websocket", (ws: WebSocket) => {
    ws.on("socketerror", (error) => console.log(`[ws error] ${error}`));
    ws.on("close", () => trace?.("ws closed"));
    ws.on("framereceived", (frame) => {
      if (!trace) return;
      const head = String(frame.payload);
      const focused = /"focused_pane_id":"([^"]*)"/.exec(head);
      trace(`recv ${head.slice(0, 40)}${focused ? ` focused_pane_id=${focused[1]}` : ""}`);
    });
    ws.on("framesent", (frame) => {
      try {
        const event = JSON.parse(String(frame.payload)) as { kind?: string; payload?: Record<string, unknown> };
        if (event.kind) {
          trace?.(`sent ${event.kind} ${JSON.stringify(event.payload ?? {}).slice(0, 100)}`);
          const action = typeof event.payload?.action === "string" ? `${event.kind}.${event.payload.action}` : null;
          // The usage hints ride their own UI-state update, which the page
          // sends on its own schedule, not for an action; it is counted apart.
          const hint = event.kind === "ui_state_update" && ["usage_window_visible", "usage_popover_open"].some((field) => field in (event.payload ?? {}));
          // A look at the project in front (`web/src/githubLook.ts`) rides the
          // GitHub request event on every focus, not for a screen's ask; it is
          // counted apart so "nothing asked GitHub" still means no screen did.
          const look = event.kind === "github_request" && event.payload?.look === true;
          for (const key of hint ? ["ui_state_update.usage_hint"] : look ? ["github_request.look"] : action ? [event.kind, action] : [event.kind]) {
            counts.set(key, (counts.get(key) ?? 0) + 1);
            last.set(key, event.payload ?? {});
          }
        }
      } catch {
        /* the handshake is not an event */
      }
    });
  });
  return counts;
}

/**
 * A first run opens on Main (S6 D-11); a spec about the Workspace goes in
 * through the sidebar's row of its Project (the one named `project`, else
 * the first), which opens that Project's Workspace. A page that already
 * shows a Workspace is left where it is.
 */
export async function enterWorkspace(page: Page, project?: string): Promise<void> {
  const main = page.locator("[data-main-screen]");
  const workspace = page.locator("[data-workspace-screen]");
  await dumpOnFailure("shell start", () => pageSide(page), () => expect(main.or(workspace)).toBeVisible({ timeout: 20_000 }));
  if ((await workspace.count()) > 0) return;
  // A repository's row is `data-project-row`; a plain folder's is its one checkout's open button.
  const named = project ? { has: page.locator("[data-row-name]", { hasText: new RegExp(`^${project}$`) }) } : {};
  const row = page.locator("nav[data-sidebar] [data-project]", named).first().locator("[data-project-row], button[data-checkout]").first();
  await (await dumpOnFailure("project row", () => pageSide(page), () => showProjectRow(page, row))).click();
  await expect(workspace).toBeVisible();
}

/**
 * Captures the page once the terminals have painted: xterm draws a written
 * buffer on the next animation frame, so a capture taken straight after a
 * probe read the rows can still show the cleared canvas of a resize.
 */
export async function screenshot(page: Page, name: string): Promise<unknown> {
  const dir = process.env.HIDE_E2E_SCREENSHOT_DIR;
  if (!dir) return;
  await page.evaluate(() => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done))));
  return page.screenshot({ path: path.join(dir, `${name}.png`) });
}

export type LaneTool = "project" | "memory" | "explorer" | "changes";

/**
 * Shows one tool in the lane's tool column: its rail icon opens the column
 * on that tool, or puts the lane back up when it holds it hidden (PRD
 * lane-two-columns B3, B14). A press on the tool the column already shows
 * would close the column, so a pressed icon is left alone.
 */
export async function showTool(page: Page, tool: LaneTool): Promise<void> {
  const icon = page.locator(`[data-lane-tool="${tool}"]`);
  if ((await icon.getAttribute("aria-pressed")) !== "true") await icon.click();
  await expect(icon).toHaveAttribute("aria-pressed", "true");
  await expect(page.locator(`aside[data-lane="${tool}"]`)).toBeVisible();
}

/** Shows the Hide AI tool (lane id `memory`), the front checkout's Project's 제안 and Memory. */
export async function showProjectMemory(page: Page): Promise<void> {
  await showTool(page, "memory");
}

/** Turns the sidebar to its PR list (PRD lane-two-columns B41). */
export async function showSidebarPrs(page: Page): Promise<void> {
  const option = page.locator('[data-sidebar-lens-option="prs"]');
  if ((await option.getAttribute("aria-pressed")) !== "true") await option.click();
  await expect(page.locator("[data-sidebar-prs]")).toBeVisible();
}

/** Brings the checkout's documents in front of the viewing column from the rail (B2, B11); a pressed button is left alone, since a press would fold them. */
export async function showDocuments(page: Page): Promise<void> {
  const button = page.locator('[data-lane-tool="documents"]');
  if ((await button.getAttribute("aria-pressed")) !== "true") await button.click();
  await expect(button).toHaveAttribute("aria-pressed", "true");
  await expect(page.locator("[data-workspace-screen]")).toHaveAttribute("data-lane-view", "documents");
}

/** The pin's state as the rail's pin button shows it: `off`, `on` or `narrow` (PRD lane-two-columns B15, B19). */
export async function pinState(page: Page): Promise<string | null> {
  return page.locator("[data-lane-pin]").getAttribute("data-lane-pin");
}

/** Pins or unpins the lane with the rail's pin button (B15). */
export async function setLanePinned(page: Page, pinned: boolean): Promise<void> {
  const button = page.locator("[data-lane-pin]");
  const on = (await button.getAttribute("data-lane-pin")) !== "off";
  if (on !== pinned) await button.click();
  // A pinned lane docks, or stays over the agents where docking has no room (B19).
  await expect(button).toHaveAttribute("data-lane-pin", pinned ? /^(on|narrow)$/ : /^off$/);
}

/** Turns the sidebar to its 태스크 list (PRD sidebar-task-lens B1) and waits for the core's task scope. */
export async function showTasks(page: Page): Promise<Locator> {
  const option = page.locator('[data-sidebar-lens-option="tasks"]');
  if ((await option.getAttribute("aria-pressed")) !== "true") await option.click();
  await expect(page.locator('[data-sidebar-lens="tasks"]')).toBeVisible();
  const list = page.locator("[data-task-lens]");
  await expect(list.locator("[data-task-count]")).toBeVisible({ timeout: 20_000 });
  return list;
}

/** Opens every 쉬는 중 and 정리할 것 fold of the 태스크 list, so a resting task's row is drawn (B5). */
export async function openTaskFolds(page: Page): Promise<void> {
  const folded = page.locator('[data-task-fold] > button[aria-expanded="false"]');
  while ((await folded.count()) > 0) await folded.first().click();
}

/**
 * The 태스크 list row of `pane` once it says `goal`. An idle session rests
 * under the folded 쉬는 중 (B5), so the fold is opened as soon as it is drawn;
 * it stays open while the list is shown.
 */
export async function taskRow(page: Page, pane: string, goal: string): Promise<Locator> {
  const list = await showTasks(page);
  const row = list.locator(`[data-task-row="${pane}"]`);
  await expect(row.or(list.locator('[data-task-fold="rest"]')).first()).toBeVisible({ timeout: 45_000 });
  await openTaskFolds(page);
  await expect(row).toContainText(goal, { timeout: 45_000 });
  return row;
}

/**
 * The boxes of a dependency graph's cards under `scope`, once the layout
 * worker has placed them: until then the cards stand in plain layers, and the
 * placed drawing mounts them anew, so a box read in between is of a card
 * already gone.
 */
export async function placedBoxes(scope: Locator, ...cards: string[]): Promise<{ x: number; y: number; width: number; height: number }[]> {
  await expect(scope.locator("[data-dependency-graph]")).toHaveAttribute("data-dependency-layout", "layered");
  return Promise.all(cards.map(async (card) => (await scope.locator(card).boundingBox())!));
}

/** Expands the lane over the agents, or brings it back, from the rail's `…` menu (PRD lane-two-columns B20). */
export async function chooseExpand(page: Page): Promise<void> {
  await page.locator("[data-lane-more]").click();
  await page.locator('[data-lane-menu-item="expand"]').click();
  await expect(page.locator('[role="menu"]')).toHaveCount(0);
}

/** Shows or hides the right lane from the Workspace path's menu (B20). */
export async function chooseLane(page: Page): Promise<void> {
  await page.locator("[data-workspace-location]").click({ button: "right" });
  await page.locator('[data-menu-item="lane"]').click();
  await expect(page.locator('[role="menu"]')).toHaveCount(0);
}

/**
 * Binds a chord to a command that has none by default in Settings, Shortcuts
 * (the area commands and the global Recent Panels pair have no other way to run
 * in a browser tab), then closes Settings and waits for it to hand the
 * keyboard back where it was: that lands a tick after the close, and a key
 * pressed before it would start from the page body instead.
 */
export async function bindChordlessCommand(page: Page, id: string, keys: string): Promise<void> {
  const before = await page.evaluateHandle(() => document.activeElement);
  await page.keyboard.press(chord("settings"));
  await page.locator('[data-settings-tab="shortcuts"]').click();
  // The area commands sit in a fold that starts closed.
  const fold = page.locator('[data-settings-fold="area-commands"]');
  if (await fold.locator(`[data-shortcut-record="${id}"]`).count() > 0 && !(await fold.evaluate((node) => (node as HTMLDetailsElement).open))) await fold.locator("summary").click();
  await expect(page.locator(`[data-shortcut-effective="${id}"]`)).toHaveText("-");
  await page.locator(`[data-shortcut-record="${id}"]`).click();
  await page.keyboard.press(keys);
  await expect(page.locator(`[data-shortcut-problem="${id}"]`)).toHaveCount(0);
  await expect(page.locator(`[data-shortcut-effective="${id}"]`)).not.toHaveText("-");
  await page.locator("[data-settings-close]").click();
  await expect(page.locator("[data-settings]")).toHaveCount(0);
  await expect.poll(() => before.evaluate((node) => node === document.activeElement)).toBe(true);
}

/** Shows the Explorer in the tool column, beside the documents or alone (PRD lane-two-columns B4, B5). */
export async function showExplorer(page: Page): Promise<void> {
  await showTool(page, "explorer");
  await expect(page.locator('[data-lane="explorer"] [data-explorer]')).toBeVisible();
}

/**
 * What must not move when a row changes state (PRD sidebar-readability B2,
 * B4): the row's height, where the row after it starts, and the edge each
 * named part is anchored at: the left edge of the title, the right edge of
 * the elapsed time, whose text grows as the seconds count. Rounded to the
 * device pixel, since a sub-pixel difference is not a visible move.
 */
export async function rowGeometry(row: Locator, next: Locator, parts: Locator[]): Promise<number[]> {
  const box = await row.boundingBox();
  const after = await next.boundingBox();
  if (!box || !after) throw new Error("a measured row is not on screen");
  const xs: number[] = [];
  for (const part of parts) {
    const partBox = await part.boundingBox();
    if (!partBox) throw new Error("a measured part is not on screen");
    const rightAnchored = (await part.getAttribute("data-agent-elapsed")) !== null;
    xs.push(Math.round(rightAnchored ? partBox.x + partBox.width : partBox.x));
  }
  return [Math.round(box.height), Math.round(after.y), ...xs];
}

/**
 * A sidebar row of a project, shown: a project with no agent rows is folded
 * under No agents (PRD sidebar-titlebar-refresh B1), so the fold is opened
 * when the row is not drawn. The fold is the core's, so it stays open.
 */
export async function showProjectRow(page: Page, row: Locator): Promise<Locator> {
  const fold = page.locator('[data-no-agent-projects][aria-expanded="false"]');
  await expect(row.or(fold).first()).toBeVisible({ timeout: 20_000 });
  if (!(await row.isVisible())) await fold.click();
  await expect(row).toBeVisible();
  return row;
}

/** Opens the one sidebar project whose agents all sleep, which folds until opened (PRD sidebar-titlebar-refresh B4). */
export async function openSleepingProject(page: Page): Promise<void> {
  // A project is folded by its own toggle; a plain folder, which has no project row, by its checkout's.
  const toggle = page.locator('nav[data-sidebar] [data-project-toggle][aria-expanded="false"], nav[data-sidebar] [data-project]:not(:has([data-project-row])) [data-checkout-toggle][aria-expanded="false"]');
  await expect(toggle).toHaveCount(1, { timeout: 30_000 });
  await toggle.click();
  await expect(toggle).toHaveCount(0);
}

/** Puts the pointer on the canvas and drops keyboard focus, so no row is hovered or focused. */
export async function rest(page: Page): Promise<void> {
  await page.mouse.move(900, 600);
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
}

/**
 * Keyboard focus on a control, the way Tab puts it there: a key press first,
 * so the browser draws `:focus-visible` rather than treating it as a click's focus.
 */
export async function keyboardFocus(page: Page, control: Locator): Promise<void> {
  await page.keyboard.press("Shift");
  await control.focus();
  expect(await control.evaluate((element) => element.matches(":focus-visible"))).toBe(true);
}

// The sidebar's geometry is measured in one module, shared with the design
// review command, so a spec and a review judge the same boxes.
export { sidebarColumns, sidebarOverflow, sidebarRowsFit } from "./sidebar-geometry.mjs";

/**
 * Sends one client event on a socket of its own, as a second client would,
 * and returns the first frame `answered` accepts, or null right after the
 * send when no answer is awaited. A spec registers a folder this way when
 * the dialog is not what it tests.
 */
export async function sendEvent(
  page: Page,
  daemon: { origin: string; token: string },
  kind: string,
  payload: Record<string, unknown>,
  answered?: string,
): Promise<Record<string, unknown> | null> {
  return page.evaluate(
    async ({ origin, token, kind, payload, answered }) =>
      new Promise<Record<string, unknown> | null>((resolve, reject) => {
        const ws = new WebSocket(`${origin.replace(/^http/, "ws")}/ws`);
        const timer = setTimeout(() => reject(new Error(`${kind}: no ${answered ?? "handshake"} frame`)), 15_000);
        let sent = false;
        ws.onerror = () => reject(new Error(`${kind}: socket failed`));
        ws.onopen = () => ws.send(JSON.stringify({ token, schema_version: 2 }));
        ws.onmessage = (message) => {
          if (!sent) {
            sent = true;
            ws.send(JSON.stringify({ schema_version: 2, kind, payload }));
            if (answered) return;
          } else {
            const frame = JSON.parse(String(message.data)) as Record<string, unknown>;
            if (frame.type !== answered) return;
            clearTimeout(timer);
            ws.close();
            resolve(frame);
            return;
          }
          clearTimeout(timer);
          ws.close();
          resolve(null);
        };
      }),
    { origin: daemon.origin, token: daemon.token, kind, payload, answered },
  );
}

/** Registers a folder of this machine the way Add a project does: one `create_workspace`. */
export async function registerFolder(page: Page, daemon: { origin: string; token: string }, folder: string): Promise<void> {
  await sendEvent(page, daemon, "create_workspace", { path: folder, label: path.basename(folder), initialize_git: false });
}

/** Chooses Light or Dark in Settings > General, closes Settings and waits for the colors to settle. */
export async function chooseTheme(page: Page, theme: "light" | "dark"): Promise<void> {
  await page.keyboard.press(chord("settings"));
  await expect(page.locator('[data-settings="true"]')).toBeVisible();
  await page.locator('[data-settings-tab="general"]').click();
  await page.locator(`[data-theme-option="${theme}"]`).click();
  await expect(page.locator("html")).toHaveClass(new RegExp(`\\b${theme}\\b`));
  await page.keyboard.press("Escape");
  await expect(page.locator('[data-settings="true"]')).toHaveCount(0);
  // Controls fade their colors into the new theme; a capture waits them out.
  await animationsFinished(page);
}
