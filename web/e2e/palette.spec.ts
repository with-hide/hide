// The ⌘K palette as a navigation palette (PRD cmdk-navigation) on an
// isolated engine and hided: the sidebar's Search icon and ⌘K open the
// same overlay; on a screen with nothing in front it is the input alone, and a
// query lists its results under one group per kind with the highlighted row's
// detail beside them; an agent row is its mark, its title and its state line
// under it; ↵ follows the selection across groups; Enter opens the agent;
// Escape closes and hands focus back; a query with no match says so and, with
// no GitHub project here, offers no GitHub search. Captured in Dark and Light.

import { expect, test, type Page } from "@playwright/test";
import { startFixture, type AgentFixture } from "./engine-fixture";
import { labelAgentPane, setAgentState } from "./fixture-agents";
import type { FixtureLabel } from "./herdr-fixture";
import { asFirstRun, startHided, type Daemon } from "./hided-fixture";
import { countSent, screenshot } from "./wire";
import { chord } from "./chords";

test.describe.configure({ timeout: 120_000 });

/** The group headings the open palette draws, top to bottom. */
function headings(page: Page) {
  return page.locator('[data-palette="Search"] [cmdk-group-heading]');
}


/** Opens the restarted daemon's page through a blank page, so the new token mounts a new connection. */
async function openAfterRestart(page: Page, daemon: Daemon): Promise<void> {
  await page.goto("about:blank");
  await page.goto(`${daemon.origin}/#token=${daemon.token}`);
}

/** Labels the agent in `pane` on the turn it is in: its session is taken while it works, and it works again for the line to show. */
async function labelWorking(fixture: AgentFixture, daemon: Daemon, pane: string, label: FixtureLabel): Promise<void> {
  await setAgentState(daemon, pane, "working");
  await labelAgentPane(fixture, daemon, pane, label);
  await setAgentState(daemon, pane, "working");
}

