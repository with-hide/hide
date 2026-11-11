// The 태스크 list and the quiet header on one exact native candidate. The engine, HOME,
// daemon and profile belong to this fixture; input never reaches the operator.
import { expect, type ElectronApplication } from "@playwright/test";
import { execFileSync } from "node:child_process";
import path from "node:path";
import { setAgentState } from "../../web/e2e/fixture-agents";
import { agentIn, showTabOf, withControl } from "../../web/e2e/daemon-actions";
import { dumpOnFailure } from "../../web/e2e/failure-dump";
import { continueFixtureTranscript } from "../../web/e2e/pane-fixture";
import { askPermission, startWork } from "../../web/e2e/session-work-fixture";
import { enterWorkspace, showTasks } from "../../web/e2e/wire";
import { animationsFinished, compositorPresents } from "../../web/e2e/wait";
import { fitWindow, launchShell, startStack, test } from "./fixture";
import { captureNativeWindow } from "./native-window";
import { finishNative } from "./native-teardown";

test("the 태스크 list and raised-child pane bands preserve native terminal geometry in both themes", async () => {
  test.setTimeout(150_000); // wait-ok: the 60 s blocked-child grace, then the existing 90 s native workflow budget.
  // The child is the parent's delegated child, spawned by the parent agent itself (family).
  const stack = await startStack("session-workflow", { family: true });
  const { fixture, run, daemon } = stack;
  let app: ElectronApplication | undefined;
  const failures: unknown[] = [];
  try {
    // The parent runs a session labelled from its start, and delegates a child running a labelled session of its own.
    const childTask = "자식의 긴 한국어 검증 요청과 회귀 결과 확인", childAsk = "검증 명령 실행 권한이 필요합니다";
    const started = await startWork(fixture, daemon,
      { task: "한국어 입력과 세션 복귀 경계를 검토하는 긴 작업 제목", progress: "자식 검토 결과를 기다리는 중" },
      { task: childTask, progress: "자식이 검증 명령을 준비하는 중" });
    const parent = started.pane, child = started.child!.pane, parentSession = started.session;
    // The child's request is its next turn, asked at a permission prompt (see `askPermission`).
    await askPermission(fixture, daemon, child, started.child!.session, { task: childTask, progress: childAsk });
    // The parent's row says what its blocked child asks in the child's label line, which the core
    // lays on the parent's raised ask once the label proves the child's session. The page draws
    // only what the snapshot carries, so wait for the snapshot, not for the page's own timeout:
    // a row drawn before the label reached the ask reads the verb's fallback, "Stopped at a
    // permission request" (with-hide/hide-private#515). PR #2443 also gives the parent
    // 60 s to handle the block before it raises: retain 30 s for the labelled snapshot after that.
    await dumpOnFailure("the parent's and the child's agent rows",
      () => withControl(daemon, (control) => control.until((rest) => ({ parent: agentIn(rest, parent), child: agentIn(rest, child) }), "the two agent rows", 2_000)),
      () => withControl(daemon, (control) => control.until(
        (rest) => agentIn(rest, parent)?.raised?.find((ask) => ask.raised_pane_id === child)?.what === childAsk || undefined,
        `${parent}'s raised ask from ${child} carrying the child's label line`, 90_000))); // wait-ok: the child's labelled raised ask after the blocked-child grace.
    // Drawing the child's prompt types into its pane. Inspect the parent's
    // header with the parent's tab selected, as the scenario requires.
    await withControl(daemon, (control) => showTabOf(control, parent));
    const opened = await launchShell(run.env);
    app = opened.app;
    const page = opened.page;
    await fitWindow(app, { width: 1440, height: 1000 });
    await enterWorkspace(page, "fixture");
    const pane = page.locator(`[data-pane-view="${parent}"]`);
    const host = pane.locator(`[data-terminal-host="${parent}"]`);
    await expect(pane.locator('[data-pane-header-band="raised"]')).toBeVisible({ timeout: 30_000 });
    const treeButton = pane.locator(`[data-pane-tree="${parent}"]`);
    await expect(treeButton).toBeVisible();
    // One child: its status mark stands in for the count, with no `+N`.
    await expect(treeButton.locator("[data-pane-tree-mark]")).toHaveCount(1);
    await expect(treeButton).toHaveText("");
    await expect(pane.locator(`[data-pane-child-name]`)).toHaveCount(0);
    await expect(pane.locator(`[data-pane-own-progress="${parent}"]`)).toHaveCount(0);
    const raisedRow = pane.locator('[data-pane-header-band="raised"]');
    const raisedWho = raisedRow.locator(`[data-pane-band-who="${child}"]`);
    // A raised ask names no verb: the warning rail says whose turn it is, and the child's name leads.
    await expect(raisedRow.locator("[data-verb]")).toHaveCount(0);
    await expect(raisedWho).toContainText("자식의 긴 한국어 검증 요청");
    await expect(raisedWho).toContainText("(child)");
    // The raised ask leads with the parent's request, its own first line while the label gave no summary (PRD pane-last-request B1, B3).
    await expect(raisedRow.locator("[data-pane-turn-request-text]")).toContainText("한국어 입력과 세션 복귀 경계를 검토하는 긴 작업 제목 진행해줘");
    const terminal = await host.boundingBox();
    const facts = { head: execFileSync("git", ["rev-parse", "HEAD"], { cwd: path.resolve(__dirname, "../.."), encoding: "utf8" }).trim(), parent, child, home: run.env.HOME, state: run.env.HIDE_STATE_DIR, provider: "synthetic agent transcript and lifecycle" };
    for (const theme of ["light", "dark"] as const) {
      await page.getByRole("button", { name: /^Settings \(/ }).click();
      await page.locator('[data-settings-tab="general"]').click();
      await page.locator(`[data-theme-option="${theme}"]`).click();
      await expect(page.locator("html")).toHaveClass(new RegExp(`\\b${theme}\\b`));
      await page.keyboard.press("Escape");
      await expect(page.locator('[data-settings="true"]')).toHaveCount(0);
      await animationsFinished(page); await compositorPresents(page);
      await captureNativeWindow(app, `session-header-${theme}`, { ...facts, terminal });
      await treeButton.click();
      await expect(page.locator(`[data-agent-child="${child}"]`)).toBeVisible();
      await captureNativeWindow(app, `session-children-${theme}`, facts);
      await page.keyboard.press("Escape");
      await expect(treeButton).toBeFocused();
      // The raised child asks through its root's row, folded under it, with its question as the row's second line (agent-hierarchy-screens B1, B2; sidebar-task-lens B7, B8, B10).
      const tasks = await showTasks(page);
      await expect(tasks.locator(`[data-task-group="needs_you"] [data-task-row="${parent}"] [data-task-line="needs_you"]`)).toContainText(childAsk);
      await expect(tasks.locator(`[data-task-row="${child}"], [data-task-child="${child}"]`)).toHaveCount(0);
      // A folded parent counts no children (status-marks B35).
      await expect(tasks.locator(`[data-task-row="${parent}"] [data-task-children]`)).toHaveCount(0);
      await captureNativeWindow(app, `tasks-${theme}`, facts);
      await page.locator('[data-sidebar-lens-option="projects"]').click();
      await expect(page.locator('[data-sidebar-lens="projects"]')).toBeVisible();
    }
    await setAgentState(daemon, child, "working");
    await expect(pane.locator('[data-pane-header-band="raised"]')).toHaveCount(0);
    continueFixtureTranscript(fixture, parentSession, { task: "한국어 입력과 세션 복귀 경계를 검토하는 긴 작업 제목", progress: "자식 검토 결과를 기다리는 중", request: "자식 검토 결과 정리와 세션 복귀 확인" });
    await setAgentState(daemon, parent, "working");
    await expect(pane.locator(`[data-pane-own-progress="${parent}"]`)).toHaveText("자식 검토 결과를 기다리는 중", { timeout: 25_000 });
    // The working turn leads with the label's summary of the new request, and the whole `<request> → <answer>` is its card's
    // trigger: the real pointer pressing the answer's words, after the arrow, opens the card under the row (B1, B2, B6).
    const request = pane.locator(`[data-pane-turn-request="${parent}"]`);
    await expect(pane.locator(`[data-pane-turn="${parent}"] [data-pane-turn-request-text]`)).toContainText("자식 검토 결과 정리와 세션 복귀 확인");
    const answer = await pane.locator(`[data-pane-own-progress="${parent}"]`).boundingBox();
    if (!answer) throw new Error("the turn's answer has no box to press");
    await page.mouse.move(answer.x + answer.width / 4, answer.y + answer.height / 2);
    await expect(page.locator(`[data-pane-turn-card="${parent}"]`), "the pointer on the turn opens nothing").toHaveCount(0);
    await page.mouse.down();
    await page.mouse.up();
    await expect(request).toHaveAttribute("aria-expanded", "true");
    await expect(page.locator(`[data-pane-turn-card="${parent}"] [data-turn-card-original]`)).toContainText("이어서 진행해줘");
    await captureNativeWindow(app, "session-turn-card", { ...facts, terminal });
    await request.click();
    await expect(page.locator(`[data-pane-turn-card="${parent}"]`)).toHaveCount(0);
    await page.mouse.move(0, 0);
    expect(await host.boundingBox()).toEqual(terminal);
    await expect(page.locator(`nav[data-sidebar] [data-pane="${child}"]`)).toHaveCount(0);
    await captureNativeWindow(app, "session-working-restored", { ...facts, terminal });
  } catch (error) { failures.push(error); }
  await finishNative(failures, [["close the app", () => app?.close()], ["stop the stack", () => stack.stop()]]);
});
