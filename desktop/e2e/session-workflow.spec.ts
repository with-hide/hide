// The 태스크 list and the quiet header on one exact native candidate. Herdr, HOME,
// daemon and profile belong to this fixture; input never reaches the operator.
import { expect, type ElectronApplication } from "@playwright/test";
import { execFileSync } from "node:child_process";
import path from "node:path";
import { continueFixtureTranscript, declareParent, labelAgent, setFixtureLifecycle, startHerdr } from "../../web/e2e/herdr-fixture";
import { enterWorkspace, showTasks } from "../../web/e2e/wire";
import { fitWindow, isolate, launchShell, test } from "./fixture";
import { captureNativeWindow } from "./native-window";

test("the 태스크 list and raised-child pane bands preserve native terminal geometry in both themes", async () => {
  const herdr = await startHerdr();
  const run = isolate(herdr, "session-workflow");
  let app: ElectronApplication | undefined;
  const failures: unknown[] = [];
  try {
    const [parent, child] = herdr.panes;
    const parentSession = labelAgent(herdr, parent, { task: "한국어 입력과 세션 복귀 경계를 검토하는 긴 작업 제목", progress: "자식 검토 결과를 기다리는 중" });
    labelAgent(herdr, child, { task: "자식의 긴 한국어 검증 요청과 회귀 결과 확인", progress: "검증 명령 실행 권한이 필요합니다" });
    declareParent(herdr, child, parent);
    await setFixtureLifecycle(herdr, child, "blocked");
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
    await expect(treeButton).toHaveText("1");
    await expect(pane.locator(`[data-pane-child-name]`)).toHaveCount(0);
    await expect(pane.locator(`[data-pane-own-progress="${parent}"]`)).toHaveCount(0);
    const raisedRow = pane.locator('[data-pane-header-band="raised"]');
    const raisedVerb = raisedRow.locator('[data-verb="approval"]');
    const raisedWho = raisedRow.locator(`[data-pane-band-who="${child}"]`);
    await expect(raisedVerb).toHaveText("Approval");
    await expect(raisedWho).toContainText("자식의 긴 한국어 검증 요청");
    await expect(raisedWho).toContainText("(child)");
    expect(await raisedVerb.evaluate((verb, childPaneId) => {
      const who = verb.parentElement?.querySelector(`[data-pane-band-who="${childPaneId}"]`);
      return !!who && !!(verb.compareDocumentPosition(who) & Node.DOCUMENT_POSITION_FOLLOWING);
    }, child)).toBe(true);
    // The raised ask leads with the parent's request, its own first line while the label gave no summary (PRD pane-last-request B1, B3).
    await expect(raisedRow.locator(`[data-pane-turn-request="${parent}"]`)).toContainText("한국어 입력과 세션 복귀 경계를 검토하는 긴 작업 제목 진행해줘");
    const terminal = await host.boundingBox();
    const facts = { head: execFileSync("git", ["rev-parse", "HEAD"], { cwd: path.resolve(__dirname, "../.."), encoding: "utf8" }).trim(), parent, child, home: run.env.HOME, socket: herdr.socket, state: run.env.HIDE_STATE_DIR, provider: "synthetic agent transcript and lifecycle" };
    for (const theme of ["light", "dark"] as const) {
      await page.getByRole("button", { name: /^Settings \(/ }).click();
      await page.locator('[data-settings-tab="general"]').click();
      await page.locator(`[data-theme-option="${theme}"]`).click();
      await expect(page.locator("html")).toHaveClass(new RegExp(`\\b${theme}\\b`));
      await page.keyboard.press("Escape");
      await captureNativeWindow(app, `session-header-${theme}`, { ...facts, terminal });
      await treeButton.click();
      await expect(page.locator(`[data-agent-child="${child}"]`)).toBeVisible();
      await captureNativeWindow(app, `session-children-${theme}`, facts);
      await page.keyboard.press("Escape");
      await expect(treeButton).toBeFocused();
      // The raised child asks through its root's row, folded under it, with its question as the row's second line (agent-hierarchy-screens B1, B2; sidebar-task-lens B7, B8, B10).
      const tasks = await showTasks(page);
      await expect(tasks.locator(`[data-task-group="needs_you"] [data-task-row="${parent}"] [data-task-line="needs_you"]`)).toContainText("검증 명령 실행 권한이 필요합니다");
      await expect(tasks.locator(`[data-task-row="${child}"], [data-task-child="${child}"]`)).toHaveCount(0);
      // A folded parent counts no children (status-marks B35).
      await expect(tasks.locator(`[data-task-row="${parent}"] [data-task-children]`)).toHaveCount(0);
      await captureNativeWindow(app, `tasks-${theme}`, facts);
      await page.locator('[data-sidebar-lens-option="projects"]').click();
      await expect(page.locator('[data-sidebar-lens="projects"]')).toBeVisible();
    }
    await setFixtureLifecycle(herdr, child, "working");
    await expect(pane.locator('[data-pane-header-band="raised"]')).toHaveCount(0);
    continueFixtureTranscript(herdr, parentSession, { task: "한국어 입력과 세션 복귀 경계를 검토하는 긴 작업 제목", progress: "자식 검토 결과를 기다리는 중", request: "자식 검토 결과 정리와 세션 복귀 확인" });
    await setFixtureLifecycle(herdr, parent, "working");
    await expect(pane.locator(`[data-pane-own-progress="${parent}"]`)).toHaveText("자식 검토 결과를 기다리는 중", { timeout: 25_000 });
    // The working turn leads with the label's summary of the new request, and its card opens under it (B1, B2, B6).
    const request = pane.locator(`[data-pane-turn-request="${parent}"]`);
    await expect(request).toContainText("자식 검토 결과 정리와 세션 복귀 확인");
    await request.hover();
    await expect(page.locator(`[data-pane-turn-card="${parent}"] [data-turn-card-original]`)).toContainText("이어서 진행해줘");
    await captureNativeWindow(app, "session-turn-card", { ...facts, terminal });
    await page.mouse.move(0, 0);
    expect(await host.boundingBox()).toEqual(terminal);
    await expect(page.locator(`nav[data-sidebar] [data-pane="${child}"]`)).toHaveCount(0);
    await captureNativeWindow(app, "session-working-restored", { ...facts, terminal });
  } catch (error) { failures.push(error); }
  finally {
    try { await app?.close(); } catch (error) { failures.push(error); }
    try { run.cleanup(); } catch (error) { failures.push(error); }
    try { herdr.stop(); } catch (error) { failures.push(error); }
  }
  if (failures.length) throw new AggregateError(failures, "native task list fixture or teardown failed");
});
