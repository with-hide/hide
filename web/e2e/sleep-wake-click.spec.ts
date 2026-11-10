// A sleeping pane's Wake agent takes a mouse click (issue 1279): the controls
// drawn over a terminal sit above every layer xterm draws, so a click at the
// button's center wakes the agent rather than landing in the terminal.
//
// The desktop app draws terminals with WebGL, whose canvases carry their own
// z-index; under the suite's --disable-webgl the DOM renderer has no layer
// over the pane's center and the click would land either way, so this file
// runs a software WebGL context as `terminal-webgl.spec.ts` does.

import { expect, test, type Page } from "@playwright/test";
import { dumpOnFailure } from "./failure-dump";
import { withControl, agentIn } from "./daemon-actions";
import { startFixture } from "./engine-fixture";
import { CLAUDE_FULL_ACCESS } from "./pane-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { screenshot } from "./wire";
import { testBrowserArgs } from "./browser-launch.cjs";

test.use({ launchOptions: { args: testBrowserArgs(["--use-gl=angle", "--use-angle=swiftshader", "--enable-unsafe-swiftshader"]) } });
test.describe.configure({ timeout: 120_000 });

// The session the fixture's second agent runs (`engine-fixture.ts`).
const SESSION = "fixture-two";

test("a mouse click at the center of Wake agent over a WebGL terminal wakes the pane", async ({ page }) => {
  const engine = await startFixture();
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(engine, "sleep-wake-click");
    const [, sleeper] = engine.panes;
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
    await page.locator("[data-checkout]").first().click();
    const pane = page.locator(`[data-pane-view="${sleeper}"]`);
    await expect(pane).toBeVisible({ timeout: 20_000 });

    await page.locator(`[data-pane-menu="${sleeper}"]`).click();
    const item = page.locator('[data-menu-item="sleep_agent"]');
    await expect(item).toBeEnabled({ timeout: 20_000 });
    await item.click();
    const wake = pane.locator(`[data-agent-wake="${sleeper}"]`);
    await expect(wake).toBeVisible({ timeout: 30_000 });
    // The woken agent is the stand-in resumed on the session, typed at the
    // pane's shell with Hide's full-access flag: each start leaves such a
    // line on the screen (the fixture's own start did too), so the wake is
    // one more of them, and the sidebar lists an agent in the pane again.
    // Whitespace is dropped, since the line wraps at the pane's width, and
    // the settings argument is not matched, since each shell quotes it its way.
    const flat = (text: string) => text.replace(/\s+/g, "");
    const line = flat(`claude --resume ${SESSION} ${CLAUDE_FULL_ACCESS[0]} ${CLAUDE_FULL_ACCESS[1]}`);
    const typed = () => page.evaluate((id) => window.__hideProbe?.paneText(id) ?? "", sleeper).then((text) => flat(text).split(line).length - 1);
    const before = await typed();
    // The terminal under the sleeping body draws with WebGL, whose link
    // layer is the canvas the operator's click landed on.
    await expect(pane.locator("canvas.xterm-link-layer")).toHaveCount(1);
    await screenshot(page, "sleep-wake-click-sleeping");

    // A pointer press at the button's center, as the operator clicks;
    // Playwright's locator click would wait for a covering layer instead.
    const box = await wake.boundingBox();
    if (!box) throw new Error("Wake agent has no box");
    await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);

    // Once in a while on Linux the wake's start was answered but no resumed
    // agent ever held the pane (with-hide/hide-private#322); what the shell's
    // line showed then says whether the line was typed, mangled or ran.
    await dumpOnFailure("woken pane", () => wokenPane(page, daemon!, sleeper), async () => {
      await expect.poll(typed, { timeout: 30_000 }).toBeGreaterThan(before);
      await withControl(daemon!, (control) => control.until((rest) => agentIn(rest, sleeper), `the resumed agent in ${sleeper}`, 30_000));
    });
    await expect(pane.locator("[data-pane-sleep]")).toHaveCount(0, { timeout: 30_000 });
  } finally {
    daemon?.stop();
    engine.stop();
  }
});

/** The pane's screen as the page drew it and the agent row the daemon lists on it, as they are now. */
async function wokenPane(page: Page, daemon: Daemon, pane: string): Promise<unknown> {
  const screen = await page.evaluate((id) => window.__hideProbe?.paneText(id) ?? "", pane);
  const agent = await withControl(daemon, (control) => control.until((rest) => agentIn(rest, pane) ?? null, `the agent row of ${pane}`, 2_000));
  return { screen, agent };
}
