// The operator reads and replies over a live pane, then returns to its unchanged
// terminal. Only a native shell over a real holder can prove that round trip.
import { expect, type ElectronApplication, type Locator, type Page } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { field } from "../../web/e2e/chords";
import { agentIn, withControl } from "../../web/e2e/daemon-actions";
import { continueFixtureTranscript } from "../../web/e2e/pane-fixture";
import { countSent, enterWorkspace, sendEvent, showExplorer } from "../../web/e2e/wire";
import { animationsFinished, compositorPresents } from "../../web/e2e/wait";
import { fitWindow, launchShell, startStack, test } from "./fixture";
import { captureElectronWindow } from "./electron-window";
import { finishNative } from "./native-teardown";
import { withProbe, type Probe } from "./terminal-probe";

test("a pane conversation with Desk off replies in place and keeps its terminal grid and attach", async () => {
  const stack = await startStack("pane-conversation");
  let app: ElectronApplication | undefined;
  const failures: unknown[] = [];
  try {
    const { fixture, run, daemon } = stack;
    const paneId = fixture.panes[0]!;
    continueFixtureTranscript(fixture, "fixture-one", { task: "한국어로 긴 제목을 가진 세션에서 대화를 읽고 답한 뒤 터미널로 돌아가기" });
    await withControl(daemon, (control) => control.until((rest) => agentIn(rest, paneId)?.session_id === "fixture-one" || undefined, "a ready thread reader"));
    const opened = await launchShell(run.env); app = opened.app;
    const page = opened.page, counts = countSent(page);
    await fitWindow(app, { width: 1440, height: 1000 });
    await withProbe(page);
    await enterWorkspace(page, "fixture");
    const toggle = page.locator(`[data-pane-conversation-toggle="${paneId}"]`);
    await expect(toggle).toBeEnabled();
    const host = page.locator(`[data-terminal-host="${paneId}"]`);
    await expect.poll(() => page.evaluate((id) => (window as unknown as { __hideProbe: Probe }).__hideProbe.paneGrid(id), paneId)).not.toBeNull();
    await animationsFinished(page); await compositorPresents(page);
    const grid = await page.evaluate((id) => (window as unknown as { __hideProbe: Probe }).__hideProbe.paneGrid(id), paneId);
    const box = await host.boundingBox(), resized = counts.get("terminal_resize"), viewed = counts.get("terminal_viewport");
    await toggle.click();
    const cover = page.locator(`[data-pane-conversation="${paneId}"]`), composer = cover.locator("[data-desk-composer-input]");
    await expect(toggle).toHaveAttribute("aria-pressed", "true");
    await expect(cover).toContainText("Agent one 진행해줘");
    await expect(composer).toBeFocused();
    await composer.fill("이 pane에만 보내는 답장");
    await composer.press("Enter");
    await expect.poll(() => fs.readFileSync(fixture.inputLogs[0]!, "utf8")).toContain("이 pane에만 보내는 답장");
    expect(fs.existsSync(fixture.inputLogs[1]!) ? fs.readFileSync(fixture.inputLogs[1]!, "utf8") : "").not.toContain("이 pane에만 보내는 답장");
    await expect(composer).toHaveValue("");
    await animationsFinished(page); await compositorPresents(page);
    expect(await host.boundingBox()).toEqual(box);
    expect(counts.get("terminal_resize")).toBe(resized);
    expect(counts.get("terminal_viewport")).toBe(viewed);
    const facts = { paneId, grid, box, home: run.env.HOME, state: run.env.HIDE_STATE_DIR, resizeEvents: resized, viewportEvents: viewed, provider: "synthetic Claude record and real pane input" };
    for (const theme of ["light", "dark"] as const) {
      await sendEvent(page, daemon, "theme_set", { theme });
      await expect(page.locator("html")).toHaveClass(new RegExp(`\\b${theme}\\b`));
      await compositorPresents(page);
      await captureElectronWindow(app, `pane-conversation-${theme}`, facts);
    }
    await toggle.click();
    await expect(cover).toHaveCount(0);
    await expect(host.locator("textarea")).toBeFocused();
    expect(await page.evaluate((id) => (window as unknown as { __hideProbe: Probe }).__hideProbe.paneGrid(id), paneId)).toEqual(grid);
    expect(counts.get("terminal_resize")).toBe(resized);
    expect(counts.get("terminal_viewport")).toBe(viewed);
    // Returning from a tool gives the visible conversation the keyboard;
    // ordinary typing must stay in its draft until the operator sends it.
    await toggle.click();
    await showExplorer(page);
    await page.locator("[data-explorer-tree]").focus();
    await page.keyboard.press("Escape");
    await expect(composer).toBeFocused();
    await page.keyboard.type("focus-probe-before-enter");
    await expect(composer).toHaveValue("focus-probe-before-enter");
    expect(fs.readFileSync(fixture.inputLogs[0]!, "utf8")).not.toContain("focus-probe-before-enter");
    await toggle.click();
    await sendEvent(page, daemon, "interface_language_set", { language: "ko" });
    await sendEvent(page, daemon, "ui_state_update", { font_size: 17 });
    await fitWindow(app, { width: 1000, height: 800 });
    await toggle.click();
    await expect(composer).toBeVisible();
    await compositorPresents(page);
    await captureElectronWindow(app, "pane-conversation-narrow-max-ko", facts);
  } catch (error) { failures.push(error); }
  await finishNative(failures, [["close the app", () => app?.close()], ["stop the stack", () => stack.stop()]]);
});

