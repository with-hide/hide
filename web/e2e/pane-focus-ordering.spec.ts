import { skipOwnWindowsAgentStart } from "./own-windows-agent-start";
import { expect, test, type CDPSession, type Page } from "@playwright/test";
import fs from "node:fs";
import { corePaneFocus, withControl } from "./daemon-actions";
import { startFixture } from "./engine-fixture";
import { herdrGate } from "./herdr-gate";
import { herdrHasFocus, startHerdr } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { enterWorkspace, screenshot } from "./wire";

/** The pane ids Herdr was asked to focus since the gate was armed, in order. */
const focused = (gate: Awaited<ReturnType<typeof herdrGate>>) => gate.params("pane.focus").map((params) => params.pane_id);

type Diagnostic = { kind: string; message: string; occurred_at: number };

/** The diagnostics list the page is sent, with a count of every entry of a
 * kind it has ever been sent and of every frame. The list is capped and drops
 * from its front as a burst appends, so the number of entries of one kind in
 * it falls and cannot say how many arrived. */
function observeDiagnostics(page: Page): (() => Diagnostic[]) & { added: (kind: string) => number; frames: () => number } {
  let diagnostics: Diagnostic[] = [];
  let frames = 0;
  const added = new Map<string, number>();
  const same = (a: Diagnostic, b: Diagnostic) => a.kind === b.kind && a.message === b.message && a.occurred_at === b.occurred_at;
  page.on("websocket", (socket) => socket.on("framereceived", ({ payload }) => {
    frames += 1;
    const frame = JSON.parse(String(payload)) as {
      type: string; payload?: { rest?: { status?: { diagnostics?: Diagnostic[] } } };
    };
    if ((frame.type === "snapshot" || frame.type === "delta") && frame.payload?.rest?.status?.diagnostics) {
      const next = frame.payload.rest.status.diagnostics;
      // The list drops from its front and appends at its back, so what is
      // new is whatever follows the longest run of the old tail that the new
      // list starts with.
      let kept = Math.min(diagnostics.length, next.length);
      while (kept > 0 && !diagnostics.slice(diagnostics.length - kept).every((entry, i) => same(entry, next[i]!))) kept -= 1;
      for (const entry of next.slice(kept)) added.set(entry.kind, (added.get(entry.kind) ?? 0) + 1);
      diagnostics = next;
    }
  }));
  return Object.assign(() => diagnostics, { added: (kind: string) => added.get(kind) ?? 0, frames: () => frames });
}

/** Passive evidence only: retain the last 64 outgoing focus messages without
 * changing the page's send, focus actions, or the test's behavior assertions. */
function observeFocusFrames(page: Page) {
  const limit = 64;
  const records: { occurred_at: number; kind: string; pane_id: string }[] = [];
  let evicted = 0;
  let lastPane: string | undefined;
  let changes = 0;
  let completedClicks = 0;
  page.on("websocket", (socket) => socket.on("framesent", ({ payload }) => {
    if (typeof payload !== "string") return;
    let frame: { kind?: string; payload?: { pane_id?: string; action?: string } };
    try {
      const parsed: unknown = JSON.parse(payload);
      if (!parsed || typeof parsed !== "object") return;
      frame = parsed as typeof frame;
    } catch {
      // Unstructured traffic supplies no focus evidence.
      return;
    }
    const kind = frame.kind;
    if (kind === "terminal_click") {
      completedClicks += 1;
      return;
    }
    const paneId = frame.payload?.pane_id;
    if (kind !== "focus_pane" && kind !== "remote_control") return;
    if (kind === "remote_control" && frame.payload?.action !== "focus_pane") return;
    if (typeof paneId !== "string") return;
    if (paneId !== lastPane) {
      changes += 1;
      lastPane = paneId;
    }
    if (records.length === limit) {
      records.shift();
      evicted += 1;
    }
    records.push({ occurred_at: Date.now(), kind, pane_id: paneId });
  }));
  return {
    completedClicks: () => completedClicks,
    changes: () => changes,
    lastPane: () => lastPane,
    save: () => test.info().attach("pane-focus-sent", {
      contentType: "application/json",
      body: Buffer.from(JSON.stringify({ record_limit: limit, evicted, records })),
    }),
  };
}

