// Add a project in the desktop app: the dialog's Browse folder asks the main
// process for macOS's folder picker, which this spec replaces with a queue of
// answers, so a pick, a cancel and a refused folder run with no native sheet.
// Clone from URL clones a local bare repository through the same dialog.

import { expect, type ElectronApplication, type Page } from "@playwright/test";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { startHerdr } from "../../web/e2e/herdr-fixture";
import { chord } from "../../web/e2e/chords";
import { toPage } from "../src/main/wirePath";
import { sendEvent, THIS_MACHINE } from "../../web/e2e/wire";
import { isolate, launch, nodeOf, test } from "./fixture";
import { compositorPresents } from "../../web/e2e/wait";

type Pick = { canceled: boolean; filePaths: string[] };

/** Replaces the picker with `answers`, one per call; returns how many calls it has had. */
async function stubPicker(app: ElectronApplication, answers: Pick[]): Promise<void> {
  await app.evaluate(({ dialog }, queued) => {
    const state = globalThis as unknown as { __picks: Pick[]; __pickCalls: number };
    state.__picks = queued;
    state.__pickCalls ??= 0;
    dialog.showOpenDialog = (async () => {
      state.__pickCalls += 1;
      const next = state.__picks.shift();
      if (!next) throw new Error("no queued pick");
      return next;
    }) as typeof dialog.showOpenDialog;
  }, answers);
}

const pickCalls = (app: ElectronApplication) => app.evaluate(() => (globalThis as unknown as { __pickCalls: number }).__pickCalls);

