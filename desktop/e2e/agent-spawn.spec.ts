// Actual spawn, lineage publication and the native delegation tree. Only the
// provider is synthetic; its session report comes from its real pane process.
import { expect, type ElectronApplication } from "@playwright/test";
import { execFileSync } from "node:child_process";
import path from "node:path";
import { agentPane } from "../../web/e2e/agent-pane";
import { openProject, openWorktree, showTabOf, withControl } from "../../web/e2e/daemon-actions";
import { enterWorkspace } from "../../web/e2e/wire";
import { launchShell, nodeOf, screenshot, startStack, test } from "./fixture";
import { captureNativeWindow } from "./native-window";
import { finishNative } from "./native-teardown";
import { expectCoreFocus, hideAnswer, makeSpawnRepository, type SpawnedAgent } from "./agent-spawn-fixture";

test("a spawned child appears in the native delegation tree", async () => {
  const stack = await startStack("agent-spawn", { agents: false });
  const { fixture, run, daemon } = stack;
  let app: ElectronApplication | undefined;
  const failures: unknown[] = [];
  try {
    const { repo, existingCheckout } = makeSpawnRepository(fixture.root, run.env);
    const project = await withControl(daemon, (control) => openProject(control, repo, "repo"));
    const parent = project.pane;
    const opened = await launchShell(run.env);
    app = opened.app;
    const page = opened.page;
    await enterWorkspace(page, "repo");
    // The core lists a linked worktree once a page shows its repository.
    await withControl(daemon, async (control) => {
      await openWorktree(control, existingCheckout);
      await showTabOf(control, parent);
    });
    // The agent in the pane is the synthetic provider; this case observes its own native session report.
    const caller = await agentPane(fixture, daemon.stateDir, parent);
    const purpose = await caller(["workspace", "purpose", "Spawn delegation fixture"]);
    expect(purpose.status, purpose.out).toBe(0);
    const command = (args: string[]) => hideAnswer(caller, args);
    await expect(page.locator(`[data-pane-view="${parent}"]`)).toHaveAttribute("data-focused", "true");
    await expectCoreFocus(daemon, parent);
    const keyboard = page.locator(`[data-pane-view="${parent}"] .xterm-helper-textarea`);
    await keyboard.focus();
    const children: SpawnedAgent[] = [];
    let registered: SpawnedAgent | undefined;
    for (const [name, branch] of [["child-existing", "existing-task"], ["child", "child-task"]]) {
      const spawned = await command(["agent", "spawn", "--parent", "here", "--name", name!, "--intent", name!, "--kind", "claude", "--repo", repo, "--branch", branch!]);
      // The spawn registers its caller as the child's parent; its own record is read back by `show here`.
      registered ??= await command(["agent", "show", "here"]);
      if (children.length === 0) expect(registered.machine).toBe(nodeOf(run.env));
      expect(spawned.parent).toBe(registered.id);
      expect(spawned.origin).toBe(registered.id);
      expect(spawned.pane.split(":")[0]).not.toBe(parent.split(":")[0]);
      await expect(page.locator(`[data-pane-view="${parent}"]`)).toHaveAttribute("data-focused", "true");
      await expect(keyboard).toBeFocused();
      await expectCoreFocus(daemon, parent);
      expect(spawned.watch).toBeTruthy();
      children.push(spawned);
    }
    const child = children[1]!;
    const parentRow = page.locator(`nav[data-sidebar] [data-pane="${parent}"]`);
    await expect(parentRow).toHaveAttribute("data-delegated", "false");
    // The root folds its children behind its chevron; the pane header's tree
    // button lists them with a branch that differs (agent-hierarchy-screens B10, B18, B21).
    await expect(parentRow.locator(`[data-tree-chevron="${parent}"]`)).toBeVisible();
    await expect(page.locator(`nav[data-sidebar] [data-pane="${child.pane}"]`)).toHaveCount(0);
    await parentRow.locator(`[data-agent-open="${parent}"]`).click();
    const treeButton = page.locator(`[data-pane-view="${parent}"] [data-pane-tree="${parent}"]`);
    await treeButton.click();
    const childRow = page.locator(`[data-agent-child="${child.pane}"]`);
    await expect(childRow.locator('[data-branch-chip="child-task"]')).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(treeButton).toBeFocused();
    await screenshot(page, "agent-spawn-delegation");
    await captureNativeWindow(app, "agent-spawn-delegation-native", { parent, children, state: run.env.HIDE_STATE_DIR, home: run.env.HOME, head: execFileSync("git", ["rev-parse", "HEAD"], { cwd: path.resolve(__dirname, "../.."), encoding: "utf8" }).trim(), provider: "synthetic native CLI" });
  } catch (error) {
    failures.push(error);
  }
  await finishNative(failures, [["close the app", () => app?.close()], ["stop the stack", () => stack.stop()]]);
});