test("⌘K lists results by kind with a detail beside them, and the sidebar Search icon opens the same palette", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const fixture = await startFixture();
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(fixture, "palette");
    const [one, two] = fixture.panes;
    daemon = await asFirstRun(daemon);
    const last = new Map<string, Record<string, unknown>>();
    const sent = countSent(page, last);
    await openAfterRestart(page, daemon);
    // One sidebar exposes the project and its root agents.
    await expect(page.locator('[data-sidebar="projects"]')).toBeVisible();
    await labelWorking(fixture, daemon, one, { task: "Agent one", progress: "팔레트 그룹 검증 중" });
    // A tree row is one line (agent-hierarchy-screens B14): the label's title says it landed.
    await expect(page.locator(`nav[data-sidebar] [data-pane="${one}"] [data-agent-title]`)).toHaveText("Agent one", { timeout: 20_000 });

    // The sidebar's Search row opens the palette with the query focused. The
    // page shows Main, which has nothing to relate to, so the palette holds
    // Recent alone: the checkout the daemon had in front (PRD cmdk-recent B5).
    const field = page.locator("[data-sidebar-search]");
    await expect(field).toHaveAccessibleName(/^Search\b/);
    await field.click();
    const input = page.locator('[data-palette="Search"] [data-palette-input]');
    await expect(input).toBeFocused();
    await expect(input).toHaveAttribute("placeholder", "Enter a name or #number");
    await expect(page.locator("[data-palette-esc]")).toHaveText("Esc");
    await expect(page.locator("[data-cmdk]")).toHaveAttribute("data-cmdk", "open");
    await expect(headings(page)).toHaveText(["Recent"]);
    await expect(page.locator("[data-palette-row]")).toHaveCount(1);
    await screenshot(page, "palette-dark-recent");

    // Typing opens the results: agents first for an agent's name, and the one command ⌘K keeps is not among them.
    await page.keyboard.type("Agent");
    await expect(page.locator("[data-cmdk]")).toHaveAttribute("data-cmdk", "open");
    await expect(headings(page).first()).toHaveText("Agents");
    const agents = page.locator('[data-palette-group="agents"]');
    await expect(agents.locator("[data-palette-row]")).toHaveCount(2);

    // An agent row: its status mark, the title, the state line under it, and no provider logo (agent-mark-quiet B9).
    const rowOne = page.locator(`[data-palette-row="agent:${one}"][title^="Claude Code"]`);
    await expect(rowOne.locator("[data-agent-mark]")).toHaveCount(0);
    await expect(rowOne).toContainText("Agent one");
    await expect(rowOne.locator("[data-palette-detail]")).toContainText("팔레트 그룹 검증 중");
    const rowTwo = page.locator(`[data-palette-row="agent:${two}"]`);
    await expect(rowTwo).toContainText("Agent two");
    await expect(rowTwo.locator("[data-palette-detail]")).not.toBeEmpty();
    const titleBox = await rowOne.getByText("Agent one").boundingBox();
    const detailBox = await rowOne.locator("[data-palette-detail]").boundingBox();
    expect(titleBox && detailBox && detailBox.y >= titleBox.y + titleBox.height - 1).toBe(true);

    // ↵ marks the selected row, the detail beside the list says what it is,
    // and the arrows move both across a group boundary (B15, B25).
    const rows = page.locator('[data-palette="Search"] [data-palette-row]');
    const firstRow = rows.first();
    await expect(firstRow).toHaveAttribute("aria-selected", "true");
    await expect(firstRow.locator("[data-palette-enter]")).toBeVisible();
    await expect(page.locator('[data-palette-detail-pane="agent"]')).toBeVisible();
    await expect(page.locator('[data-palette-detail-pane="agent"]')).toContainText(/Agent (one|two)/);
    await screenshot(page, "palette-dark-default");
    await page.keyboard.press("ArrowDown");
    const second = rows.nth(1);
    await expect(second).toHaveAttribute("aria-selected", "true");
    await expect(second.locator("[data-palette-enter]")).toBeVisible();
    await expect(firstRow.locator("[data-palette-enter]")).toBeHidden();
    await expect(page.locator("[data-palette-detail-pane]")).toContainText((await second.locator("span.truncate").first().innerText()).trim());

    // Escape closes and hands focus back to the field that opened it.
    await page.keyboard.press("Escape");
    await expect(page.locator("[data-palette-input]")).toHaveCount(0);
    await expect(field).toBeFocused();

    // ⌘K is the same overlay; Enter opens the agent the query names.
    await page.keyboard.press(chord("search"));
    await expect(input).toBeFocused();
    await page.keyboard.type("Agent two");
    await expect(headings(page).first()).toHaveText("Agents");
    await expect(rows.first()).toHaveAttribute("data-palette-row", `agent:${two}`);
    const focuses = sent.get("focus_pane") ?? 0;
    await page.keyboard.press("Enter");
    await expect(page.locator("[data-palette-input]")).toHaveCount(0);
    await expect.poll(() => sent.get("focus_pane") ?? 0).toBeGreaterThan(focuses);
    expect(last.get("focus_pane")?.pane_id).toBe(two);

    // No match is one line, with no group; this Mac has no GitHub project, so
    // there is no GitHub search to offer either, and nothing is sent to GitHub.
    // Opening the agent moved the page to its Workspace, and the palette can
    // open after that screen does; typing before it holds focus loses keys.
    await page.keyboard.press(chord("search"));
    await expect(input).toBeFocused();
    await page.keyboard.type("zzzz-no-such-agent");
    await expect(page.locator('[data-palette-state="no-match"]')).toHaveText("No matching items");
    await expect(headings(page)).toHaveCount(0);
    await expect(page.locator('[data-palette-row="github-search"]')).toHaveCount(0);
    await screenshot(page, "palette-dark-no-match");
    await page.keyboard.press("Escape");
    expect(sent.get("github_search") ?? 0).toBe(0);

    // Light: the same palette on the Light tokens. Opening the agent put its
    // Workspace on screen, so an empty ⌘K lists what is connected to it.
    await page.keyboard.press(chord("settings"));
    await page.locator('[data-settings-tab="general"]').click();
    await page.locator('[data-theme-option="light"]').click();
    await expect(page.locator("html")).toHaveClass(/(^|\s)light(\s|$)/);
    await page.keyboard.press("Escape");
    await field.click();
    await expect(input).toBeFocused();
    await expect(headings(page)).toHaveText(["Related"]);
    await screenshot(page, "palette-light-default");
    // A query the agents match best brings their group first.
    await page.keyboard.type("Agent");
    await expect(headings(page).first()).toHaveText("Agents");
    await expect(rows.first()).toHaveAttribute("aria-selected", "true");
    await screenshot(page, "palette-light-query");
    await page.keyboard.press("Escape");
  } finally {
    daemon?.stop();
    fixture.stop();
  }
});
