// The sidebar's 태스크 list under an ⌥ hold (PRD sidebar-task-lens with
// electron-digit-shortcuts-hints D-02): holding ⌥ numbers the task rows top
// to bottom and moves nothing on them, releasing hides the numbers, ⌥n opens
// the task at that number as one focus_pane, a number with no task sends
// nothing, and the `프로젝트 | 태스크` switch decides which list the numbers
// belong to. A private engine, hided and Electron app.

import { expect, type ElectronApplication, type Locator, type Page } from "@playwright/test";
import { labelAgentPane, setAgentState } from "../../web/e2e/fixture-agents";
import "../../web/src/host";
import { countSent, enterWorkspace, screenshot } from "../../web/e2e/wire";
import { quietFor } from "../../web/e2e/wait";
import { hostLog, launchShell, startStack, test, type Stack } from "./fixture";

let stack: Stack;
let app: ElectronApplication | null = null;

// One stack per test: its panes belong to the daemon that made them.
test.beforeEach(async () => {
  stack = await startStack("task-digits");
});
test.afterEach(async () => {
  const info = test.info();
  if (info.status !== info.expectedStatus) console.log(hostLog(stack.run.env).map((line) => JSON.stringify(line)).join("\n"));
  await app?.close().catch(() => undefined);
  app = null;
  stack.stop();
});

/** The boxes of what must not move while a keycap shows, rounded to the device pixel. */
async function boxes(locators: Locator[]): Promise<string[]> {
  return Promise.all(locators.map(async (locator) => {
    const box = (await locator.boundingBox())!;
    return [box.x, box.y, box.width, box.height].map(Math.round).join(",");
  }));
}

/** After the one expected event, a quiet moment in which no second one arrives. */
async function exactlyOnce(sent: Map<string, number>, kind: string, expected: number, page: Page): Promise<void> {
  await expect.poll(() => sent.get(kind) ?? 0).toBe(expected);
  await quietFor(page, 600, "no second event follows the one counted");
  expect(sent.get(kind) ?? 0).toBe(expected);
}

test("⌥ numbers the 태스크 list's rows without moving them, ⌥n opens that task, and the switch decides which list is numbered", async () => {
  // A pane is a task once it carries a session, so both get one.
  for (const [index, pane] of stack.fixture.panes.entries()) {
    await labelAgentPane(stack.fixture, stack.daemon, pane, { task: `Task ${index + 1}`, progress: `Task ${index + 1} 진행 중` });
    await setAgentState(stack.daemon, pane, "working");
  }
  const launched = await launchShell(stack.run.env);
  app = launched.app;
  const page: Page = launched.page;
  const sent = countSent(page);
  // Observe the new live socket before it connects, reloading the shell the host has already loaded.
  await Promise.all([
    page.waitForEvent("websocket", { predicate: (socket) => new URL(socket.url()).pathname === "/ws", timeout: 5_000 }),
    page.reload(),
  ]);
  const cdp = await page.context().newCDPSession(page);
  await cdp.send("Emulation.setFocusEmulationEnabled", { enabled: true });
  await enterWorkspace(page, "fixture");
  await page.locator("[data-pane-view] .xterm-helper-textarea").first().focus();
  await page.locator('[data-sidebar-lens-option="tasks"]').click();
  const rows = page.locator("[data-task-list] [data-task-row]");
  await expect(rows).toHaveCount(2, { timeout: 30_000 });
  await expect(page.locator("[data-keycap]")).toHaveCount(0);

  // The hold floats a digit on each task row in list order; the rows and their titles keep their boxes.
  const kept = [rows.first(), rows.first().locator('[data-task-focus="row"]'), rows.nth(1)];
  const before = await boxes(kept);
  await page.keyboard.down("Alt");
  const caps = page.locator("[data-task-list] [data-keycap]");
  await expect(caps).toHaveCount(2);
  await expect(caps).toHaveText(["1", "2"]);
  expect(await boxes(kept)).toEqual(before);
  await screenshot(page, "task-digit-hints-held");
  await page.keyboard.up("Alt");
  await expect(page.locator("[data-keycap]")).toHaveCount(0);

  // ⌥n opens the task at that number, one focus_pane, and the row of the pane in front is selected; past the end nothing is sent.
  const order = await rows.evaluateAll((items) => items.map((item) => item.getAttribute("data-task-row")));
  const front = await page.locator('[data-task-row][data-selected="true"]').first().getAttribute("data-task-row");
  const behind = order.findIndex((pane) => pane !== front);
  const opened = sent.get("focus_pane") ?? 0;
  await page.keyboard.press(`Alt+Digit${behind + 1}`);
  await exactlyOnce(sent, "focus_pane", opened + 1, page);
  await expect(page.locator(`[data-task-row="${order[behind]}"][data-selected="true"]`)).toHaveCount(1);
  await page.keyboard.press("Alt+Digit3");
  await quietFor(page, 400, "the shortcut with no task sends nothing");
  expect(sent.get("focus_pane") ?? 0).toBe(opened + 1);

  // Back on 프로젝트 the same hold numbers the tree's rows, not the tasks.
  await page.locator('[data-sidebar-lens-option="projects"]').click();
  const treeRows = page.locator("nav[data-sidebar] li[data-pane]");
  await expect(treeRows).toHaveCount(2);
  await page.keyboard.down("Alt");
  await expect(treeRows.locator("[data-keycap]")).toHaveCount(2);
  await expect(page.locator("[data-task-list]")).toHaveCount(0);
  await page.keyboard.up("Alt");
  await expect(page.locator("[data-keycap]")).toHaveCount(0);
});
