import { expect, test, type Locator, type Page } from "@playwright/test";
import { startFixture } from "./engine-fixture";
import { corePaneFocus, withControl } from "./daemon-actions";
import { startHided, type Daemon } from "./hided-fixture";
import { enterWorkspace } from "./wire";
import { chord } from "./chords";

type Operator = { client_id: string; sequence: number };
type Frame = { type?: string; payload?: { rest?: { focused?: { pane_id?: string; operator_focus?: Operator[] } } } };

/** Waits until the core holds the keyboard on `pane`, the one focus there is on either engine. */
async function coreFocusOn(daemon: Daemon, pane: string) {
  await withControl(daemon, (control) => control.until((rest) => corePaneFocus(rest) === pane || undefined, `the core's focus on ${pane}`, 30_000));
}

/** The pane's terminal is attached to the page and controls it. */
async function attached(pane: Locator) {
  await expect(pane).toHaveAttribute("data-transport", "controlling");
}

/** Two frames on: the passive effect that follows a snapshot's focused pane has run by then. */
const settle = (page: Page) => page.evaluate(() => new Promise<void>((done) => requestAnimationFrame(() => requestAnimationFrame(() => done()))));

/** The page's socket has read every frame handed to it, and the effects that follow have run. */
async function read(page: Page, net: { handed: () => number }) {
  await expect.poll(() => page.evaluate(() => window.__hideProbe?.arrivals())).toBe(net.handed());
  await settle(page);
}

/**
 * The page's socket with the daemon's frames held back until the test hands
 * them over, in order. The test decides which snapshot the page has seen when
 * the operator's last click is already sent, so no timing is involved.
 */
async function holdSnapshots(page: Page) {
  const frames: string[] = [];
  const sent: Operator[] = [];
  let holding = false;
  let handed = 0;
  let toPage: ((message: string | Buffer) => void) | undefined;
  await page.routeWebSocket(/\/ws$/, (socket) => {
    const server = socket.connectToServer();
    toPage = (message) => {
      handed += 1;
      socket.send(message);
    };
    socket.onMessage((message) => {
      if (typeof message === "string") {
        const event = JSON.parse(message) as { kind?: string; payload?: Partial<Operator> };
        if (event.kind === "focus_pane" && event.payload?.client_id) sent.push(event.payload as Operator);
      }
      server.send(message);
    });
    server.onMessage((message) => {
      if (holding) frames.push(message as string);
      else toPage?.(message);
    });
  });
  const applied = (frame: string, client: string) =>
    (JSON.parse(frame) as Frame).payload?.rest?.focused?.operator_focus?.find((entry) => entry.client_id === client)?.sequence ?? 0;
  return {
    hold: () => { holding = true; },
    sent: () => sent,
    /** Frames handed to the page so far; the page's probe counts the ones its socket has read. */
    handed: () => handed,
    /** Held frames that already include operator focus number `sequence`. */
    includes: (sequence: number) => frames.filter((frame) => applied(frame, sent[0]?.client_id ?? "") >= sequence).length,
    /** Hands the page every held frame before the first one that includes `sequence`; with none given, all of them. */
    deliver: (sequence?: number) => {
      const client = sent[0]?.client_id ?? "";
      const stop = sequence === undefined ? frames.length : frames.findIndex((frame) => applied(frame, client) >= sequence);
      for (const frame of frames.splice(0, stop === -1 ? frames.length : stop)) toPage?.(frame);
    },
  };
}

test("a snapshot older than the last click does not take the keys back, and the one that includes it keeps them", { tag: "@platform" }, async ({ page }) => {
  const fixture = await startFixture();
  let daemon: Daemon | undefined;
  try {
    daemon = await startHided(fixture, "stale-snapshot-focus");
    const net = await holdSnapshots(page);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    const panes = fixture.panes.map((pane) => page.locator(`[data-pane-view="${pane}"]`));
    const keys = panes.map((pane) => pane.locator(".xterm-helper-textarea"));
    for (const pane of panes) await attached(pane);
    await coreFocusOn(daemon, fixture.panes[0]);
    await expect(keys[0]).toBeFocused();
    const boxes = await Promise.all(panes.map((pane) => pane.boundingBox()));
    const click = (index: number) => page.mouse.click(boxes[index]!.x + 100, boxes[index]!.y + 100);

    net.hold();
    await click(1);
    await expect.poll(() => net.sent().length).toBe(1);
    await expect.poll(() => net.includes(1)).toBeGreaterThan(0);
    await click(0);
    await expect.poll(() => net.sent().length).toBe(2);
    await expect.poll(() => net.includes(2)).toBeGreaterThan(0);
    await expect(keys[0]).toBeFocused();

    // The page has sent click 2; the daemon's answers so far include only
    // click 1, which named the other pane. They arrive and the header follows
    // them, but the keys stay where the operator last clicked.
    net.deliver(2);
    await expect(panes[1]).toHaveAttribute("data-focused", "true");
    await settle(page);
    await expect(keys[0]).toBeFocused();
    await expect(keys[1]).not.toBeFocused();

    // The snapshot that includes click 2 agrees with the keys.
    net.deliver();
    await expect(panes[0]).toHaveAttribute("data-focused", "true");
    await settle(page);
    await expect(keys[0]).toBeFocused();
  } finally {
    daemon?.stop();
    fixture.stop();
  }
});

