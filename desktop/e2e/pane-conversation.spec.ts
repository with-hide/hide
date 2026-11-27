// The operator reads and replies over a live pane, then returns to its unchanged
// terminal. Only a native shell over a real holder can prove that round trip.
import { expect, type ElectronApplication } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { agentIn, withControl } from "../../web/e2e/daemon-actions";
import { continueFixtureTranscript } from "../../web/e2e/pane-fixture";
import { countSent, enterWorkspace, sendEvent } from "../../web/e2e/wire";
import { animationsFinished, compositorPresents } from "../../web/e2e/wait";
import { fitWindow, launchShell, startStack, test } from "./fixture";
import { captureNativeWindow } from "./native-window";
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
      await captureNativeWindow(app, `pane-conversation-${theme}`, facts);
    }
    await toggle.click();
    await expect(cover).toHaveCount(0);
    await expect(host.locator("textarea")).toBeFocused();
    expect(await page.evaluate((id) => (window as unknown as { __hideProbe: Probe }).__hideProbe.paneGrid(id), paneId)).toEqual(grid);
    expect(counts.get("terminal_resize")).toBe(resized);
    expect(counts.get("terminal_viewport")).toBe(viewed);
    await sendEvent(page, daemon, "interface_language_set", { language: "ko" });
    await sendEvent(page, daemon, "ui_state_update", { font_size: 17 });
    await fitWindow(app, { width: 1000, height: 800 });
    await toggle.click();
    await expect(composer).toBeVisible();
    await compositorPresents(page);
    await captureNativeWindow(app, "pane-conversation-narrow-max-ko", facts);
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
    await captureNativeWindow(first, "pane-conversation-taken", { panes: fixture.panes, profiles: 2, state: run.env.HIDE_STATE_DIR });
    await reclaim.click();
    await expect(reclaim).toHaveCount(0);
    await expect(another.page.locator("[data-thread-reclaim]")).toBeVisible();
  } catch (error) { failures.push(error); }
  await finishNative(failures, [["close the second app", () => second?.close()], ["close the first app", () => first?.close()], ["stop the stack", () => stack.stop()]]);
});