// Two independent native profiles share one candidate core, as two windows do.
// A hook unit test covers reconnect; this journey observes the actual shared hold.
test("another window takes the thread once and only an explicit press reclaims it", async () => {
  test.setTimeout(150_000); // wait-ok: two cold native profiles and the core's ready thread in each.
  const stack = await startStack("pane-conversation-taken");
  let first: ElectronApplication | undefined, second: ElectronApplication | undefined;
  const failures: unknown[] = [];
  try {
    const { fixture, run } = stack;
    const opened = await launchShell(run.env); first = opened.app;
    const page = opened.page;
    await enterWorkspace(page, "fixture");
    await page.locator(`[data-pane-conversation-toggle="${fixture.panes[0]}"]`).click();
    await expect(page.locator("[data-pane-conversation]")).toContainText("Agent one 진행해줘");
    const another = await launchShell({ ...run.env, HIDE_DESKTOP_USER_DATA_DIR: path.join(run.env.HOME!, "second-window") }); second = another.app;
    await enterWorkspace(another.page, "fixture");
    await another.page.locator(`[data-pane-conversation-toggle="${fixture.panes[1]}"]`).click();
    const reclaim = page.locator("[data-thread-reclaim]");
    await expect(reclaim).toBeVisible();
    await expect(page.locator("[data-thread-taken=true]")).toHaveAttribute("inert", "");
    await expect(page.locator("[data-pane-conversation]")).toContainText("Agent one 진행해줘");
    await compositorPresents(page);
    await captureElectronWindow(first, "pane-conversation-taken", { panes: fixture.panes, profiles: 2, state: run.env.HIDE_STATE_DIR });
    await reclaim.click();
    await expect(reclaim).toHaveCount(0);
    await expect(another.page.locator("[data-thread-reclaim]")).toBeVisible();
  } catch (error) { failures.push(error); }
  await finishNative(failures, [["close the second app", () => second?.close()], ["close the first app", () => first?.close()], ["stop the stack", () => stack.stop()]]);
});

/**
 * Drags from the start of `from` down past the rows drawn, scrolling the conversation while the button is held, and
 * lets go on the lowest answer standing in view; answers that answer's number.
 */
async function dragAcrossMessages(page: Page, scroll: Locator, from: Locator): Promise<number> {
  // The first visual line of the message's first line, where its words begin.
  const start = await from.locator(".cm-line").first().evaluate((line) => {
    const words = document.createRange();
    words.selectNodeContents(line);
    const first = words.getClientRects()[0]!;
    return { x: first.left + 1, y: first.top + first.height / 2 };
  });
  await page.mouse.move(start.x, start.y);
  await page.mouse.down();
  const box = (await scroll.boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height - 4, { steps: 5 });
  const before = await scroll.evaluate((element) => element.scrollTop);
  await page.mouse.wheel(0, 3 * box.height);
  await expect.poll(() => scroll.evaluate((element) => element.scrollTop)).toBeGreaterThan(before + box.height);
  const end = await scroll.evaluate((element) => {
    const view = element.getBoundingClientRect();
    const lines = [...element.querySelectorAll<HTMLElement>('[data-desk-thread-line="agent"] .cm-line')].filter((line) => {
      const rect = line.getBoundingClientRect();
      return rect.top >= view.top && rect.bottom <= view.bottom;
    });
    const last = lines.at(-1)!;
    const rect = last.getBoundingClientRect();
    return { x: rect.right - 2, y: rect.top + rect.height / 2, n: Number(/History (\d+):/.exec(last.textContent ?? "")?.[1]) };
  });
  await page.mouse.move(end.x, end.y, { steps: 5 });
  await page.mouse.up();
  return end.n;
}

