// A tab the operator named through Rename outlives its agents
// (docs/UI_BEHAVIOR.md, Agent areas): closing its last agent pane leaves a
// shell in the tab, and closing that shell closes the tab.
import { expect, test as base, type Page } from "@playwright/test";
// Throwaway probe (with-hide/hide-private#476, #187, #140, #534, #518): this spec's flaky test only, registered 20 times.
// eslint-disable-next-line @typescript-eslint/no-explicit-any
const test = Object.assign((title: string, ...rest: any[]) => {
  if (title !== "reopening an unnamed tab leaves it unnamed") return;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  for (let run = 1; run <= 20; run++) (base as any)(`${title} #${run}`, ...rest);
}, { describe: base.describe, use: base.use, step: base.step, setTimeout: base.setTimeout });
import path from "node:path";
import { startFixture } from "./engine-fixture";
import { newTab, tabPanes, tabsOf } from "./daemon-actions";
import { connectControl, type HidedControl, type Rest } from "./hided-control.mjs";
import { toPage } from "../../desktop/src/main/wirePath";
import { chord } from "./chords";
import { startHided, type Daemon } from "./hided-fixture";
import { enterWorkspace, screenshot } from "./wire";

test.describe.configure({ timeout: 120_000 });

/** The tab the core shows, as the snapshot publishes it: its panes with their folders, and the name the sidebar gives it. */
type ShownTab = { id: string | null; label: string | null; sidebar?: { manual_name: string | null; number: number; named_by_operator: boolean }; panes: { id: string; cwd: string }[] };

function shown(rest: Rest): ShownTab | undefined {
  return (rest.tab ?? undefined) as ShownTab | undefined;
}

test("closing a named tab's last agent keeps the tab, its name and its sidebar line until its shell closes", async ({ page }) => {
  const fixture = await startFixture();
  let daemon: Daemon | null = null;
  let control: HidedControl | null = null;
  try {
    daemon = await startHided(fixture, "named-tab-stays");
    const [first, second] = fixture.panes;
    const watch = (control = await connectControl(daemon));
    const panesOfTab = () => watch.until((rest) => tabPanes(rest, fixture.tab), "the panes of the tab");
    const tabExists = (rest: Rest) => tabsOf(rest, fixture.workspace)?.includes(fixture.tab) === true;
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    await page.locator(`[data-agent-tab-bar] [data-tab="${fixture.tab}"]`).click({ button: "right" });
    await page.locator('[data-menu-item="rename_tab"]').click();
    const input = page.getByRole("textbox", { name: "Tab name", exact: true });
    await input.fill("Planning");
    await input.press("Enter");
    await expect(input).toHaveCount(0);
    const line = page.locator(`[data-sidebar-tab-line="${fixture.tab}"]`);
    await expect(line).toContainText("Planning");

    // One agent of two leaves as any pane does.
    await page.locator(`[data-pane-view="${second}"]`).getByRole("button", { name: /^Close/ }).click();
    await expect.poll(panesOfTab).toEqual([first]);

    // The last one leaves a shell at the checkout in the same tab.
    await page.locator(`[data-pane-view="${first}"]`).getByRole("button", { name: /^Close/ }).click();
    const [shell] = await watch.until((rest) => {
      const now = tabPanes(rest, fixture.tab);
      return now.length === 1 && now[0] !== first ? now : undefined;
    }, "a shell in the tab where the last agent was");
    const kept = await watch.until((rest) => (shown(rest)?.id === fixture.tab ? shown(rest) : undefined), "the named tab shown");
    expect(kept.panes.find((pane) => pane.id === shell)?.cwd).toBe(toPage(path.join(fixture.root, "fixture")));
    expect(kept.sidebar?.manual_name).toBe("Planning");
    await expect(page.locator(`[data-agent-tab-bar] [data-tab="${fixture.tab}"]`)).toContainText("Planning");
    await expect(page.locator(`[data-pane-view="${shell!}"]`)).toBeVisible();
    await expect(page.locator(`[data-pane-view="${shell!}"]`)).toHaveAttribute("data-focused", "true");
    await expect(line).toContainText("Planning");
    // With no agent left, its line says its name alone, never a count of none.
    await expect(line.locator("[data-sidebar-tab]")).toHaveAttribute("aria-label", "Planning");
    await screenshot(page, "named-tab-kept-with-shell");

    // Choosing the line opens the tab; closing the shell closes the tab and its name.
    await line.locator("[data-sidebar-tab]").click();
    await expect(page.locator("[data-canvas]").first()).toHaveAttribute("data-canvas", fixture.tab);
    await page.locator(`[data-pane-view="${shell!}"]`).getByRole("button", { name: /^Close/ }).click();
    await watch.until((rest) => !tabExists(rest) || undefined, "the tab closed with its shell");
    await expect(line).toHaveCount(0);
    await expect(page.locator(`[data-tab="${fixture.tab}"]`)).toHaveCount(0);
  } finally {
    control?.close();
    daemon?.stop();
    fixture.stop();
  }
});