/** The page on `daemon`'s two panes, with the clicks and the facts the burst tests read: the same on either engine. */
async function focusHarness(page: Page, daemon: Daemon, panes: readonly string[], focusFrames: ReturnType<typeof observeFocusFrames>, mark: (stage: string) => void) {
  const diagnostics = observeDiagnostics(page);
  await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
  await enterWorkspace(page, "fixture");
  const [first] = panes as [string, string];
  const views = panes.map((pane) => page.locator(`[data-pane-view="${pane}"]`));
  for (const pane of views) await expect(pane).toHaveAttribute("data-transport", "controlling");
  const boxes = await Promise.all(views.map((pane) => pane.boundingBox()));
  // These polls read captured diagnostics in memory, without browser or
  // engine I/O. Observe acceptance promptly within the unchanged timeout.
  const focusObservation = { intervals: [25] };
  let lastClickAt = 0;
  const click = async (index: number) => {
    const box = boxes[index]!;
    lastClickAt = Date.now();
    await page.mouse.click(box.x + 100, box.y + 100);
  };
  const clickBurst = async (mouse: CDPSession) => {
    const completedAfter = focusFrames.completedClicks() + 40;
    const inputs: Promise<unknown>[] = [];
    lastClickAt = Date.now();
    // Use the same trusted Chromium input as page.mouse, queued in order.
    // Awaiting 40 separate browser round trips can outlast the held
    // request's real five-second deadline on a loaded macOS runner.
    for (let i = 0; i < 20; i += 1) {
      for (const index of [1, 0]) {
        const box = boxes[index]!;
        const point = { x: box.x + 100, y: box.y + 100 };
        inputs.push(mouse.send("Input.dispatchMouseEvent", { ...point, type: "mouseMoved", buttons: 0 }));
        inputs.push(mouse.send("Input.dispatchMouseEvent", { ...point, type: "mousePressed", button: "left", buttons: 1, clickCount: 1 }));
        inputs.push(mouse.send("Input.dispatchMouseEvent", { ...point, type: "mouseReleased", button: "left", buttons: 0, clickCount: 1 }));
      }
    }
    mark("cdp.inputs.before");
    await Promise.all(inputs);
    mark("cdp.inputs.after");
    // ACKs from the separate CDP session do not fence frame observation.
    // Each real mouseup sends terminal_click after its focus event on the
    // same WebSocket. The last click fences every earlier focus frame.
    mark("cdp.processed.before");
    await expect.poll(() => focusFrames.completedClicks(), focusObservation).toBe(completedAfter);
    mark("cdp.processed.after");
  };
  const requested = () => diagnostics().filter((entry) => entry.kind === "pane.focus.requested");
  const confirmed = () => {
    const last = diagnostics().filter((entry) => entry.kind.startsWith("pane.focus")).at(-1);
    return last?.kind === "pane.focus"
      && last.message.startsWith(`Pane ${first} focus confirmed`)
      && last.occurred_at >= lastClickAt;
  };
  return { diagnostics, views, focusObservation, click, clickBurst, requested, confirmed };
}

/** Hide on two panes of the platform's Herdr, behind the gate, with the clicks
 * and the facts the burst tests read. `stop` ends everything that started. */
async function startFocusStack(page: Page, label: string, mark: (stage: string) => void = () => {}) {
  const herdr = await startHerdr();
  const focusFrames = observeFocusFrames(page);
  let gate: Awaited<ReturnType<typeof herdrGate>> | undefined;
  let daemon: Daemon | undefined;
  const stop = async () => {
    try {
      daemon?.stop();
      await gate?.stop();
    } finally {
      try {
        herdr.stop();
      } finally {
        await focusFrames.save();
      }
    }
  };
  try {
    gate = await herdrGate(herdr);
    daemon = await startHided({ ...herdr, socket: gate.socket }, label);
    const [first, second] = herdr.panes;
    const { diagnostics, views: panes, focusObservation, click, clickBurst, requested, confirmed } = await focusHarness(page, daemon, herdr.panes, focusFrames, mark);
    await expect.poll(() => herdrHasFocus(herdr, first)).toBe(true);
    return { herdr, gate, diagnostics, focusFrames, first, second, panes, focusObservation, click, clickBurst, requested, confirmed, stop };
  } catch (error) {
    await stop();
    throw error;
  }
}