// @platform: The picker answers a native path, which the host spells for the wire (`wirePath.ts`: `/` on every system, a drive path on Windows).
test("Add a project picks a folder with the native picker, and a cancel or a refusal keeps the dialog", { tag: "@platform" }, async () => {
  const herdr = await startHerdr();
  const run = isolate(herdr, "add-project");
  let app: ElectronApplication | undefined;
  try {
    const home = run.env.HOME!;
    fs.mkdirSync(path.join(home, "projects", "alpha"), { recursive: true });
    fs.mkdirSync(path.join(run.root, "outside"), { recursive: true });
    // The picker answers real paths; hided keeps the canonical spelling it checked.
    const alpha = fs.realpathSync(path.join(home, "projects", "alpha"));
    const outside = fs.realpathSync(path.join(run.root, "outside"));
    const launched = await launch(run.env);
    app = launched.app;
    const page = launched.page;
    await expect(page.locator("[data-main-screen], [data-workspace-screen]")).toBeVisible({ timeout: 30_000 });
    const dialog = page.locator("[data-add-project]");
    const browse = page.locator("[data-add-project-browse]");
    const alert = dialog.locator("[data-registration-reason]");

    // The Add project row under Overview opens the dialog on this Mac, Browse folder holding the keyboard
    // (PRD first-run-onboarding B51); it is the window's one Add project, none beside Search or on Home's title (B52).
    const add = page.locator("[data-sidebar-new-workspace]");
    await expect(add).toHaveAccessibleName(/Add project/);
    await expect(page.locator("[data-sidebar-new-workspace]")).toHaveCount(1);
    await expect(page.locator("[data-sidebar-title] [data-sidebar-new-workspace], [data-main-add-project]")).toHaveCount(0);
    // The row names itself, so it carries no tooltip.
    await expect(add).toContainText("Add project");
    await add.click();
    await expect(dialog).toHaveAttribute("data-add-project", nodeOf(run.env));
    await expect(dialog.getByRole("heading", { name: "Add a project" })).toBeVisible();
    await expect(dialog.locator("[data-add-project-host]")).toHaveText(new RegExp(THIS_MACHINE));
    await expect(browse).toBeFocused();

    // A cancelled pick changes nothing: the dialog stays, with no alert and nothing pending.
    await stubPicker(app, [{ canceled: true, filePaths: [] }]);
    await page.keyboard.press("Enter");
    await expect.poll(() => pickCalls(app!)).toBe(1);
    await expect(dialog).toBeVisible();
    await expect(alert).toHaveCount(0);
    await expect(dialog.locator("[data-registration-pending]")).toHaveCount(0);

    // A folder outside home is refused by hided; the alert names it and the dialog stays for another pick.
    await stubPicker(app, [{ canceled: false, filePaths: [outside] }]);
    await browse.click();
    await expect(alert).toHaveAttribute("data-registration-reason", "outside_home");
    await expect(alert).toHaveAttribute("data-registration-path", toPage(outside));
    await expect(alert).toContainText(toPage(outside));
    await expect(browse).toBeEnabled();
    await captureWindow(app, page, "add-project-refused");

    // A folder under home registers: the dialog closes and the project appears.
    await stubPicker(app, [{ canceled: false, filePaths: [alpha] }]);
    await browse.click();
    await expect(dialog).toHaveCount(0, { timeout: 20_000 });
    await expect(page.locator("[data-project-list]")).toContainText("alpha", { timeout: 20_000 });

    // ⌘⇧N opens it again; the same folder is refused by the shell before anything is sent.
    await page.keyboard.press(chord("new_workspace", "electron"));
    await expect(dialog).toBeVisible();
    await stubPicker(app, [{ canceled: false, filePaths: [alpha] }]);
    await page.keyboard.press("Enter");
    await expect(alert).toHaveAttribute("data-registration-reason", "already_registered");
    await expect(alert).toContainText(toPage(alpha));
    await page.keyboard.press("Escape");
    await expect(dialog).toHaveCount(0);

    // Over All projects, the page a first run stays on, with a project listed, the Projects title's + still opens the same dialog, and its × closes it.
    await expect(page.locator("[data-overview-page]")).toBeVisible();
    await expect(page.locator("[data-main-add-project]")).toHaveCount(0);
    await add.click();
    await expect(dialog).toBeVisible();
    await captureWindow(app, page, "add-project-dialog");
    await dialog.getByRole("button", { name: "Close" }).click();
    await expect(dialog).toHaveCount(0);

    // A device's folders are not this Mac's to browse: its Host gets a ~/ field,
    // and what the device path answers (here: no connection) stays in the dialog.
    const state = JSON.parse(fs.readFileSync(path.join(run.env.HIDE_STATE_DIR!, "hided.json"), "utf8")) as { token: string };
    await sendEvent(page, { origin: new URL(page.url()).origin, token: state.token }, "register_device", {
      id: "ssh-none", label: "Offline box", ssh_alias: "hide-e2e-no-such-host.invalid", host_consent: false,
    });
    await page.keyboard.press(chord("new_workspace", "electron"));
    await dialog.locator("[data-add-project-host]").click();
    await page.locator('[data-host-option="ssh-none"]').click();
    await expect(dialog).toHaveAttribute("data-add-project", "ssh-none");
    await expect(browse).toHaveCount(0);
    const field = dialog.getByLabel("Folder on Offline box");
    await expect(field).toBeFocused();
    await expect(field).toHaveValue("~/");
    await field.fill("~/projects/app");
    await field.press("Enter");
    await expect(alert).toHaveAttribute("data-registration-reason", "workspace.create_unavailable");
    await expect(alert).toHaveAttribute("data-registration-path", "~/projects/app");
    await expect(alert).toContainText("This device is not connected");
    await captureWindow(app, page, "add-project-device");
    await page.keyboard.press("Escape");
    await expect(dialog).toHaveCount(0);
  } finally {
    await app?.close();
    run.cleanup();
    herdr.stop();
  }
});

