import { expect, test, type Locator, type Page } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { agentIn, sendKeys, withControl } from "./daemon-actions";
import { startFixture } from "./engine-fixture";
import { startHided } from "./hided-fixture";
import { chord } from "./chords";
import { nativeSessionId, PI_ID, prepareEngineWriter } from "./session-reader-fixture";
import { enterWorkspace } from "./wire";

// mission-106 answer, picture section 5: leaving Desk for another screen with
// the recent-place chord (⌃⇥) and coming back keeps where the operator was
// reading in Desk's conversation and in the session beside it, and one read at
// the foot stays at the foot. Each test starts one stack and asserts one
// conversation's place; the conversations are written into the stack's own
// state and session files before the page opens, since the stack runs no
// Desk agent.

const PAIRS = 24;
const words = (n: number) => Array.from({ length: 4 }, (_, line) => `${n}번째 답의 ${line + 1}번째 줄: 지금 돌고 있는 세션과 미션을 정리해서 알려 드려요.`).join("\n\n");

/** Desk's store with a conversation long enough to scroll, Desk switched on. */
function deskStore(): string {
  const items = Array.from({ length: PAIRS }, (_, n) => [
    { kind: "user", id: 2 * n + 1, text: `${n + 1}번째 질문`, at_ms: 1_000 + n, mentions: [] },
    { kind: "reply", id: 2 * n + 2, agent: "claude", text: words(n + 1), at_ms: 1_000 + n, state: "done", thought_ms: null, steps: { sessions: 0, files: 0, letters: 0, starts: 0, other: 0 }, mentions: [], missions: [] },
  ]).flat();
  return JSON.stringify({
    schema_version: 1,
    selection: { agent: "claude", model: null, effort: null },
    settings: { enabled: true, default_agent: "claude", location: "desk", cap: 8, permission: "allow_all" },
    conversation: { generation: 1, dropped: 0, next_id: items.length + 1, items },
    sessions: [],
    choices: [],
    read: true,
    changed_at_ms: 1,
  });
}

/** Pi's session seed with `PAIRS` turns, each answer long enough that the thread scrolls. */
function piSeed(cwd: string): string {
  const timestamp = "2026-10-03T01:00:00Z";
  const records: unknown[] = [{ type: "session", version: 3, id: PI_ID, timestamp, cwd }];
  let parent: string | null = null;
  for (let n = 1; n <= PAIRS; n += 1) {
    const ask = `u${String(n).padStart(7, "0")}`;
    const answer = `a${String(n).padStart(7, "0")}`;
    records.push({ type: "message", id: ask, parentId: parent, timestamp, message: { role: "user", content: `${n}번째 요청` } });
    records.push({ type: "message", id: answer, parentId: ask, timestamp, message: { role: "assistant", content: [{ type: "text", text: words(n) }], stopReason: "stop" } });
    parent = answer;
  }
  return records.map((record) => `${JSON.stringify(record)}\n`).join("");
}

/** The item at the conversation's top edge and how far below the edge its top stands, as Desk remembers it. */
function placeIn(box: Locator): Promise<{ anchor: string; offset: number } | null> {
  return box.evaluate((element) => {
    const top = element.getBoundingClientRect().top;
    for (const item of element.querySelectorAll<HTMLElement>("[data-scroll-anchor]")) {
      const rect = item.getBoundingClientRect();
      if (rect.bottom > top) return { anchor: item.dataset.scrollAnchor!, offset: Math.round(rect.top - top) };
    }
    return null;
  });
}

const footDistance = (box: Locator) => box.evaluate((element) => element.scrollHeight - element.scrollTop - element.clientHeight);

/** Scrolls `box` by the wheel, as the operator does, and waits until it moved. */
async function wheel(page: Page, box: Locator, delta: number) {
  const before = await box.evaluate((element) => element.scrollTop);
  await box.hover();
  await page.mouse.wheel(0, delta);
  await expect.poll(() => box.evaluate((element) => element.scrollTop)).not.toBe(before);
}

/** Leaves Desk for the place before it with the recent-place chord, then comes back to Desk with it. */
async function awayAndBack(page: Page) {
  await page.keyboard.press(chord("recent_area_tab"));
  await expect(page.locator("[data-desk-screen]")).toHaveCount(0);
  await page.keyboard.press(chord("recent_area_tab"));
  await expect(page.locator("[data-desk-screen]")).toBeVisible();
}

