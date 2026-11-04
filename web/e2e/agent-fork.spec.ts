import { skipOwnWindowsAgentStart } from "./own-windows-agent-start";
// Fork agent and Fork agent to the side from the pane menu, on an isolated
// engine and hided (PRD fork-operator-root): each starts the pane's
// conversation again as a new session, the operator's own root, which takes
// the keyboard and wears the fork mark naming its source. What is typed
// between the press and the agent's start reaches the fork's agent, not the
// shell that starts it. The disabled items of an agent with no conversation
// yet are the pane menu unit test's and the core's.

import fs from "node:fs";
import path from "node:path";
import { expect, test, type Page } from "@playwright/test";
import { startFixture, type AgentFixture } from "./engine-fixture";
import { agentIn, corePaneFocus, focusTab, newTab, withControl } from "./daemon-actions";
import { setAgentSession } from "./fixture-agents";
import { CLAUDE_FULL_ACCESS } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { countSent, enterWorkspace, screenshot } from "./wire";

test.describe.configure({ timeout: 150_000 });

// The fork's own conversation; the source runs the fixture's second agent, whose transcript names it Agent two.
const FORKED_SESSION = "66666666-7777-8888-9999-000000000000";

/** The panes the sidebar lists an agent in. */
async function agentPanes(daemon: Daemon): Promise<string[]> {
  return withControl(daemon, (control) => control.until((rest) => ((rest.navigator as { agents?: { pane_id: string }[] } | undefined)?.agents ?? []).map((row) => row.pane_id).filter((pane) => agentIn(rest, pane)) as string[], "the listed agents"));
}

/** The pane the core holds the keyboard on. */
function coreFocus(daemon: Daemon): Promise<string | null> {
  return withControl(daemon, (control) => control.until((rest) => corePaneFocus(rest) ?? undefined, "the focused pane", 2_000)).catch(() => null);
}

/** The pane's screen as the page draws it, once it shows the pane. */
function screenOf(page: Page, pane: string): Promise<string> {
  return page.evaluate((id) => window.__hideProbe?.paneText(id) ?? "", pane);
}

/** The source pane runs the fixture's second Claude conversation; hided serves the page. */
async function forkStack(page: Page, herdr: AgentFixture, name: string): Promise<{ daemon: Daemon; forker: string; last: Map<string, Record<string, unknown>> }> {
  await page.setViewportSize({ width: 1440, height: 900 });
  const daemon = await startHided(herdr, name);
  const [, forker] = herdr.panes;
  const last = new Map<string, Record<string, unknown>>();
  countSent(page, last);
  await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
  await enterWorkspace(page, "fixture");
  await expect(page.locator(`[data-pane-view="${forker}"]`)).toBeVisible({ timeout: 20_000 });
  return { daemon, forker, last };
}

/**
 * Presses `item` in the source's pane menu and types a marker as soon as the
 * fork's terminal has the keyboard. Returns the fork's pane once its agent runs
 * the clean launch, how long that took, and whether the marker was typed before.
 */
