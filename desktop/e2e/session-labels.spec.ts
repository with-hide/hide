// Session boundaries for the labels the core makes (PRD labels-in-hided D-04,
// B5, B10; the session-label-isolation behaviors). A pane's label belongs to
// the native session it was analyzed from: a new session shows nothing of the
// last one, an analysis that lands after its session was replaced is never
// shown, returning to a session labels it from that session alone, and a
// daemon restart restores the label with no new provider request.
//
// Everything is real but the provider's words: the transcripts are synthetic
// Claude transcripts, the fixture `claude` answers with the label each one
// carries, and the engine's session reports, the daemon's read and analysis,
// and the native window are the product's own.

import { expect, type ElectronApplication, type Page } from "@playwright/test";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { setAgentSession } from "../../web/e2e/fixture-agents";
import { claudeProjects, writeFixtureTranscript } from "../../web/e2e/herdr-fixture";
import { enterWorkspace } from "../../web/e2e/wire";
import { launchShell, startStack, test } from "./fixture";
import { compositorPresents, quietFor } from "../../web/e2e/wait";

const A = "세션 A의 한글 작업";
const A_REPLY = "현재 세션 답변 요청";
const B = "세션 B의 한글 작업";
const LATE = "폐기해야 할 지연 결과";

/** Counts every DOM change after which any of `texts` is on the page. */
async function forbid(page: Page, texts: string[]): Promise<void> {
  await page.evaluate((forbidden) => {
    const probe = { count: 0, observer: new MutationObserver(() => {
      if (forbidden.some((text) => document.body.textContent?.includes(text))) probe.count++;
    }) };
    probe.observer.observe(document.body, { subtree: true, childList: true, characterData: true });
    (window as unknown as { forbiddenProbe: typeof probe }).forbiddenProbe = probe;
  }, texts);
}

async function forbiddenSeen(page: Page): Promise<number> {
  return page.evaluate(() => {
    const probe = (window as unknown as { forbiddenProbe: { count: number; observer: MutationObserver } }).forbiddenProbe;
    probe.observer.disconnect();
    return probe.count;
  });
}

test("a label belongs to its session: never shown for another, restored for its own", async () => {
  test.setTimeout(150_000);
  // Each line is one answered provider request; the daemon logs them from its start.
  let calls = "";
  const stack = await startStack("session-labels", { prepare: (_fixture, home) => {
    calls = path.join(home.root, "provider-calls");
    home.env.HIDE_E2E_PROVIDER_LOG = calls;
  } });
  const { fixture, run, daemon } = stack;
  const answered = () => (fs.existsSync(calls) ? fs.readFileSync(calls, "utf8").split("\n").filter(Boolean).length : 0);
  const pane = fixture.panes[0];
  const projects = claudeProjects(fixture);
  writeFixtureTranscript(projects, "session-a", { task: A, reply: A_REPLY, question: true });
  writeFixtureTranscript(projects, "session-b", { task: B });
  writeFixtureTranscript(projects, "session-late", { task: LATE, delayMs: 4_000 });
  let app: ElectronApplication | undefined;
  try {
    await setAgentSession(fixture, daemon, pane, "session-a");
    const open = async () => {
      const launched = await launchShell(run.env);
      app = launched.app;
      const page = launched.page;
      const cdp = await page.context().newCDPSession(page);
      await cdp.send("Emulation.setFocusEmulationEnabled", { enabled: true });
      await page.reload();
      await expect(page.locator("[data-main-screen], [data-workspace-screen]")).toBeVisible({ timeout: 30_000 });
      await enterWorkspace(page, "fixture");
      await page.locator(`[data-pane-view="${pane}"]`).click({ position: { x: 30, y: 60 } });
      const window = await app.evaluate(({ BrowserWindow }) => {
        const windows = BrowserWindow.getAllWindows();
        if (windows.length !== 1) throw new Error("expected one isolated candidate window");
        return { pid: process.pid, source: windows[0]!.getMediaSourceId() };
      });
      return { page, window };
    };
    let { page, window } = await open();
    const capture = async (name: string) => {
      const dir = process.env.HIDE_E2E_SCREENSHOT_DIR;
      if (!dir) return;
      fs.mkdirSync(dir, { recursive: true });
      await compositorPresents(page);
      execFileSync("/usr/sbin/screencapture", ["-x", "-o", "-l", window.source.split(":")[1]!, path.join(dir, `session-labels-${name}.png`)]);
    };
    const tab = page.locator(`[data-tab="${fixture.tab}"]`);
    const row = page.locator(`nav[data-sidebar] [data-checkout-agents-open] [data-pane="${pane}"]`);

    // Session A is analyzed into its title, which its tree row names too.
    await expect(tab).toContainText(A, { timeout: 30_000 });
    await expect(row.locator("[data-agent-title]")).toHaveText(A);
    await capture("a");

    // A new session whose analysis is slow: nothing of A stays, the pane is
    // the provider's name until its own label comes.
    await setAgentSession(fixture, daemon, pane, "session-late");
    await expect(page.getByText(A, { exact: true })).toHaveCount(0, { timeout: 20_000 });
    await expect(page.getByText(A_REPLY, { exact: true })).toHaveCount(0);
    await expect(tab).toContainText("Claude");
    await forbid(page, [A, A_REPLY, LATE]);
    await capture("late-unlabelled");

    // Replaced again before that analysis answers: the late answer is for a
    // session the pane no longer has, and is never drawn.
    await setAgentSession(fixture, daemon, pane, "session-b");
    await expect(tab).toContainText(B, { timeout: 30_000 });
    await quietFor(page, 5_000, "the replaced session's late answer is never drawn");
    await expect(tab).toContainText(B);
    expect(await forbiddenSeen(page)).toBe(0);
    await capture("b");

    // Back to A: B's words go, and A is labelled from A's own transcript.
    await setAgentSession(fixture, daemon, pane, "session-a");
    await expect(page.getByText(B, { exact: true })).toHaveCount(0, { timeout: 20_000 });
    await forbid(page, [B, LATE]);
    await expect(tab).toContainText(A, { timeout: 30_000 });
    await expect(row.locator("[data-agent-title]")).toHaveText(A);
    expect(await forbiddenSeen(page)).toBe(0);
    await capture("a-again");

    // A daemon restart restores A's label from the daemon's own record, with
    // no new provider request.
    const before = answered();
    await app!.close();
    app = undefined;
    run.hide(["stop"]);
    ({ page, window } = await open());
    await expect(page.locator(`[data-tab="${fixture.tab}"]`)).toContainText(A, { timeout: 30_000 });
    await quietFor(page, 2_000, "no later answer replaces the drawn label");
    expect(answered()).toBe(before);
    await capture("restarted");
  } finally {
    await app?.close();
    stack.stop();
  }
});