test("the answer to a click on the focused pane leaves the keys where the operator took them since", { tag: "@platform" }, async ({ page }) => {
  const fixture = await startFixture();
  let daemon: Daemon | undefined;
  try {
    daemon = await startHided(fixture, "stale-snapshot-answer");
    const net = await holdSnapshots(page);
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    const pane = page.locator(`[data-pane-view="${fixture.panes[0]}"]`);
    const keys = pane.locator(".xterm-helper-textarea");
    const agents = page.locator('nav[data-sidebar="projects"]');
    const sidebarHasKeys = () => agents.evaluate((node) => node.contains(document.activeElement));
    await attached(pane);
    await coreFocusOn(daemon, fixture.panes[0]);
    await expect(keys).toBeFocused();
    await page.keyboard.press(chord("sidebar_projects"));
    await expect.poll(sidebarHasKeys).toBe(true);

    // A click back into the pane that is already focused is still a click the
    // core answers; the operator leaves for the sidebar before the answer lands.
    net.hold();
    // The focused checkout's interactive card can cover a fixed offset.
    // Target the terminal itself so the single click reaches its screen.
    await pane.locator(".xterm-screen").click();
    await expect(keys).toBeFocused();
    await expect.poll(() => net.sent().length).toBe(1);
    await expect.poll(() => net.includes(1)).toBeGreaterThan(0);
    await page.keyboard.press(chord("sidebar_projects"));
    await expect.poll(sidebarHasKeys).toBe(true);

    // The answer moves no pane, so it moves no keys.
    net.deliver();
    await read(page, net);
    expect(await sidebarHasKeys()).toBe(true);
    await expect(pane).toHaveAttribute("data-focused", "true");
  } finally {
    daemon?.stop();
    fixture.stop();
  }
});

test("a focus move that waited for the last click is dropped once the operator takes the keys elsewhere", { tag: "@platform" }, async ({ page }) => {
  const fixture = await startFixture();
  let daemon: Daemon | undefined;
  try {
    daemon = await startHided(fixture, "stale-snapshot-waited");
    const net = await holdSnapshots(page);
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    const panes = fixture.panes.map((pane) => page.locator(`[data-pane-view="${pane}"]`));
    const keys = panes[1].locator(".xterm-helper-textarea");
    const agents = page.locator('nav[data-sidebar="projects"]');
    const sidebarHasKeys = () => agents.evaluate((node) => node.contains(document.activeElement));
    for (const pane of panes) await attached(pane);
    await coreFocusOn(daemon, fixture.panes[0]);
    const box = (await panes[1].boundingBox())!;

    // Two clicks on the other pane, with a visit to the sidebar between them
    // so the second one is sent too.
    net.hold();
    await page.mouse.click(box.x + 100, box.y + 100);
    await expect.poll(() => net.includes(1)).toBeGreaterThan(0);
    await page.keyboard.press(chord("sidebar_projects"));
    await expect.poll(sidebarHasKeys).toBe(true);
    await page.mouse.click(box.x + 100, box.y + 100);
    await expect(keys).toBeFocused();
    await expect.poll(() => net.includes(2)).toBeGreaterThan(0);

    // The answer to the first click moves the focus onto the pane while the
    // second is still unanswered, so the move waits; the operator leaves for
    // the sidebar, and the answer to the second click finds the keys gone.
    net.deliver(2);
    await read(page, net);
    await expect(panes[1]).toHaveAttribute("data-focused", "true");
    await expect(keys).toBeFocused();
    await page.keyboard.press(chord("sidebar_projects"));
    await expect.poll(sidebarHasKeys).toBe(true);
    net.deliver();
    await read(page, net);
    expect(await sidebarHasKeys()).toBe(true);
    await expect(panes[1]).toHaveAttribute("data-focused", "true");
  } finally {
    daemon?.stop();
    fixture.stop();
  }
});

test("opening the pane that is already focused from the sidebar takes the keyboard", { tag: "@platform" }, async ({ page }) => {
  const fixture = await startFixture();
  let daemon: Daemon | undefined;
  try {
    daemon = await startHided(fixture, "open-focused-from-sidebar");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    const pane = page.locator(`[data-pane-view="${fixture.panes[0]}"]`);
    const keys = pane.locator(".xterm-helper-textarea");
    const agents = page.locator('nav[data-sidebar="projects"]');
    const sidebarHasKeys = () => agents.evaluate((node) => node.contains(document.activeElement));
    await attached(pane);
    await coreFocusOn(daemon, fixture.panes[0]);
    await expect(keys).toBeFocused();
    await page.keyboard.press(chord("sidebar_projects"));
    await expect.poll(sidebarHasKeys).toBe(true);

    // The row of the focused pane: the core moves nothing, and the keys still
    // leave the row for the terminal, which shows its outline again.
    await page.locator(`nav[data-sidebar] [data-agent-open="${fixture.panes[0]}"]`).first().click();
    await expect(keys).toBeFocused();
    await expect(pane).toHaveAttribute("data-focused", "true");
    await settle(page);
    await expect(keys).toBeFocused();
  } finally {
    daemon?.stop();
    fixture.stop();
  }
});
