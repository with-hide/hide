// A native caller hands off roots through the real hided.
import { expect, type ElectronApplication } from "@playwright/test";
import { execFileSync } from "node:child_process";
import path from "node:path";
import { agentPane } from "../../web/e2e/agent-pane";
import { openProject, openWorktree, showTabOf, withControl } from "../../web/e2e/daemon-actions";
import { enterWorkspace, openTaskFolds, showTasks } from "../../web/e2e/wire";
import { launchShell, screenshot, startStack, test } from "./fixture";
import { captureNativeWindow } from "./native-window";
import { coreFocus, expectCoreFocus, hideAnswer, makeSpawnRepository, type SpawnedAgent } from "./agent-spawn-fixture";

test("handoff roots have provenance without delegation or a focus change in either checkout path", async () => {
  const stack = await startStack("agent-handoff", { agents: false });
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
    const caller = await agentPane(fixture, daemon.stateDir, parent);
    const purpose = await caller(["workspace", "purpose", "Spawn handoff fixture"]);
    expect(purpose.status, purpose.out).toBe(0);
    const command = (args: string[]) => hideAnswer(caller, args);
    await expect(page.locator(`[data-pane-view="${parent}"]`)).toHaveAttribute("data-focused", "true");
    const keyboard = page.locator(`[data-pane-view="${parent}"] .xterm-helper-textarea`);
    await keyboard.focus();
    await expectCoreFocus(daemon, parent);
    const roots: SpawnedAgent[] = [];
    let registered: SpawnedAgent | undefined;
    for (const [name, branch] of [["handoff-existing", "existing-task"], ["handoff-new", "handoff-task"]]) {
      const root = await command(["agent", "spawn", "--name", name!, "--intent", name!, "--kind", "claude", "--repo", repo, "--branch", branch!]);
      // The first spawn registers its caller as the origin, and the caller's own record is read back by `show here`.
      registered ??= await command(["agent", "show", "here"]);
      if (roots.length === 0) expect(registered.origin).toBeNull();
      expect(root.parent).toBeNull();
      expect(root.origin).toBe(registered.id);
      expect(root.watch).toBeNull();
      expect(root.pane).not.toBe(parent);
      expect(root.pane.split(":")[0]).not.toBe(parent.split(":")[0]);
      await expect(page.locator(`[data-pane-view="${parent}"]`)).toHaveAttribute("data-focused", "true");
      await expect(keyboard).toBeFocused();
      expect(await coreFocus(daemon)).toBe(parent);
      const shown = await command(["agent", "show", root.id]);
      expect(shown).toMatchObject({ parent: null, origin: registered.id, watch: null });
      roots.push(root);
    }
    const parentRow = page.locator(`nav[data-sidebar] [data-pane="${parent}"]`);
    await expect(parentRow).toHaveAttribute("data-waiting", "false");
    await expect(parentRow.locator("[data-tree-chevron], [data-descendant-mark]")).toHaveCount(0);
    for (const root of roots) {
      const row = page.locator(`nav[data-sidebar] [data-pane="${root.pane}"]`);
      await expect(row).toHaveAttribute("data-delegated", "false");
      await expect(row).toHaveAttribute("data-depth", "0");
    }
    await expect(page.locator(`[data-pane-children="${parent}"]`)).toHaveCount(0);
    const facts = { parent, roots, state: run.env.HIDE_STATE_DIR, home: run.env.HOME, socket: fixture.socket, head: execFileSync("git", ["rev-parse", "HEAD"], { cwd: path.resolve(__dirname, "../.."), encoding: "utf8" }).trim(), provider: "synthetic native CLI" };
    await screenshot(page, "agent-handoff-roots");
    await captureNativeWindow(app, "agent-handoff-roots-native", facts).catch((error) => failures.push(error));
    // Each handed-off root is its own task in the sidebar's 태스크 list, not a child of the spawner's (PRD sidebar-task-lens B4, B7).
    const tasks = await showTasks(page);
    await openTaskFolds(page);
    for (const pane of [parent, ...roots.map((root) => root.pane)]) await expect(tasks.locator(`[data-task-row="${pane}"]`)).toHaveCount(1);
    await expect(tasks.locator(`[data-task-row="${parent}"] [data-task-children]`)).toHaveCount(0);
    await expect(tasks.locator("[data-task-child]")).toHaveCount(0);
    await screenshot(page, "agent-handoff-tasks");
    await captureNativeWindow(app, "agent-handoff-tasks-native", facts).catch((error) => failures.push(error));
  } catch (error) {
    failures.push(error);
  } finally {
    try { await app?.close(); } catch (error) { failures.push(error); }
    try { stack.stop(); } catch (error) { failures.push(error); }
  }
  if (failures.length) throw new AggregateError(failures, "native handoff fixture or teardown failed");
});