// These cases exercise real terminal input/focus through the platform's engine.
// Herdr only: it holds the core's pane.focus request at Herdr's socket and counts what Herdr was asked, which is the coalescing of the core's requests to Herdr.
test("rapid pane clicks coalesce behind one request @herdr", { tag: "@platform" }, async ({ page }) => {
  // Two fixed bursts produce at most 32 stage records, with no input contents.
  const stages: { stage: string; at: number }[] = [];
  const mark = (stage: string) => { stages.push({ stage, at: Date.now() }); };
  let firstFocusAt: number | undefined;
  let mouseSession: CDPSession | undefined;
  const stack = await startFocusStack(page, "focus-ordering", mark);
  const { herdr, gate, diagnostics, focusFrames, first, second, panes, focusObservation, click, clickBurst, requested, confirmed } = stack;
  try {
    // Protocol setup and detach belong outside the held Herdr request.
    // Detach alone consumed two seconds of its five-second budget on macOS.
    mark("cdp.connect.before");
    const mouse = await page.context().newCDPSession(page);
    mouseSession = mouse;
    mark("cdp.connect.after");

    mark("gate.arm");
    const held = gate.arm("pane.focus");
    mark("first.click.before");
    await click(1);
    mark("first.click.after");
    await held;
    mark("gate.held");
    mark("first.requested.before");
    await expect.poll(() => requested().at(-1)?.message, focusObservation).toBe(`Focusing pane ${second}`);
    firstFocusAt = requested()[0]?.occurred_at;
    mark("first.requested.after");
    await expect.poll(() => focusFrames.completedClicks(), focusObservation).toBe(1);
    const acceptedBefore = requested().length;
    const sentBefore = focusFrames.changes();
    await clickBurst(mouse);
    expect(focusFrames.lastPane()).toBe(first);
    // Count changes actually sent by the UI, not clicks: focusing an already
    // focused textarea sends nothing, and repeated targets share one intent.
    // Observe every accepted change before releasing the gate; a stale first-
    // pane snapshot or a later focus diagnostic cannot satisfy this barrier.
    const acceptedAfter = acceptedBefore + focusFrames.changes() - sentBefore;
    mark("burst.accepted.before");
    await expect.poll(() => requested().length, focusObservation).toBe(acceptedAfter);
    mark("burst.accepted.after");
    expect(requested().at(-1)?.message).toBe(`Focusing pane ${first}`);
    expect(diagnostics().some((entry) => entry.kind === "pane.focus.unknown")).toBe(false);
    expect(focused(gate)).toEqual([second]);
    mark("focused.attribute.before");
    await expect(panes[0]).toHaveAttribute("data-focused", "true");
    mark("focused.attribute.after");
    mark("gate.release.before");
    await gate.release();
    mark("gate.release.after");
    mark("first.confirmed.before");
    await expect.poll(confirmed, focusObservation).toBe(true);
    mark("first.confirmed.after");
    mark("cdp.detach.before");
    await mouse.detach();
    mouseSession = undefined;
    mark("cdp.detach.after");
    expect(focused(gate)).toEqual([second, first]);
    expect(gate.maximum("pane.focus")).toBe(1);
    expect(herdrHasFocus(herdr, first)).toBe(true);
  } finally {
    try {
      mark("cdp.detach.before");
      await mouseSession?.detach();
      mark("cdp.detach.after");
    } finally {
      console.log("[focus-burst-timing]", JSON.stringify({
        first_focus_at: firstFocusAt,
        stages: stages.map(({ stage, at }) => ({ stage, at, since_first_focus_ms: firstFocusAt === undefined ? null : at - firstFocusAt })),
      }));
      await stack.stop();
    }
  }
});

test("keys typed after rapid pane clicks reach the last pane", { tag: "@platform" }, async ({ page }) => {
  skipOwnWindowsAgentStart();
  const fixture = await startFixture();
  const focusFrames = observeFocusFrames(page);
  let daemon: Daemon | undefined;
  let mouse: CDPSession | undefined;
  try {
    daemon = await startHided(fixture, "focus-keys");
    const live = daemon;
    const [first] = fixture.panes;
    const { diagnostics, views: panes, focusObservation, clickBurst, confirmed } = await focusHarness(page, live, fixture.panes, focusFrames, () => {});
    await withControl(live, (control) => control.until((rest) => corePaneFocus(rest) === first || undefined, `the focus of ${first}`, 20_000));
    mouse = await page.context().newCDPSession(page);
    // The page has sent every click when the burst returns, not hided accepted
    // them: confirming a request that was not the last one is not the burst's
    // end. Every focus change the page sent is observed accepted, so the last
    // confirmation is the last click's.
    const requestedBefore = diagnostics.added("pane.focus.requested");
    const sentBefore = focusFrames.changes();
    await clickBurst(mouse);
    await expect.poll(() => diagnostics.added("pane.focus.requested"), focusObservation).toBe(requestedBefore + focusFrames.changes() - sentBefore);
    await expect.poll(confirmed, focusObservation).toBe(true);
    // Playwright sees a frame on the wire before the page's handler runs it, so
    // every frame seen has to have reached the page before its focus is read.
    await expect.poll(() => page.evaluate(() => window.__hideProbe?.arrivals() ?? 0), focusObservation).toBeGreaterThanOrEqual(diagnostics.frames());
    // The keys go to the terminal that holds the keyboard, which follows the snapshot.
    await expect(panes[0].locator(".xterm-helper-textarea")).toBeFocused();
    await expect(panes[0]).toHaveAttribute("data-focused", "true");
    await withControl(live, (control) => control.until((rest) => corePaneFocus(rest) === first || undefined, `the focus of ${first}`, 20_000));
    const marker = "FOCUS_ORDERING_INPUT";
    await page.keyboard.type(marker);
    await expect.poll(() => fs.readFileSync(fixture.inputLogs[0], "utf8")).toContain(marker);
    expect(fs.readFileSync(fixture.inputLogs[1], "utf8")).not.toContain(marker);
    await screenshot(page, "pane-focus-ordering-last-click");
  } finally {
    try {
      await mouse?.detach();
    } finally {
      try {
        daemon?.stop();
      } finally {
        try {
          fixture.stop();
        } finally {
          await focusFrames.save();
        }
      }
    }
  }
});