// The conversation draws only the rows near its scroll viewport (issue 3101).
// A long history stays readable, the reading position holds while answers
// arrive, and a selection dragged past the rows drawn copies every message.
test("long conversation history draws the rows near the reader, keeps the reading position and copies a selection across messages", async () => {
  const stack = await startStack("pane-conversation-history");
  let app: ElectronApplication | undefined;
  const failures: unknown[] = [];
  try {
    const { fixture, run } = stack;
    const at = Date.now() - 120_000;
    for (let index = 0; index < 120; index++) continueFixtureTranscript(fixture, "fixture-one", { task: `History ${index}: **한국어 대화**를 읽고 돌아와요`, at: at + index * 1000 });
    const opened = await launchShell(run.env); app = opened.app;
    const page = opened.page;
    await fitWindow(app, { width: 1440, height: 1000 });
    await enterWorkspace(page, "fixture");
    await page.locator(`[data-pane-conversation-toggle="${fixture.panes[0]}"]`).click();
    const cover = page.locator("[data-pane-conversation]");
    const scroll = cover.locator("[data-desk-thread-conversation]");
    const answer = (n: number) => cover.locator('[data-desk-thread-line="agent"]').filter({ hasText: `History ${n}:` });
    const first = answer(0), last = answer(119);
    await expect(last.locator(".cm-editor")).toHaveCount(1);
    await expect(cover.locator("[data-desk-thread-lines]")).toHaveAttribute("data-desk-thread-lines", "242");
    // Every line is read (the fixture's own first turn and these 120), and only the rows near the foot are drawn.
    await expect(first).toHaveCount(0);
    expect(await cover.locator("[data-thread-row]").count()).toBeLessThan(60);
    // A real scroll, rather than scrollIntoView's implicit programmatic jump,
    // establishes the reading position before the next answer arrives.
    await scroll.evaluate((element) => { element.scrollTop = 0; element.dispatchEvent(new Event("scroll")); });
    await expect(first.locator(".cm-editor")).toHaveCount(1);
    await expect(last).toHaveCount(0);
    await expect(first).toBeInViewport();
    const firstEditor = await first.locator(".cm-editor").elementHandle();
    const composer = cover.locator("[data-desk-composer-input]");
    await composer.fill("읽던 곳을 유지해요");
    expect(await first.locator(".cm-editor").evaluate((element, before) => element === before, firstEditor)).toBe(true);
    continueFixtureTranscript(fixture, "fixture-one", { task: "History 120: 새 답변이 왔어요", at: Date.now() });
    await expect(cover.locator("[data-desk-thread-lines]")).toHaveAttribute("data-desk-thread-lines", "244");
    await expect(first).toBeInViewport();
    await compositorPresents(page);
    await captureElectronWindow(app, "pane-conversation-history", { turns: 121, state: run.env.HIDE_STATE_DIR, condition: "reading oldest answer while a new answer arrives" });
    // A drag from the oldest answer past the rows drawn selects every message between, and the copy holds them all.
    const held = await app.evaluate(({ clipboard }) => clipboard.readText());
    try {
      const reached = await dragAcrossMessages(page, scroll, first);
      expect(reached).toBeGreaterThan(5);
      await page.keyboard.press(field("KeyC"));
      await expect.poll(() => app!.evaluate(({ clipboard }) => clipboard.readText())).toContain(`History ${reached}:`);
      const copied = await app.evaluate(({ clipboard }) => clipboard.readText());
      for (let n = 0; n <= reached; n++) expect(copied, `History ${n}`).toContain(`History ${n}:`);
    } finally {
      await app.evaluate(({ clipboard }, text) => clipboard.writeText(text), held);
    }
    await expect(composer).toHaveValue("읽던 곳을 유지해요");
    // While the selection stands its rows stay drawn, even with the conversation at its foot.
    await scroll.evaluate((element) => { element.scrollTop = element.scrollHeight; element.dispatchEvent(new Event("scroll")); });
    await expect(answer(120).locator(".cm-editor")).toHaveCount(1);
    await expect(first).toHaveCount(1);
    // A press in the box clears it, and the rows it held are let go.
    await composer.click();
    await expect.poll(() => page.evaluate(() => document.getSelection()?.isCollapsed ?? true)).toBe(true);
    await expect(first).toHaveCount(0);
    continueFixtureTranscript(fixture, "fixture-one", { task: "History 121: 아래에서는 새 글을 따라가요", at: Date.now() + 1000 });
    const following = answer(121);
    await expect(following.locator(".cm-editor")).toHaveCount(1);
    await expect(following).toBeInViewport();
  } catch (error) { failures.push(error); }
  await finishNative(failures, [["close the app", () => app?.close()], ["stop the stack", () => stack.stop()]]);
});
