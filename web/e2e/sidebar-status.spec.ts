// Session-first sidebar and the agent tree on an isolated real daemon
// (PRD agent-hierarchy-screens B10 to B21). Ordinary questions remain the
// parent's responsibility; children unfold under their root only when the
// operator opens it, and the pane header's tree button opens the same tree.
import { expect, test } from "@playwright/test";
import { continueFixtureTranscript, declareParent, elsewhereTab, finishFixtureTurn, labelAgent, setFixtureLifecycle, startHerdr } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { countSent, screenshot, sidebarOverflow } from "./wire";

test.describe.configure({ timeout: 150_000 });
const LONG_TITLE = "행 높이 WIDE TITLE MUST CUT HERE";
const QUESTION = "PR 병합 전 검증을 다시 돌려도 될까요?";
const WAIT_SENTENCE = "하위 작업 위임 후 자식이 계보 투영과 테스트를 모두 끝낼 때까지 기다리는 중";

test("roots stay in one sidebar, open their children in place, and the header's tree button opens the tree", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const herdr = await startHerdr({ readiness: "registered" });
  let daemon: Daemon | null = null;
  try {
    const [parent, child] = herdr.panes;
    labelAgent(herdr, parent, { task: "Agent one", progress: WAIT_SENTENCE });
    await finishFixtureTurn(herdr, parent, elsewhereTab(herdr));
    const childSession = labelAgent(herdr, child, { task: LONG_TITLE, progress: "계보 투영 구현 중" });
    await setFixtureLifecycle(herdr, child, "working");
    declareParent(herdr, child, parent);
    daemon = await startHided(herdr, "sidebar-status");
    const last = new Map<string, Record<string, unknown>>();
    const sent = countSent(page, last);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    // The parent's row in its project's tree: once it is an unread finished root it also stands raised under Done.
    const parentRow = page.locator(`nav[data-sidebar] [data-checkout-agents-open] [data-pane="${parent}"]`);
    const childRow = page.locator(`nav[data-sidebar] [data-pane="${child}"]`);
    await expect(parentRow).toHaveAttribute("data-waiting", "children", { timeout: 20_000 });
    await expect(page.locator("[data-sidebar-mode]")).toHaveCount(0);
    // Folded, a root whose descendants only work wears no mark: its waiting ring says it (B10, B17).
    await expect(childRow).toHaveCount(0);
    await expect(parentRow.locator('[data-descendant-mark]')).toHaveCount(0);
    // A waiting ring's tree line leads with what it waits for, and at the narrowest sidebar only its sentence is cut (status-marks B29, needs-you-cleanup B17).
    const waitLine = parentRow.locator('[data-agent-line="waiting"]');
    await expect(waitLine).toHaveText(`Waiting on children · ${WAIT_SENTENCE}`, { timeout: 20_000 });
    expect(await sidebarOverflow(page, "calc(var(--size-sidebar-min) + var(--size-rail))")).toEqual([]);
    expect(await waitLine.locator('[data-line-part="text"]').evaluate((node) => node.scrollWidth > node.clientWidth)).toBe(true);
    await sidebarOverflow(page, "");
    continueFixtureTranscript(herdr, childSession, { task: LONG_TITLE, reply: QUESTION, question: true });
    await setFixtureLifecycle(herdr, child, "idle");
    // An ordinary question waits on the root, so the root stops waiting and keeps its own state, never raised, and a quiet child adds no mark.
    await expect(parentRow).toHaveAttribute("data-waiting", "false", { timeout: 20_000 });
    await expect(page.locator(`[data-raised-group="done"] [data-pane="${parent}"]`)).toBeVisible();
    await expect(parentRow.locator("[data-descendant-mark]")).toHaveCount(0);
    await expect(page.locator('[data-raised-group="needs_you"]')).toHaveCount(0);

    // Right opens the root in place and Left folds it (B15).
    const open = parentRow.locator(`[data-agent-open="${parent}"]`);
    await open.focus();
    await page.keyboard.press("ArrowRight");
    await expect.poll(() => sent.get("agent_tree_toggle") ?? 0).toBe(1);
    await expect(childRow).toHaveAttribute("data-depth", "1", { timeout: 20_000 });
    await expect(childRow).toContainText(LONG_TITLE);
    await expect(open).toHaveAttribute("aria-expanded", "true");
    await screenshot(page, "sessions-sidebar-child-question");
    for (const width of ["calc(240px + var(--size-rail))", "calc(var(--size-sidebar-min) + var(--size-rail))"]) expect(await sidebarOverflow(page, width)).toEqual([]);
    await sidebarOverflow(page, "");

    // A child row opens the child's pane.
    const before = sent.get("focus_pane") ?? 0;
    await childRow.locator(`[data-agent-open="${child}"]`).click();
    await expect.poll(() => sent.get("focus_pane") ?? 0).toBe(before + 1);
    expect(last.get("focus_pane")?.pane_id).toBe(child);
    await expect(page.locator(`[data-pane-view="${child}"]`)).toBeVisible();
    // The child's header walks back to its parent (B20).
    const back = page.locator(`[data-pane-return="${parent}"]`);
    await expect(back).toBeVisible();
    await back.click();
    await expect(page.locator(`[data-pane-view="${parent}"]`)).toBeVisible();

    // The parent's tree button opens the tree popover; Escape returns to it (B18, B19, B21).
    const treeButton = page.locator(`[data-pane-view="${parent}"] [data-pane-tree="${parent}"]`);
    const tree = page.locator(`[data-agent-tree="${parent}"]`);
    await expect(page.locator(`[data-pane-view="${parent}"] > header [data-pane-tree]`)).toHaveCount(0);
    await expect(page.locator(`[data-pane-view="${parent}"] [data-pane-header-band="children"] [data-pane-tree]`)).toBeVisible();
    await expect(treeButton).toHaveAccessibleName(/Child agents 1.*Child requests: 1/);
    await expect(treeButton).not.toHaveAccessibleName(/needing you/);
    await treeButton.focus();
    await page.keyboard.press("Space");
    await expect(tree.locator(`[data-agent-child="${child}"]`)).toContainText(LONG_TITLE);
    await page.keyboard.press("Escape");
    await expect(tree).toHaveCount(0);
    await expect(treeButton).toBeFocused();
    await treeButton.click();
    await screenshot(page, "sessions-pane-child-popover");
    // The tree's task button reveals the parent's row in the sidebar's 태스크 list in one event, its record in the lane's 보기 slot when it has a session, and no Overview opens (PRD sidebar-task-lens B40).
    const reveals = sent.get("task_reveal") ?? 0;
    await tree.locator("[data-agent-tree-task]").click();
    await expect.poll(() => sent.get("task_reveal") ?? 0).toBe(reveals + 1);
    expect(last.get("task_reveal")).toMatchObject({ pane_id: parent });
    await expect(page.getByRole("dialog", { name: "Overview", exact: true })).toHaveCount(0);

    // B16: when the child goes, the tree and its button go with it. The reveal left the sidebar on 태스크; the row is in 프로젝트.
    await page.locator('[data-sidebar-lens-option="projects"]').click();
    await page.locator(`nav[data-sidebar] [data-pane="${parent}"] [data-agent-open="${parent}"]`).first().click();
    // The record the reveal opened stays up over the panes' right edge, where
    // the children button sits, until the operator closes it.
    await page.locator("[data-detail-close]").click();
    await treeButton.click();
    herdr.run(["pane", "close", child]);
    await expect(tree).toHaveCount(0, { timeout: 20_000 });
    await expect(treeButton).toHaveCount(0);
    await expect(parentRow.locator(`[data-tree-chevron="${parent}"]`)).toHaveCount(0);
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});
