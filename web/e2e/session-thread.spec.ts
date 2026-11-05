// A pane's session read as a thread on the engine and hided (PRD
// session-docs-and-assistant B25, B29): the node reads the record, reads on
// as the session writes, and the input box's words reach that pane's program.
// The Claude and Pi panes run on both engines; the Pi stand-in reports its
// session file as Pi's own integration does (`prepareEngineWriter`).
import { expect, test, type Page } from "@playwright/test";
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { agentIn, preparePaneInput, sendKeys, withControl } from "./daemon-actions";
import { startFixture } from "./engine-fixture";
import { setAgentState } from "./fixture-agents";
import { continueFixtureTranscript } from "./pane-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { nativeSessionId, prepareEngineWriter } from "./session-reader-fixture";
import { sendEvent } from "./wire";
import { afterCleanup } from "./worker-owned";
import type { SessionThreadHistorySection, SessionThreadSection } from "../src/snapshot";

test.describe.configure({ timeout: 150_000 });

/** The thread's two sections as the page's own socket last received them. */
function watchThread(page: Page): { thread: () => SessionThreadSection | null; words: () => string } {
  let thread: SessionThreadSection | null = null;
  let history: SessionThreadHistorySection | null = null;
  page.on("websocket", (socket) => socket.on("framereceived", (frame) => {
    if (typeof frame.payload !== "string") return;
    const incoming = JSON.parse(frame.payload) as { type: string; payload?: { session_thread?: SessionThreadSection | null; session_thread_history?: SessionThreadHistorySection | null } };
    if ((incoming.type !== "snapshot" && incoming.type !== "delta") || !incoming.payload) return;
    if ("session_thread" in incoming.payload) thread = incoming.payload.session_thread ?? null;
    if ("session_thread_history" in incoming.payload) history = incoming.payload.session_thread_history ?? null;
  }));
  // The words the screen draws: this thread's kept pages, then the live one.
  const words = () => [...(history && history.generation === thread?.generation ? history.pages : []), ...(thread?.tail ? [thread.tail] : [])]
    .flatMap((page) => page.updates).map((update) => (update.content as { text?: string } | undefined)?.text ?? "").join("\n");
  return { thread: () => thread, words };
}

test("a Claude pane's record reads as a thread, follows what the session writes, and takes the input box's words", async ({ page }) => {
  const { thread, words } = watchThread(page);
  const fixture = await startFixture();
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(fixture, "claude-thread");
    const pane = fixture.panes[0];
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await expect(page.locator(`nav[data-sidebar] [data-pane="${pane}"]`)).toBeVisible({ timeout: 30_000 });

    await sendEvent(page, daemon, "session_thread_open", { pane_id: pane });
    await expect.poll(() => thread()?.state, { timeout: 30_000 }).toBe("ready");
    expect(thread()).toMatchObject({ pane_id: pane, pane: "same", connected: true });
    expect(words()).toContain("Agent one 진행해줘");

    // The session writes on while it works: the thread reads what it wrote.
    continueFixtureTranscript(fixture, "fixture-one", { task: "스레드가 따라 읽는 새 말" });
    await setAgentState(daemon, pane, "working");
    await expect.poll(() => words(), { timeout: 30_000 }).toContain("스레드가 따라 읽는 새 말");

    // The input box's words reach the pane's program, with Enter, and nowhere else.
    const request = crypto.randomUUID();
    await sendEvent(page, daemon, "thread_input", { generation: thread()!.generation, pane_id: pane, request_id: request, text: "계속해 주세요", stages: [] });
    await expect.poll(() => thread()?.input, { timeout: 30_000 }).toEqual({ request_id: request, state: "sent" });
    await expect.poll(() => fs.readFileSync(fixture.inputLogs[0], "utf8"), { timeout: 10_000 }).toContain("계속해 주세요");
    expect(fs.existsSync(fixture.inputLogs[1]) ? fs.readFileSync(fixture.inputLogs[1], "utf8") : "").not.toContain("계속해 주세요");
  } finally {
    daemon?.stop();
    fixture.stop();
  }
});

test("a Pi pane's record reads as a thread, follows what the session writes, and takes the input box's words", async ({ page }) => {
  const { thread, words } = watchThread(page);
  const fixture = await startFixture({ agents: false });
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(fixture, "pi-thread", path.join(fixture.root, "home"));
    const pane = fixture.panes[0];
    const session = prepareEngineWriter(fixture, "pi");
    await withControl(daemon, async (control) => sendKeys(control, pane, "pi\r"));
    await expect.poll(() => fs.existsSync(session)).toBe(true);
    await withControl(daemon, (control) => control.until((rest) => (agentIn(rest, pane)?.agent_kind === "pi" ? true : undefined), `a pi agent in ${pane}`, 30_000));
    // The stand-in reported Pi's own session.
    await withControl(daemon, (control) => control.until((rest) => (agentIn(rest, pane)?.session_id === nativeSessionId("pi") ? true : undefined), `${pane} on its pi session`, 30_000));
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await expect(page.locator(`nav[data-sidebar] [data-pane="${pane}"]`)).toBeVisible({ timeout: 30_000 });

    await sendEvent(page, daemon, "session_thread_open", { pane_id: pane });
    await expect.poll(() => thread()?.state, { timeout: 30_000 }).toBe("ready");
    expect(thread()).toMatchObject({ pane_id: pane, pane: "same", connected: true });
    expect(words()).toContain("요청 보기를 만들어줘");

    // The session writes on while it works: the thread reads what it wrote.
    const record = { type: "message", id: "a0000004", parentId: "a0000003", timestamp: "2026-10-03T01:01:00Z", message: { role: "assistant", content: [{ type: "text", text: "스레드가 따라 읽는 새 말" }], stopReason: "stop" } };
    fs.appendFileSync(session, `${JSON.stringify(record)}\n`);
    await withControl(daemon, async (control) => {
      await preparePaneInput(control, pane);
      sendKeys(control, pane, "\x1b[2J\x1b[HWorking...\r\n");
      await control.until((rest) => agentIn(rest, pane)?.activity === "working" ? true : undefined, `${pane} working`, 30_000);
    });
    await expect.poll(() => words(), { timeout: 30_000 }).toContain("스레드가 따라 읽는 새 말");

    // The input box's words reach the pane's program, with Enter, and nowhere else.
    const request = crypto.randomUUID();
    await sendEvent(page, daemon, "thread_input", { generation: thread()!.generation, pane_id: pane, request_id: request, text: "계속해 주세요", stages: [] });
    await expect.poll(() => thread()?.input, { timeout: 30_000 }).toEqual({ request_id: request, state: "sent" });
    await expect.poll(() => fs.readFileSync(fixture.inputLogs[0], "utf8"), { timeout: 10_000 }).toContain("계속해 주세요");
    expect(fs.existsSync(fixture.inputLogs[1]) ? fs.readFileSync(fixture.inputLogs[1], "utf8") : "").not.toContain("계속해 주세요");
  } catch (error) {
    throw afterCleanup(afterCleanup(error, () => daemon?.stop()), () => fixture.stop());
  }
  try { daemon?.stop(); } catch (error) { throw afterCleanup(error, () => fixture.stop()); }
  fixture.stop();
});