// Herdr only: it moves the focus in Herdr outside Hide, which Hide must follow.
test("a real external Herdr focus is followed @herdr", { tag: "@platform" }, async ({ page }) => {
  const stack = await startFocusStack(page, "focus-external");
  const { herdr, diagnostics, first, panes } = stack;
  try {
    await expect(panes[0]).toHaveAttribute("data-focused", "true");
    const startedAt = Date.now();
    herdr.run(["pane", "focus", "--direction", "right", "--pane", first]);
    await expect(panes[1]).toHaveAttribute("data-focused", "true");
    await expect.poll(() => diagnostics().some((entry) => entry.kind === "pane.focus.followed" && entry.occurred_at >= startedAt)).toBe(true);
  } finally {
    await stack.stop();
  }
});

// Herdr only: it holds a pane.focus request at Herdr's socket until Herdr's answer is unknown, then delivers it late.
test("an unknown focus result ends the burst before a delayed mutation is released @herdr", { tag: "@platform" }, async ({ page }) => {
  const herdr = await startHerdr();
  const focusFrames = observeFocusFrames(page);
  let gate: Awaited<ReturnType<typeof herdrGate>> | undefined;
  let daemon: Daemon | undefined;
  try {
    gate = await herdrGate(herdr);
    daemon = await startHided({ ...herdr, socket: gate.socket }, "focus-unknown");
    const diagnostics = observeDiagnostics(page);
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    const [last, delayed] = herdr.panes;
    const panes = herdr.panes.map((pane) => page.locator(`[data-pane-view="${pane}"]`));
    for (const pane of panes) await expect(pane).toHaveAttribute("data-transport", "controlling");
    await expect.poll(() => herdrHasFocus(herdr, last)).toBe(true);
    const boxes = await Promise.all(panes.map((pane) => pane.boundingBox()));
    const click = async (index: number) => {
      const box = boxes[index]!;
      await page.mouse.click(box.x + 100, box.y + 100);
    };
    const held = gate.arm("pane.focus");
    const started = Date.now();
    await click(1);
    await held;
    await click(0);
    await expect(panes[0]).toHaveAttribute("data-focused", "true");

    // Hold until the actual existing five-second transport budget reports
    // unknown, rather than releasing after an arbitrary delay.
    await expect.poll(() => diagnostics().some((entry) => entry.kind === "pane.focus.unknown"), { timeout: 10_000 }).toBe(true);
    expect(focused(gate)).toEqual([delayed]);
    expect(diagnostics().some((entry) => entry.kind === "pane.focus" && entry.occurred_at >= started)).toBe(false);
    expect(herdrHasFocus(herdr, last)).toBe(true);
    await expect(panes[0]).toHaveAttribute("data-focused", "true");

    // A timeout did not cancel the sent effect. Deliver it to real Herdr now
    // and observe the documented late external move, without a queued B.
    await gate.release();
    await expect.poll(() => herdrHasFocus(herdr, delayed)).toBe(true);
    await expect(panes[1]).toHaveAttribute("data-focused", "true");
    await expect.poll(() => diagnostics().some((entry) => entry.kind === "pane.focus.followed" && entry.occurred_at >= started)).toBe(true);
    expect(focused(gate)).toEqual([delayed]);
    expect(gate.maximum("pane.focus")).toBe(1);
    await screenshot(page, "pane-focus-unknown-late-effect");

    // A later explicit user selection starts a fresh, confirmed burst.
    const retryAt = Date.now();
    await click(0);
    await expect.poll(() => diagnostics().some((entry) => entry.kind === "pane.focus"
      && entry.message.startsWith(`Pane ${last} focus confirmed`) && entry.occurred_at >= retryAt)).toBe(true);
    await expect.poll(() => herdrHasFocus(herdr, last)).toBe(true);
    await expect(panes[0]).toHaveAttribute("data-focused", "true");
    expect(focused(gate)).toEqual([delayed, last]);
  } finally {
    daemon?.stop();
    await gate?.stop();
    herdr.stop();
    await focusFrames.save();
  }
});