async function fork(page: Page, herdr: AgentFixture, daemon: Daemon, forker: string, item: "fork_agent" | "fork_agent_side", marker: string, client?: Daemon): Promise<{ forked: string; startMs: number }> {
  const before = await agentPanes(daemon);
  await page.locator(`[data-pane-menu="${forker}"]`).click();
  await expect(page.locator('[data-menu-group="fork"]')).toBeEnabled({ timeout: 20_000 });
  await page.locator('[data-menu-group="fork"]').hover();
  await expect(page.locator('[data-menu-item="fork_agent"]')).toBeEnabled();
  await expect(page.locator('[data-menu-item="fork_agent_side"]')).toBeEnabled();
  // The source is the rightmost pane: the submenu flips left at the window
  // edge. Moving directly into it must not dismiss it or lose the target.
  await expect(page.locator('[data-menu-submenu="fork"]')).toHaveAttribute("data-side", "left");
  await page.locator(`[data-menu-item="${item}"]`).hover();
  await expect(page.locator('[data-menu-submenu="fork"]')).toBeVisible();
  if (item === "fork_agent") await screenshot(page, "agent-fork-menu");
  const pressed = Date.now();
  let heldPane: string | null = null;
  if (client) {
    await page.keyboard.press("Escape");
    heldPane = await page.evaluate(({ origin, token, forker, marker }) => new Promise<string>((resolve, reject) => {
      const ws = new WebSocket(`${origin.replace(/^http/, "ws")}/ws`);
      const timer = setTimeout(() => { ws.close(); reject(new Error("fork never published a pending pane holder")); }, 30_000);
      let requested = false;
      ws.onopen = () => ws.send(JSON.stringify({ token, schema_version: 2 }));
      ws.onerror = () => { clearTimeout(timer); ws.close(); reject(new Error("fork client failed")); };
      ws.onmessage = (message) => {
        if (!requested) {
          requested = true;
          ws.send(JSON.stringify({ schema_version: 2, kind: "fork_pane", payload: { pane_id: forker, placement: "tab", origin: "operator" } }));
          return;
        }
        const frame = JSON.parse(String(message.data));
        const held = frame.payload?.rest?.terminal?.input_requests?.find((row: { state: string; held_for?: string }) => row.state === "pending" && row.held_for);
        if (!held) return;
        // Send on the very frame that announces the hold, before UI focus or
        // a process-info round trip can let the startup finish unnoticed.
        ws.send(JSON.stringify({ schema_version: 2, kind: "key", payload: { pane_id: held.held_for, bytes_base64: btoa(marker) } }));
        clearTimeout(timer);
        ws.close();
        resolve(held.held_for);
      };
    }), { origin: client.origin, token: client.token, forker, marker });
  } else await page.locator(`[data-menu-item="${item}"]`).click();
  const terminalWithFocus = () => page.evaluate(() => document.activeElement?.closest<HTMLElement>("[data-terminal-host]")?.dataset.terminalHost ?? "");
  let focused = "";
  await expect.poll(async () => (focused = await terminalWithFocus()), { timeout: 30_000 }).not.toMatch(new RegExp(`^(${forker})?$`));
  const focusMs = Date.now() - pressed;
  if (client) expect(focused).toBe(heldPane);
  else await page.keyboard.type(marker);
  let forked = "";
  await expect.poll(async () => (forked = (await agentPanes(daemon)).find((pane) => !before.includes(pane)) ?? ""), { timeout: 60_000 }).toBe(focused);
  await expect.poll(() => screenOf(page, focused), { timeout: 30_000 }).toContain("FORK_INPUT_READY");
  const startMs = Date.now() - pressed;
  test.info().annotations.push({ type: `${item} timing`, description: `keyboard on fork ${focusMs} ms, fixture ready ${startMs} ms, sent against pending holder ${heldPane !== null}` });
  console.log(`${item}: keyboard on fork ${focusMs} ms, fixture ready ${startMs} ms; sent against pending holder: ${heldPane !== null}`);
  // Claude Code reports the fork's own conversation id.
  await setAgentSession(herdr, daemon, forked, FORKED_SESSION);
  return { forked, startMs };
}