test("Create new project makes a Git repository and adds it; a name already taken is refused before anything is sent", async () => {
  const herdr = await startHerdr();
  const run = isolate(herdr, "create-project");
  let app: ElectronApplication | undefined;
  try {
    const home = run.env.HOME!;
    fs.mkdirSync(path.join(home, "projects", "taken"), { recursive: true });
    fs.writeFileSync(path.join(home, "projects", "taken", "notes.md"), "mine\n");
    const projects = fs.realpathSync(path.join(home, "projects"));
    const launched = await launch(run.env);
    app = launched.app;
    const page = launched.page;
    await expect(page.locator("[data-main-screen], [data-workspace-screen]")).toBeVisible({ timeout: 30_000 });
    const dialog = page.locator("[data-add-project]");
    const view = dialog.locator("[data-create-project]");
    const name = view.getByLabel("Name");
    const location = view.locator("[data-create-project-location]");
    const preview = view.locator("[data-create-project-path]");
    const problem = view.locator("[data-create-project-problem]");
    const submit = view.locator("[data-create-project-submit]");

    // Under Browse folder, Create new project opens its own view with the name field focused;
    // with no project yet the folder goes in home.
    await page.keyboard.press(chord("new_workspace", "electron"));
    await expect(dialog.locator("[data-add-project-other-ways]")).toContainText("Other ways to add");
    await dialog.locator('[data-add-project-way="create"]').click();
    await expect(view.getByRole("heading", { name: "Create a new project" })).toBeVisible();
    await expect(name).toBeFocused();
    await expect(location).toContainText("Will be created in ~");
    await expect(submit).toBeDisabled();

    // The location row opens the native picker; the full path follows the name as it is typed.
    await stubPicker(app, [{ canceled: false, filePaths: [projects] }]);
    await location.click();
    await expect(location).toContainText("Will be created in ~/projects");
    await expect(preview).toHaveAttribute("data-create-project-path", toPage(path.join(projects, "project-name")));
    await name.fill("taken");
    await expect(problem).toHaveAttribute("data-create-project-problem", "already_exists");
    await expect(submit).toBeDisabled();
    await name.fill("a/b");
    await expect(problem).toHaveAttribute("data-create-project-problem", "name");
    await expect(submit).toBeDisabled();
    await name.fill("fresh");
    await expect(preview).toHaveAttribute("data-create-project-path", toPage(path.join(projects, "fresh")));
    await expect(problem).toHaveCount(0);
    await expect(submit).toBeEnabled();
    await captureWindow(app, page, "create-project");
    expect(fs.existsSync(path.join(projects, "fresh"))).toBe(false);

    // Enter creates it: the folder is a repository of its own and the project appears.
    await name.press("Enter");
    await expect(dialog).toHaveCount(0, { timeout: 20_000 });
    await expect(page.locator("[data-project-list]")).toContainText("fresh", { timeout: 20_000 });
    expect(fs.statSync(path.join(projects, "fresh", ".git")).isDirectory()).toBe(true);
    expect(fs.readFileSync(path.join(projects, "taken", "notes.md"), "utf8")).toBe("mine\n");

    // The next create starts beside it, and the same name again is the project already added.
    await page.keyboard.press(chord("new_workspace", "electron"));
    await dialog.locator('[data-add-project-way="create"]').click();
    await expect(location).toContainText("Will be created in ~/projects");
    await name.fill("fresh");
    await expect(problem).toHaveAttribute("data-create-project-problem", "already_registered");
    await expect(submit).toBeDisabled();

    // Back returns to the first view, the keyboard on the way it came from.
    await view.locator("[data-create-project-back]").click();
    await expect(dialog.getByRole("heading", { name: "Add a project" })).toBeVisible();
    await expect(dialog.locator('[data-add-project-way="create"]')).toBeFocused();
    await page.keyboard.press("Escape");
    await expect(dialog).toHaveCount(0);
  } finally {
    await app?.close();
    run.cleanup();
    herdr.stop();
  }
});

