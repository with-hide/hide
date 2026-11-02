// Cmd+F reaches the whole of what a pane shows, on an isolated engine:
// a pane whose history the engine holds is searched by the find bar, which scrolls
// a match above the screen into view, and a full-screen agent, whose history
// the engine does not hold, is handed to the agent's own search.

import { expect, test, type Page } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { sendKeys, withControl } from "./daemon-actions";
import { startFixture } from "./engine-fixture";
import { herdrGate } from "./herdr-gate";
import { paneCommandLine, startHerdr } from "./herdr-fixture";
import { fixtureProgram } from "./platform-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { countSent, enterWorkspace, screenshot } from "./wire";
import { chord } from "./chords";
import { unchangedForFrames } from "./wait";

test.describe.configure({ timeout: 120_000 });
test.use({ actionTimeout: 15_000 });

async function paneText(page: Page, pane: string): Promise<string> {
  return page.evaluate((id) => window.__hideProbe?.paneText(id) ?? "", pane);
}

/** The pane whose terminal holds the page's keyboard, or null. */
async function keyboardPane(page: Page): Promise<string | null> {
  return page.evaluate(() => document.activeElement?.closest("[data-pane-view]")?.getAttribute("data-pane-view") ?? null);
}

// @platform: Keys reach an agent pane's PTY through the platform's engine.
test("Cmd+F scrolls the focused pane to a match above the screen", { tag: "@platform" }, async ({ page }) => {
  await page.setViewportSize({ width: 1680, height: 900 });
  const fixture = await startFixture({ agents: false });
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(fixture, "find-scroll");
    const [, focused] = fixture.panes;
    // 160 rows from a program, so the first rows are history above the screen.
    const rows = fixtureProgram(path.join(fixture.root, "bin"), "print-rows", 'for (let i = 1; i <= 160; i++) console.log("row-" + String(i).padStart(3, "0"));');
    await withControl(daemon, async (control) => sendKeys(control, focused, `${paneCommandLine(fixture.shell, [rows])}\r`));
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");

    const view = page.locator(`[data-pane-view="${focused}"]`);
    await expect.poll(() => paneText(page, focused), { timeout: 15_000 }).toContain("row-160");
    await view.locator("[data-terminal-host]").click();
    await expect(view).toHaveAttribute("data-focused", "true");
    // The screen shows the last rows: the first ones are above it.
    expect(await paneText(page, focused)).not.toContain("row-005");

    await page.keyboard.press(chord("find_in_pane"));
    const bar = page.locator("[data-find-bar]");
    await bar.locator("input").fill("row-005");
    await page.keyboard.press("Enter");
    await expect(bar).toContainText("1/1");
    await expect.poll(() => paneText(page, focused), { timeout: 10_000 }).toContain("row-005");
    await screenshot(page, "pane-find-scrolled");
  } finally {
    daemon?.stop();
    fixture.stop();
  }
});

test("Cmd+F on a full-screen agent opens the agent's own search instead of the find bar", async ({ page }) => {
  await page.setViewportSize({ width: 1680, height: 900 });
  const fixture = await startFixture();
  let daemon: Daemon | null = null;
  try {
    // The first agent has printed nothing, so the engine holds no history for it,
    // as for an agent drawing on the alternate screen.
    daemon = await startHided(fixture, "agent-find");
    const [agent] = fixture.panes;
    const last = new Map<string, Record<string, unknown>>();
    const sent = countSent(page, last);
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    // The fixture gave this pane the keyboard when it made it; a click would send
    // the agent a mouse report, so the pane is not clicked.
    const view = page.locator(`[data-pane-view="${agent}"]`);
    await expect(view).toHaveAttribute("data-focused", "true", { timeout: 20_000 });
    const before = fs.readFileSync(fixture.inputLogs[0], "latin1").length;

    await page.keyboard.press(chord("find_in_pane"));
    // The agent hears Claude Code's transcript key and its search key, and
    // nothing else; Hide's own bar stays closed and the keyboard stays on the pane.
    // JSON keeps the control byte readable in a failure.
    await expect.poll(() => JSON.stringify(fs.readFileSync(fixture.inputLogs[0], "latin1").slice(before)), { timeout: 10_000 }).toBe(JSON.stringify("\x0f/"));
    await expect(page.locator("[data-find-bar]")).toHaveCount(0);
    expect(last.get("pane_find_open")).toMatchObject({ pane_id: agent });
    expect(sent.get("pane_find") ?? 0).toBe(0);
    await expect.poll(() => keyboardPane(page)).toBe(agent);
  } finally {
    daemon?.stop();
    fixture.stop();
  }
});

// Herdr only: the late answer is ordered by holding a request at Herdr's socket, which the core's own engine has none of.
test("an agent search answer that lands after the keyboard moved leaves the keyboard where it went @herdr", async ({ page }) => {
  await page.setViewportSize({ width: 1680, height: 900 });
  const herdr = await startHerdr();
  let gate: Awaited<ReturnType<typeof herdrGate>> | undefined;
  let daemon: Daemon | null = null;
  try {
    const [agent, other] = herdr.panes;
    gate = await herdrGate(herdr);
    daemon = await startHided({ ...herdr, socket: gate.socket }, "agent-find-late");
    const last = new Map<string, Record<string, unknown>>();
    countSent(page, last);
    const received: string[] = [];
    page.on("websocket", (socket) => socket.on("framereceived", ({ payload }) => { received.push(String(payload)); }));
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    await expect(page.locator(`[data-pane-view="${agent}"]`)).toHaveAttribute("data-focused", "true", { timeout: 20_000 });
    await expect.poll(() => keyboardPane(page)).toBe(agent);

    // ⌘F asks the core where the agent's search goes, and its answer waits
    // on Herdr: the keys it sends the agent are held at the socket.
    const held = gate.arm("pane.send_keys");
    await page.keyboard.press(chord("find_in_pane"));
    await held;
    const request = String(last.get("pane_find_open")?.request_id);
    // Meanwhile the operator moves on to the other pane.
    await page.locator(`[data-pane-view="${other}"] [data-terminal-host]`).click({ position: { x: 40, y: 60 } });
    await expect(page.locator(`[data-pane-view="${other}"]`)).toHaveAttribute("data-focused", "true");
    expect(received.some((frame) => frame.includes(`{"request_id":"${request}",`))).toBe(false);

    // The answer lands after the move: the agent's search opened in its pane,
    // and the keyboard stays where the operator took it.
    await gate.release();
    await expect.poll(() => received.some((frame) => frame.includes(`{"request_id":"${request}","route":"agent"}`))).toBe(true);
    await unchangedForFrames(page, () => keyboardPane(page));
    expect(await keyboardPane(page)).toBe(other);
    const agentHeard = fs.readFileSync(herdr.inputLogs[0], "latin1").length;
    await page.keyboard.type("LATE_FIND_ANSWER");
    await expect.poll(() => fs.readFileSync(herdr.inputLogs[1], "latin1"), { timeout: 10_000 }).toContain("LATE_FIND_ANSWER");
    expect(fs.readFileSync(herdr.inputLogs[0], "latin1").slice(agentHeard)).not.toContain("LATE");
  } finally {
    daemon?.stop();
    await gate?.stop();
    herdr.stop();
  }
});