test("Fork agent accepts bare pane keys from another client and opens right of the source", { tag: "@platform" }, async ({ page }) => {
  skipOwnWindowsAgentStart();
  const herdr = await startFixture();
  let daemon: Daemon | null = null;
  try {
    const stack = await forkStack(page, herdr, "agent-fork-tab");
    daemon = stack.daemon;
    const { forker, last } = stack;
    // A tab after the source's, so the fork's tab has to land between them; the source's tab stays the one shown.
    const after = await withControl(daemon, async (control) => {
      const made = await newTab(control, herdr, "after");
      await focusTab(control, herdr, herdr.tab);
      return made.tab;
    });
    const { forked } = await fork(page, herdr, daemon, forker, "fork_agent", "typed-before-start", daemon);
    expect(last.get("fork_pane")).toMatchObject({ pane_id: forker, placement: "tab" });
    await expect(page.locator(`[data-pane-view="${forked}"]`)).toBeVisible({ timeout: 30_000 });
    await expect.poll(() => coreFocus(daemon!), { timeout: 30_000 }).toBe(forked);
    const strip = page.locator("[data-agent-tab-bar] [data-tab]");
    await expect.poll(async () => (await strip.evaluateAll((tabs) => tabs.map((tab) => (tab as HTMLElement).dataset.tab))).join(" ")).toMatch(new RegExp(`^${herdr.tab} \\S+ ${after}$`));
    // Its keys reached the fork's agent after the clean launch, not the shell.
    const inputLog = path.join(herdr.root, "fork-input.log");
    await expect.poll(() => fs.existsSync(inputLog) ? fs.readFileSync(inputLog, "utf8") : "", { timeout: 10_000 }).toBe("typed-before-start");
    await expect.poll(() => screenOf(page, forked), { timeout: 10_000 }).toContain("FORK_INPUT_READY");
    // The default 전부 허용 reached the fork whole through the pane's own shell, PowerShell on Windows:
    // Claude's settings value is JSON, whose quotes a shell could strip.
    const received = fs.readFileSync(path.join(herdr.root, "fork-argv.jsonl"), "utf8").trim().split("\n").map((line) => JSON.parse(line) as string[]);
    expect(received).toHaveLength(1);
    expect(received[0]!.join("\n")).toContain(CLAUDE_FULL_ACCESS.join("\n"));
    const screen = await screenOf(page, forked);
    expect(screen.slice(0, screen.indexOf("FORK_INPUT_READY"))).not.toContain("typed-before-start");

    // The fork is the operator's own root: a row of its own with the mark,
    // the source unmarked and with no child under it.
    const row = page.locator(`nav[data-sidebar] [data-pane="${forked}"]`);
    await expect(row).toBeVisible({ timeout: 30_000 });
    await expect(row).not.toHaveAttribute("data-delegated", "true");
    await expect(row.locator(`[data-agent-fork="${forked}"]`)).toBeVisible();
    await expect(page.locator(`nav[data-sidebar] [data-pane="${forker}"] [data-agent-fork]`)).toHaveCount(0);
    await expect(page.locator(`[data-tree-chevron="${forker}"]`)).toHaveCount(0);
    const mark = page.locator(`[data-pane-view="${forked}"] [data-pane-fork]`);
    await expect(mark).toHaveAttribute("data-pane-fork", forker, { timeout: 30_000 });
    await expect(mark).toHaveAccessibleName("Forked from Agent two");
    await expect(page.locator(`[data-pane-view="${forker}"] [data-pane-fork]`)).toHaveCount(0);
    await screenshot(page, "agent-fork-tab");

    // The header mark opens the source.
    await mark.click();
    await expect.poll(() => coreFocus(daemon!), { timeout: 30_000 }).toBe(forker);
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});

test("Fork agent to the side splits the source's tab and moves the keyboard to the fork", async ({ page }) => {
  const herdr = await startFixture();
  let daemon: Daemon | null = null;
  try {
    const stack = await forkStack(page, herdr, "agent-fork-side");
    daemon = stack.daemon;
    const { forker, last } = stack;
    const { forked } = await fork(page, herdr, daemon, forker, "fork_agent_side", "typed-beside");
    expect(last.get("fork_pane")).toMatchObject({ pane_id: forker, placement: "side" });
    await expect.poll(() => coreFocus(daemon!), { timeout: 30_000 }).toBe(forked);
    // The fork shares the source's tab, to its right.
    await expect(page.locator(`[data-pane-view="${forked}"]`)).toBeVisible({ timeout: 30_000 });
    const source = await page.locator(`[data-pane-view="${forker}"]`).boundingBox();
    const beside = await page.locator(`[data-pane-view="${forked}"]`).boundingBox();
    expect(beside!.x).toBeGreaterThan(source!.x);
    await expect.poll(() => screenOf(page, forked), { timeout: 10_000 }).toContain("typed-beside");
    await expect(page.locator(`nav[data-sidebar] [data-pane="${forked}"] [data-agent-fork="${forked}"]`)).toBeVisible({ timeout: 30_000 });
    await expect(page.locator(`[data-pane-view="${forked}"] [data-pane-fork]`)).toHaveAccessibleName("Forked from Agent two");
    await screenshot(page, "agent-fork-side");
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});