test("Clone from URL clones a repository into a folder under home and adds it as a project", async () => {
  const herdr = await startHerdr();
  const run = isolate(herdr, "clone-project");
  let app: ElectronApplication | undefined;
  try {
    const home = run.env.HOME!;
    // A local bare repository stands in for a remote one; its folder name is `origin`.
    const work = path.join(run.root, "work");
    fs.mkdirSync(work, { recursive: true });
    const git = (cwd: string, args: string[]) =>
      execFileSync("git", args, { cwd, env: { ...process.env, GIT_AUTHOR_NAME: "t", GIT_AUTHOR_EMAIL: "t@example.com", GIT_COMMITTER_NAME: "t", GIT_COMMITTER_EMAIL: "t@example.com" } });
    git(work, ["init", "-q", "-b", "main"]);
    fs.writeFileSync(path.join(work, "README.md"), "fixture\n");
    git(work, ["add", "."]);
    git(work, ["commit", "-q", "-m", "fixture"]);
    const bare = path.join(run.root, "origin.git");
    git(run.root, ["clone", "-q", "--bare", work, bare]);
    const url = `file://${fs.realpathSync(bare)}`;
    // `~/origin` is taken, so the default parent (home: nothing is registered yet) is refused.
    fs.mkdirSync(path.join(home, "origin"));
    const projects = fs.realpathSync(path.join(home, "projects"));

    const launched = await launch(run.env);
    app = launched.app;
    const page = launched.page;
    await expect(page.locator("[data-main-screen], [data-workspace-screen]")).toBeVisible({ timeout: 30_000 });
    const dialog = page.locator("[data-add-project]");
    // With no project registered the Projects title keeps its + (B51).
    await page.locator("[data-sidebar-new-workspace]").click();
    await expect(dialog.locator("[data-add-project-other-ways]")).toContainText("Other ways to add");
    await dialog.locator('[data-add-project-way="clone"]').click();

    // The sub-view: Back, its own title, the URL field holding the keyboard, and home as the parent.
    await expect(dialog.getByRole("heading", { name: "Clone from URL" })).toBeVisible();
    await expect(dialog.locator("[data-add-project-back]")).toBeVisible();
    const field = dialog.locator("[data-clone-url]");
    await expect(field).toBeFocused();
    await expect(dialog.locator("[data-clone-parent]")).toHaveValue("~");
    const submit = dialog.locator("[data-clone-submit]");
    await expect(submit).toBeDisabled();

    // What Git cannot clone is said at once, and Clone stays off.
    await field.fill("not a url");
    await expect(dialog.locator("[data-clone-url-reason]")).toBeVisible();
    await expect(submit).toBeDisabled();

    // The folder the URL names is shown; hided finds it taken under home.
    await field.fill(url);
    await expect(dialog.locator("[data-clone-target-reason]")).toHaveAttribute("data-clone-target-reason", "already_exists");
    await expect(submit).toBeDisabled();

    // Another parent from the folder picker frees it.
    await stubPicker(app, [{ canceled: false, filePaths: [projects] }]);
    await dialog.locator("[data-clone-browse]").click();
    await expect(dialog.locator("[data-clone-parent]")).toHaveValue(toPage(projects));
    await expect(dialog.locator("[data-clone-target]")).toHaveAttribute("data-clone-target", toPage(path.join(projects, "origin")));
    await expect(submit).toBeEnabled();
    await captureWindow(app, page, "clone-from-url-ready");

    // Clone: the repository lands under the parent, and the dialog closes when the project appears.
    await submit.click();
    await expect(dialog).toHaveCount(0, { timeout: 30_000 });
    await expect(page.locator("[data-project-list]")).toContainText("origin", { timeout: 20_000 });
    expect(fs.readFileSync(path.join(projects, "origin", "README.md"), "utf8")).toBe("fixture\n");
    expect(fs.readdirSync(projects)).toEqual(["origin"]);

    // Opened again, the parent defaults beside the project just added, where the folder is now taken.
    await page.keyboard.press(chord("new_workspace", "electron"));
    await dialog.locator('[data-add-project-way="clone"]').click();
    await expect(dialog.locator("[data-clone-parent]")).toHaveValue(toPage(projects));
    await dialog.locator("[data-clone-url]").fill(url);
    await expect(dialog.locator("[data-clone-target-reason]")).toHaveAttribute("data-clone-target-reason", "already_exists");
    await expect(dialog.locator("[data-clone-submit]")).toBeDisabled();
    // Back returns to the first view.
    await dialog.locator("[data-add-project-back]").click();
    await expect(dialog.getByRole("heading", { name: "Add a project" })).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(dialog).toHaveCount(0);
  } finally {
    await app?.close();
    run.cleanup();
    herdr.stop();
  }
});

/** A capture of the candidate window by its own id, when a run directory was named. */
async function captureWindow(app: ElectronApplication, page: Page, name: string): Promise<void> {
  const dir = process.env.HIDE_E2E_SCREENSHOT_DIR;
  // The capture tool is macOS's; other systems keep the renderer captures.
  if (!dir || process.platform !== "darwin") return;
  // The window paints on its own frame; capture after two, so the state just asserted is on screen.
  await page.evaluate(() => new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
  await compositorPresents(page);
  const source = await app.evaluate(({ BrowserWindow }) => {
    const windows = BrowserWindow.getAllWindows();
    if (windows.length !== 1) throw new Error("Expected exactly one candidate window");
    return windows[0]!.getMediaSourceId();
  });
  fs.mkdirSync(dir, { recursive: true });
  execFileSync("/usr/sbin/screencapture", ["-x", "-o", "-l", source.split(":")[1]!, path.join(dir, `${name}.png`)]);
}
