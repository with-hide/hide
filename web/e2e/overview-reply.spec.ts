// A reply to an agent from the Overview without opening its session (issue
// 2513): the 나를 기다리는 것 item and the session detail each carry a field
// whose words reach the agent's pane as the phone's reply does (`pane_reply`),
// while 답하러 가기 and 세션 열기 stay.
import fs from "node:fs";
import path from "node:path";
import { expect, test, type Locator } from "@playwright/test";
import { startFixture, type AgentFixture } from "./engine-fixture";
import { setAgentState } from "./fixture-agents";
import { startHided, type Daemon } from "./hided-fixture";
import { openOverview } from "./overview-entry";
import { enterWorkspace } from "./wire";

test.describe.configure({ timeout: 150_000 }); // wait-ok: the test budget, which starts the engine, the daemon and two stand-ins

/** What the stand-in of pane `index` read after `from` bytes of its input log. */
function heard(fixture: AgentFixture, index: 0 | 1, from: number): string {
  const log = fixture.inputLogs[index];
  return fs.existsSync(log) ? fs.readFileSync(log).subarray(from).toString("utf8") : "";
}

function logLength(fixture: AgentFixture, index: 0 | 1): number {
  const log = fixture.inputLogs[index];
  return fs.existsSync(log) ? fs.readFileSync(log).length : 0;
}

/** A capture of one panel for the pull request, when the run asks for captures. */
async function capture(panel: Locator, name: string) {
  const dir = process.env.HIDE_E2E_SCREENSHOT_DIR;
  if (dir) await panel.screenshot({ path: path.join(dir, `${name}.png`) });
}

test("a reply from the Overview's waiting list and from the session detail reaches the agent without opening its session", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const fixture = await startFixture();
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(fixture, "overview-reply");
    const [asking] = fixture.panes;
    await setAgentState(daemon, asking, "blocked");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    await openOverview(page, "work");
    const graph = page.locator("[data-overview-graph]");
    await expect(graph).toHaveAttribute("data-graph-phase", "ready", { timeout: 30_000 }); // wait-ok: the graph's first layout on a cold page

    // The waiting item: its question, the reply field, and 답하러 가기 still there.
    const item = graph.locator(`[data-graph-ask="${asking}"]`);
    await expect(item).toBeVisible();
    await expect(item.getByRole("button", { name: "Go answer" })).toBeVisible();
    const field = item.locator(`[data-agent-reply="${asking}"]`);
    const input = field.getByRole("textbox");
    await expect(input).toBeVisible();
    await capture(graph.locator("[data-graph-projects]"), "waiting-item");

    const before = logLength(fixture, 0);
    await input.fill("2");
    await capture(graph.locator("[data-graph-projects]"), "waiting-item-typed");
    await input.press("Enter");
    // The core's receipt says the engine took the words (the field empties),
    // they reached the agent's own pane, and no session opened.
    await expect(input).toHaveValue("");
    await expect(field).toHaveAttribute("data-agent-reply-state", "idle");
    await expect.poll(() => heard(fixture, 0, before)).toMatch(/^2[\r\n]+$/);
    await expect(page.locator("[data-overview-page]")).toBeVisible();

    // The session detail: the same field beside 세션 열기.
    await item.locator(".og-ask-who").click();
    const preview = graph.locator("[data-graph-preview]");
    await expect(preview).toBeVisible();
    await expect(preview.getByRole("button", { name: "Open session" })).toBeVisible();
    const detail = preview.locator(`[data-agent-reply="${asking}"]`);
    await expect(detail).toBeVisible();
    await capture(preview, "session-detail");
    const again = logLength(fixture, 0);
    await detail.getByRole("textbox").fill("1");
    await detail.getByRole("button", { name: "Send" }).click();
    await expect(detail.getByRole("textbox")).toHaveValue("");
    await expect.poll(() => heard(fixture, 0, again)).toMatch(/^1[\r\n]+$/);
    await expect(page.locator("[data-overview-page]")).toBeVisible();
  } finally {
    daemon?.stop();
    fixture.stop();
  }
});