test("Desk's conversation comes back to the answer the operator was reading after ⌃⇥ away and back, and to its foot when they read there", async ({ page }) => {
  test.setTimeout(120_000);
  const engine = await startFixture();
  const first = await startHided(engine, "desk-scroll-desk");
  const daemon = await first.restart((stateDir) => {
    fs.mkdirSync(path.join(stateDir, "desk"), { recursive: true });
    fs.writeFileSync(path.join(stateDir, "desk", "desk.json"), deskStore());
  });
  try {
    await page.goto("about:blank");
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    // The agent pane the operator was in is the place the chord goes back to from Desk.
    await page.locator(`[data-pane-view="${engine.panes[0]}"]`).click();
    const places = () => (JSON.parse(fs.readFileSync(path.join(daemon.stateDir, "core-state.json"), "utf8")) as { recent_places?: unknown[] }).recent_places;
    await expect.poll(() => places()?.[0]).toEqual({ kind: "pane", pane_id: engine.panes[0] });
    await page.locator("[data-desk-sidebar-row]").click();
    const box = page.locator("[data-desk-chat-scroll]");
    await expect(box.locator(`[data-scroll-anchor="${2 * PAIRS}"]`)).toBeVisible();
    // Desk opens at its foot; the operator scrolls up to read an older answer.
    await expect.poll(() => footDistance(box)).toBeLessThan(48);
    await wheel(page, box, -2_400);
    const reading = await placeIn(box);
    expect(reading).not.toBeNull();
    expect(await footDistance(box)).toBeGreaterThan(1_000);

    await awayAndBack(page);
    await expect.poll(() => placeIn(box)).toEqual(reading);

    // Read at the foot, it comes back at the foot.
    await wheel(page, box, 100_000);
    await expect.poll(() => footDistance(box)).toBeLessThan(48);
    await awayAndBack(page);
    await expect.poll(() => footDistance(box)).toBeLessThan(48);
  } finally {
    daemon.stop();
    engine.stop();
  }
});

test("the session beside Desk comes back to the turn the operator was reading after ⌃⇥ away and back", async ({ page }) => {
  // Pi's stand-in reports its session for a Unix shell, as desk-room.spec.ts's Pi tests, so no Windows lane runs this.
  test.setTimeout(150_000);
  const engine = await startFixture({ agents: false });
  const daemon = await startHided(engine, "desk-scroll-thread", path.join(engine.root, "home"));
  try {
    const pane = engine.panes[0]!;
    const session = prepareEngineWriter(engine, "pi");
    fs.writeFileSync(path.join(engine.root, "pi-session-seed.jsonl"), piSeed(fs.realpathSync(path.join(engine.root, "fixture"))));
    await withControl(daemon, (control) => sendKeys(control, pane, "pi\r"));
    await expect.poll(() => fs.existsSync(session)).toBe(true);
    await withControl(daemon, (control) => control.until((rest) => (agentIn(rest, pane)?.session_id === nativeSessionId("pi") ? true : undefined), `${pane} on its pi session`, 30_000));
    await page.goto("about:blank");
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    await page.locator(`[data-pane-view="${pane}"]`).click();
    await expect(page.locator(`[data-pane-view="${pane}"]`)).toHaveAttribute("data-focused", "true");
    await page.locator("[data-open-settings]").click();
    await page.locator('[data-settings-tab="labs"]').click();
    await page.locator("[data-desk-use]").click();
    await expect(page.locator("[data-desk-use]")).toHaveAttribute("data-desk-use", "true");
    await page.locator("[data-settings-close]").click();
    await page.locator("[data-desk-sidebar-row]").click();
    // A quiet agent's project can be folded before the page first reads its session.
    const row = page.locator(`[data-agent-open="${pane}"]`);
    const folded = page.locator('nav[data-sidebar] [data-checkout-toggle][aria-expanded="false"]');
    await expect(row.or(folded)).toBeVisible();
    if (!(await row.isVisible())) await folded.click();
    await row.click();
    const box = page.locator(`[data-desk-thread-panel="${pane}"] [data-desk-thread-conversation]`);
    await expect(box.locator("[data-desk-thread-lines]")).toBeVisible({ timeout: 30_000 }); // wait-ok: the core's first read of the Pi session file
    // Every turn is read; only the rows near the viewport are drawn (issue 3101).
    await expect(box.locator("[data-desk-thread-lines]")).toHaveAttribute("data-desk-thread-lines", String(2 * PAIRS));
    await expect.poll(() => footDistance(box)).toBeLessThan(24);
    await wheel(page, box, -1_600);
    const reading = await placeIn(box);
    expect(reading).not.toBeNull();

    await awayAndBack(page);
    if (!(await box.isVisible())) await row.click();
    await expect.poll(() => placeIn(box)).toEqual(reading);
  } finally {
    daemon.stop();
    engine.stop();
  }
});