/**
 * Whether the core has said there is a close to reopen. Reopen closed tab
 * does nothing until then, and nothing on the page shows it, so the spec
 * reads the snapshot frames the page receives.
 */
function reopenOffered(page: Page): () => boolean {
  let offered = false;
  const find = (value: unknown): boolean =>
    typeof value === "object" && value !== null &&
    Object.entries(value).some(([key, inner]) => (key === "recent_closed" && (inner as { can_reopen?: unknown }).can_reopen === true) || find(inner));
  page.on("websocket", (ws) => ws.on("framereceived", (frame) => {
    try {
      offered ||= find(JSON.parse(String(frame.payload)));
    } catch {
      /* binary terminal frames are not snapshots */
    }
  }));
  return () => offered;
}

// An engine keeps any label a reopen sends as the tab's name, so a reopen
// sends only the operator's: a tab never named comes back unnamed.
test("reopening an unnamed tab leaves it unnamed", async ({ page }) => {
  const fixture = await startFixture();
  let daemon: Daemon | null = null;
  let control: HidedControl | null = null;
  try {
    daemon = await startHided(fixture, "named-tab-reopen");
    const watch = (control = await connectControl(daemon));
    // A second tab keeps the workspace, which its last tab's close would end.
    const other = (await newTab(watch, fixture, "other")).tab;
    const offered = reopenOffered(page);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");

    // The number its sidebar line gives it now, which a reopen has to give back.
    const number = /Tab (\d+)/.exec((await page.locator(`[data-sidebar-tab-line="${fixture.tab}"]`).textContent()) ?? "")?.[1];
    expect(number, "the unnamed tab's sidebar line says its number").toBeDefined();
    const closing = page.locator(`[data-agent-tab-bar] [data-tab="${fixture.tab}"]`);
    await closing.hover();
    await closing.getByRole("button", { name: /^Close tab/ }).click();
    await watch.until((rest) => tabsOf(rest, fixture.workspace)?.includes(fixture.tab) === false || undefined, "the tab closed");
    await expect.poll(offered).toBe(true);
    await page.locator(`[data-agent-tab-bar] [data-tab="${other}"]`).click();
    await page.keyboard.press(chord("reopen_closed_tab"));
    const reopenedId = await watch.until((rest) => tabsOf(rest, fixture.workspace)?.find((id) => id !== other && tabPanes(rest, id).length === 2), "the reopened tab with its two panes");
    // The reopened tab is shown, and the name it carries is the operator's, which it never had.
    const reopened = await watch.until((rest) => (shown(rest)?.id === reopenedId ? shown(rest) : undefined), "the reopened tab shown");
    expect(reopened.sidebar?.named_by_operator, "the reopened tab carries no name Hide made up").toBe(false);
    expect(reopened.sidebar?.manual_name).toBeNull();

    // The sidebar names it as any unnamed tab, so its last agent's close
    // takes it (closing restored agents waits on a status check here). A
    // reopen makes the tab after the others and then moves it back to its old
    // place, so the first snapshot that shows it carries a number it is about
    // to leave; the number it settles on is the one it had when it closed.
    await expect(page.locator(`[data-sidebar-tab-line="${reopenedId}"]`)).toContainText(`Tab ${number}`);
  } finally {
    control?.close();
    daemon?.stop();
    fixture.stop();
  }
});
