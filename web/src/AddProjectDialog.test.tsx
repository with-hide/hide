// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { AddProjectDialog } from "./AddProjectDialog";
import { TooltipProvider } from "./components/ui/tooltip";
import { useShellStore, type HomeListing } from "./store";
import { useUiStore } from "./ui";
import type { DispatchFn } from "./ws";

// The shell's modules reach xterm, which asks jsdom for a canvas it lacks.
vi.hoisted(() => {
  HTMLCanvasElement.prototype.getContext = () => null;
});

type Sent = Parameters<DispatchFn>[0];

const machines = (drives: string | null) => [
  { id: "local", kind: "local", label: null, host: { platform: "macos aarch64" } },
  { id: "ubuntu", kind: "remote", label: "Ubuntu", state: "ready", host: { platform: "linux x86_64", state: "ready", windows_drives: drives } },
  { id: "win", kind: "remote", label: "Windows", state: "unavailable", host: { platform: "windows x86_64" } },
];

const state = (drives: string | null = null, focused = "local") => ({
  connection: "live" as const,
  revision: 1,
  pathRefusal: null,
  rest: {
    ui_state: { workspace_registrations: [{ id: 7, path: "/home/me/old", device_id: "ubuntu" }] },
    navigator: { devices: machines(drives), focused_device_id: focused },
    status: { last_error: null },
  },
});

async function mount(next: ReturnType<typeof state>) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} unobserve() {} });
  const events: Sent[] = [];
  const actions = createActions((event) => { events.push(event); return true; });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const saved = useShellStore.getState();
  await act(async () => {
    useShellStore.setState(next as never);
    useUiStore.getState().openOverlay("add_project");
    root.render(<TooltipProvider><AddProjectDialog actions={actions} /></TooltipProvider>);
  });
  return {
    events,
    unmount: async () => {
      await act(async () => root.unmount());
      useUiStore.getState().closeOverlay("add_project");
      useShellStore.setState(saved, true);
    },
  };
}

const creates = (events: Sent[]) => events.filter((event) => event.kind === "create_workspace").map((event) => event.payload);
const find = (selector: string) => document.querySelector<HTMLElement>(selector);

/** What hided answers a `create_workspace` it did not forward. */
const refuse = (path: string, reason: string, confirm?: string) =>
  act(async () => useShellStore.setState({ pathRefusal: { kind: "create_workspace", path, reason, ...(confirm ? { confirm } : {}) } } as never));

afterEach(() => {
  delete window.hideHost;
  document.body.innerHTML = "";
  vi.unstubAllGlobals();
});

it("asks once about a folder outside home and sends it again with the confirmation of that path (PRD wsl-device B15)", async () => {
  const picked = "/mnt/c/work/app";
  window.hideHost = { kind: "electron", platform: "linux", pickFolder: vi.fn(async () => picked) } as never;
  const { events, unmount } = await mount(state());
  await act(async () => find("[data-add-project-browse]")!.click());
  expect(creates(events)).toEqual([{ path: picked, label: "app", initialize_git: false }]);

  await refuse(picked, "needs_confirmation");
  const question = find(`[data-registration-confirm="${picked}"]`);
  expect(question?.textContent).toContain("This folder is outside your home folder. Add it as a project?");
  expect(find("[data-registration-slow]")?.textContent).toBe("Folders on the C drive can be a little slow in WSL.");
  expect(find("[role=alert]")).toBeNull();
  expect(find("[data-add-project]")).not.toBeNull();

  await act(async () => find("[data-registration-confirm-add]")!.click());
  expect(creates(events).at(-1)).toEqual({ path: picked, label: "app", initialize_git: false, confirmed_outside_home: picked });
  expect(find("[data-registration-confirm]")).toBeNull();

  // A refusal after the confirmation is the folder's own and keeps the dialog open (B18).
  await refuse(picked, "not_found");
  expect(find("[data-registration-reason]")?.getAttribute("data-registration-reason")).toBe("not_found");
  expect(find("[data-add-project]")).not.toBeNull();
  // A folder behind one the operator cannot read is hided's `invalid_path`, as today.
  await refuse(picked, "invalid_path");
  expect(find("[data-registration-reason]")?.getAttribute("data-registration-reason")).toBe("invalid_path");
  expect(find("[role=alert]")?.textContent).toContain("The path has to be absolute, without `..`.");
  expect(find("[data-add-project]")).not.toBeNull();
  await unmount();

  // The dialog remembers no confirmation: opening it again sends the folder without one (B20).
  const again = await mount(state());
  await act(async () => find("[data-add-project-browse]")!.click());
  expect(creates(again.events)).toEqual([{ path: picked, label: "app", initialize_git: false }]);
  await again.unmount();
});

it("asks once about the project a folder outside home belongs to, and confirms that project", async () => {
  const picked = "/srv/repo/public";
  window.hideHost = { kind: "electron", platform: "linux", pickFolder: vi.fn(async () => picked) } as never;
  const { events, unmount } = await mount(state());
  await act(async () => find("[data-add-project-browse]")!.click());
  await refuse(picked, "needs_confirmation", "/srv/repo");
  expect(find("[data-registration-confirm]")?.getAttribute("data-registration-confirm")).toBe("/srv/repo");
  await act(async () => find("[data-registration-confirm-add]")!.click());
  expect(creates(events)).toEqual([
    { path: picked, label: "public", initialize_git: false },
    { path: "/srv/repo", label: "repo", initialize_git: false, confirmed_outside_home: "/srv/repo" },
  ]);
  await unmount();
});

it("leaves out the slow line for a folder outside home that is not on the C drive", async () => {
  window.hideHost = { kind: "electron", platform: "darwin", pickFolder: vi.fn(async () => "/Volumes/Work/app") } as never;
  const { unmount } = await mount(state());
  await act(async () => find("[data-add-project-browse]")!.click());
  await refuse("/Volumes/Work/app", "needs_confirmation");
  expect(find("[data-registration-confirm]")).not.toBeNull();
  expect(find("[data-registration-slow]")).toBeNull();
  await unmount();
});

const lists = (events: Sent[]) => events.filter((event) => event.kind === "home_folder_list").map((event) => event.payload);

/** What hided answers a `home_folder_list` with, as the frame handler stores it. */
const answerList = (device: string, request: string, answer: { listing?: HomeListing; reason?: string }) =>
  act(async () =>
    useShellStore.setState((current: ReturnType<typeof useShellStore.getState>) => {
      const seq = current.homeFolderSeq + 1;
      return {
        homeFolderSeq: seq,
        homeFolders: { ...current.homeFolders, [`${device}\n${request}`]: { device_id: device, request, listing: answer.listing ?? null, reason: answer.reason ?? null, seq } },
      };
    }) as never,
  );

const home: HomeListing = {
  path: "/home/me",
  label: "~",
  home: "/home/me",
  truncated: false,
  entries: [
    { name: "app", is_directory: true, repository: true, branch: "main" },
    { name: "old", is_directory: true, repository: true, branch: "main" },
    { name: "projects", is_directory: true, repositories: 3 },
    { name: "notes.txt", is_directory: false },
  ],
};

/** Types `text` into the folder list's field and lets the typed ask settle. */
const type = (text: string) =>
  act(async () => {
    const field = find("[data-folder-browser-field]") as HTMLInputElement;
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(field, text);
    field.dispatchEvent(new Event("input", { bubbles: true }));
    await new Promise((resolve) => setTimeout(resolve, 200));
  });

it("lists a device's folders through its node and adds the one chosen there (PRD device-add-project B3-B9)", async () => {
  window.hideHost = { kind: "electron", platform: "darwin", pickFolder: async () => null } as never;
  const { events, unmount } = await mount(state("/mnt", "ubuntu"));
  expect(find("[data-add-project-browse]")?.getAttribute("data-add-project-browse")).toBe("list");
  await act(async () => find("[data-add-project-browse]")!.click());
  expect(lists(events)).toEqual([{ device_id: "ubuntu", path: "~" }]);
  expect(find("[data-folder-list-state]")?.getAttribute("data-folder-list-state")).toBe("loading");

  await answerList("ubuntu", "~", { listing: home });
  expect([...document.querySelectorAll("[data-folder-row]")].map((row) => row.getAttribute("data-folder-row"))).toEqual(["app", "old", "projects", "notes.txt"]);
  expect(find('[data-folder-row="app"] [data-folder-row-branch]')?.getAttribute("data-folder-row-branch")).toBe("main");
  expect(find('[data-folder-row="old"]')?.hasAttribute("data-folder-row-added")).toBe(true);
  expect(find('[data-folder-row="projects"] [data-folder-row-repositories]')?.textContent).toBe("3 repos");
  // Home itself is never a project; the list says so and Add waits for a choice.
  expect((find("[data-folder-browser-submit]") as HTMLButtonElement).disabled).toBe(true);
  // The list it showed is the folder the core keeps for that machine (D-04).
  expect(events.filter((event) => event.kind === "add_project_folder_set").map((event) => event.payload)).toEqual([{ device_id: "ubuntu", path: "/home/me" }]);

  await act(async () => find('[data-folder-row="notes.txt"]')!.click());
  expect((find("[data-folder-browser-submit]") as HTMLButtonElement).disabled).toBe(true);
  await act(async () => find('[data-folder-row="old"]')!.click());
  expect((find("[data-folder-browser-submit]") as HTMLButtonElement).disabled).toBe(true);
  await act(async () => find('[data-folder-row="app"]')!.click());
  await act(async () => find("[data-folder-browser-submit]")!.click());
  expect(creates(events)).toEqual([{ device_id: "ubuntu", path: "/home/me/app", label: "app", initialize_git: false }]);
  await unmount();
});

it("moves the list to a typed folder and filters it by the rest of the line", async () => {
  window.hideHost = { kind: "electron", platform: "darwin", pickFolder: async () => null } as never;
  const { events, unmount } = await mount(state(null, "ubuntu"));
  await act(async () => find("[data-add-project-browse]")!.click());
  await answerList("ubuntu", "~", { listing: home });
  await type("ap");
  expect([...document.querySelectorAll("[data-folder-row]")].map((row) => row.getAttribute("data-folder-row"))).toEqual(["app"]);
  await type("~/projects/");
  expect(lists(events).at(-1)).toEqual({ device_id: "ubuntu", path: "~/projects" });
  await answerList("ubuntu", "~/projects", { listing: { path: "/home/me/projects", label: "~/projects", home: "/home/me", truncated: false, entries: [{ name: "app", is_directory: true }] } });
  expect(find("[data-folder-browser-target]")?.getAttribute("data-folder-browser-target")).toBe("/home/me/projects");
  // A whole child name typed after the folder names that child (B7, B11).
  await type("~/projects/app");
  expect(find("[data-folder-browser-target]")?.getAttribute("data-folder-browser-target")).toBe("/home/me/projects/app");
  await act(async () => find("[data-folder-browser-field]")!.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })));
  expect(creates(events)).toEqual([{ device_id: "ubuntu", path: "/home/me/projects/app", label: "app", initialize_git: false }]);
  await type("~/projects/nothing");
  expect(find("[data-folder-list-state]")?.getAttribute("data-folder-list-state")).toBe("no-match");
  await unmount();
});

it("waits for an answer newer than its ask, and moves nothing for a folder the field no longer names", async () => {
  window.hideHost = { kind: "electron", platform: "darwin", pickFolder: async () => null } as never;
  const { events, unmount } = await mount(state(null, "ubuntu"));
  await act(async () => find("[data-add-project-browse]")!.click());
  await answerList("ubuntu", "~", { listing: home });
  const projects = (names: string[]): HomeListing => ({ path: "/home/me/projects", label: "~/projects", home: "/home/me", truncated: false, entries: names.map((name) => ({ name, is_directory: true })) });
  const rows = () => [...document.querySelectorAll("[data-folder-row]")].map((row) => row.getAttribute("data-folder-row"));
  await type("~/projects/");
  await answerList("ubuntu", "~/projects", { listing: projects(["old"]) });
  expect(rows()).toEqual(["old"]);
  await type("~/");
  await answerList("ubuntu", "~", { listing: home });
  // Asked again, the folder's earlier answer is not this ask's: the list loads until a new one arrives.
  await type("~/projects/");
  expect(lists(events).filter((asked) => asked.path === "~/projects")).toHaveLength(2);
  expect(find("[data-folder-list-state]")?.getAttribute("data-folder-list-state")).toBe("loading");
  expect((find("[data-folder-browser-submit]") as HTMLButtonElement).disabled).toBe(true);
  await answerList("ubuntu", "~/projects", { listing: projects(["new"]) });
  expect(rows()).toEqual(["new"]);
  // A typed folder that names nothing keeps the list, and Add does not take the folder shown instead.
  await type("~/projcts/");
  await answerList("ubuntu", "~/projcts", { reason: "not_found" });
  expect(find("[data-folder-browser-hint]")?.getAttribute("data-folder-browser-hint")).toBe("not_found");
  expect((find("[data-folder-browser-submit]") as HTMLButtonElement).disabled).toBe(true);
  // An answer arriving after the field was cleared moves nothing.
  await type("~/elsewhere/");
  await type("");
  await answerList("ubuntu", "~/elsewhere", { listing: { ...projects(["moved"]), path: "/home/me/elsewhere", label: "~/elsewhere" } });
  expect(rows()).toEqual(["new"]);
  await unmount();
});

it("follows a typed or completed link to the folder it names, keeping what was typed after it (review F3)", async () => {
  window.hideHost = { kind: "electron", platform: "darwin", pickFolder: async () => null } as never;
  const { events, unmount } = await mount(state(null, "ubuntu"));
  await act(async () => find("[data-add-project-browse]")!.click());
  const field = () => find("[data-folder-browser-field]") as HTMLInputElement;
  const submit = () => find("[data-folder-browser-submit]") as HTMLButtonElement;
  const rows = () => [...document.querySelectorAll("[data-folder-row]")].map((row) => row.getAttribute("data-folder-row"));
  const work: HomeListing = { path: "/home/me/work", label: "~/work", home: "/home/me", truncated: false, entries: [{ name: "api", is_directory: true }, { name: "web", is_directory: true }] };
  await answerList("ubuntu", "~", { listing: { ...home, entries: [{ name: "inward", is_directory: true }] } });
  // `~/inward` is a link to `~/work`: the machine answers with the folder's real label.
  await type("~/inward/we");
  expect(lists(events).at(-1)).toEqual({ device_id: "ubuntu", path: "~/inward" });
  await answerList("ubuntu", "~/inward", { listing: work });
  expect(field().value).toBe("~/work/we");
  expect(rows()).toEqual(["web"]);
  expect(submit().disabled).toBe(false);
  expect(find("[data-folder-browser-target]")?.getAttribute("data-folder-browser-target")).toBe("/home/me/work");
  await unmount();

  // Tab into the link from home does the same.
  const again = await mount(state(null, "ubuntu"));
  await act(async () => find("[data-add-project-browse]")!.click());
  await answerList("ubuntu", "~", { listing: { ...home, entries: [{ name: "inward", is_directory: true }] } });
  await type("~/");
  await act(async () => field().dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", bubbles: true })));
  expect(lists(again.events).at(-1)).toEqual({ device_id: "ubuntu", path: "~/inward" });
  await answerList("ubuntu", "~/inward", { listing: work });
  expect(field().value).toBe("~/work/");
  expect(rows()).toEqual(["api", "web"]);
  expect(submit().disabled).toBe(false);
  await again.unmount();
});

it("shows a typed folder it could not read with a retry, recovers on the retry, and ignores a late answer (review F4)", async () => {
  window.hideHost = { kind: "electron", platform: "darwin", pickFolder: async () => null } as never;
  const { events, unmount } = await mount(state(null, "ubuntu"));
  await act(async () => find("[data-add-project-browse]")!.click());
  const state_ = () => find("[data-folder-list-state]")?.getAttribute("data-folder-list-state");
  const submit = () => find("[data-folder-browser-submit]") as HTMLButtonElement;
  await answerList("ubuntu", "~", { listing: home });
  await type("~/private/");
  expect(state_()).toBe("loading");
  await answerList("ubuntu", "~/private", { reason: "unreadable" });
  expect(state_()).toBe("failed");
  expect(submit().disabled).toBe(true);
  // Typed again after a change, the same folder is asked again; so is a retry.
  await type("~/privat");
  await type("~/private/");
  expect(lists(events).filter((asked) => asked.path === "~/private")).toHaveLength(2);
  await answerList("ubuntu", "~/private", { reason: "unreadable" });
  await act(async () => find("[data-folder-list-retry]")!.click());
  expect(lists(events).filter((asked) => asked.path === "~/private")).toHaveLength(3);
  expect(state_()).toBe("loading");
  const opened: HomeListing = { path: "/home/me/private", label: "~/private", home: "/home/me", truncated: false, entries: [{ name: "keys", is_directory: true }] };
  await answerList("ubuntu", "~/private", { listing: opened });
  expect(state_()).toBeUndefined();
  expect(submit().disabled).toBe(false);
  // An answer for a folder the field moved away from changes nothing.
  await type("~/elsewhere/");
  await type("~/private/");
  await answerList("ubuntu", "~/elsewhere", { reason: "unreadable" });
  expect(state_()).toBeUndefined();
  expect([...document.querySelectorAll("[data-folder-row]")].map((row) => row.getAttribute("data-folder-row"))).toEqual(["keys"]);
  await unmount();
});

it("keeps the keyboard in Clone from URL's parent field while a parent is typed", async () => {
  window.hideHost = { kind: "electron", platform: "darwin", pickFolder: async () => null } as never;
  const { unmount } = await mount(state(null, "ubuntu"));
  await act(async () => find('[data-add-project-way="clone"]')!.click());
  const field = find("[data-clone-parent]") as HTMLInputElement;
  field.focus();
  for (const text of ["~", "~/w", "~/work"]) {
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(document.activeElement, text);
      document.activeElement!.dispatchEvent(new Event("input", { bubbles: true }));
    });
  }
  expect(document.activeElement).toBe(field);
  expect(field.value).toBe("~/work");
  await unmount();
});

it("asks the same question when a device's node answers that a typed folder is outside its home (PRD wsl-device B15)", async () => {
  window.hideHost = { kind: "electron", platform: "darwin", pickFolder: async () => null } as never;
  const { events, unmount } = await mount(state("/mnt", "ubuntu"));
  await act(async () => find("[data-add-project-browse]")!.click());
  await answerList("ubuntu", "~", { listing: home });
  // A Windows path pasted on a WSL device is that device's own path (B16).
  await type("C:\\work\\app");
  expect(lists(events).at(-1)).toEqual({ device_id: "ubuntu", path: "/mnt/c/work/app" });
  await answerList("ubuntu", "/mnt/c/work/app", { reason: "outside_home" });
  expect(find("[data-folder-browser-hint]")?.getAttribute("data-folder-browser-hint")).toBe("outside_home");
  await act(async () => find("[data-folder-browser-submit]")!.click());
  expect(creates(events)).toEqual([{ device_id: "ubuntu", path: "/mnt/c/work/app", label: "app", initialize_git: false }]);
  await act(async () =>
    useShellStore.setState((current: ReturnType<typeof useShellStore.getState>) => ({
      rest: { ...current.rest, status: { last_error: { kind: "workspace.create_refused.needs_confirmation", message: "outside", occurred_at: Date.now() + 1_000 } } },
    }) as never),
  );
  expect(find("[data-registration-confirm]")).not.toBeNull();
  expect(find("[data-registration-slow]")).not.toBeNull();
  await act(async () => find("[data-registration-confirm-add]")!.click());
  expect(creates(events).at(-1)).toEqual({ device_id: "ubuntu", path: "/mnt/c/work/app", label: "app", initialize_git: false, confirmed_outside_home: "/mnt/c/work/app" });

  // The node names the repository the folder belongs to; that is what is asked about and confirmed.
  await act(async () =>
    useShellStore.setState((current: ReturnType<typeof useShellStore.getState>) => ({
      rest: { ...current.rest, status: { last_error: { kind: "workspace.create_refused.needs_confirmation", message: "outside", occurred_at: Date.now() + 2_000, confirm: "/mnt/c/work" } } },
    }) as never),
  );
  expect(find("[data-registration-confirm]")?.getAttribute("data-registration-confirm")).toBe("/mnt/c/work");
  await act(async () => find("[data-registration-confirm-add]")!.click());
  expect(creates(events).at(-1)).toEqual({ device_id: "ubuntu", path: "/mnt/c/work", label: "work", initialize_git: false, confirmed_outside_home: "/mnt/c/work" });
  await unmount();
});

it("keeps a Windows path as written on a Linux device that is not WSL", async () => {
  window.hideHost = { kind: "electron", platform: "darwin", pickFolder: async () => null } as never;
  const { events, unmount } = await mount(state(null, "ubuntu"));
  await act(async () => find("[data-add-project-browse]")!.click());
  await type("C:\\work\\app");
  expect(lists(events)).toEqual([{ device_id: "ubuntu", path: "~" }]);
  await unmount();
});

it("shows a device that is not connected as one that cannot be chosen, and opens on this Mac (PRD device-add-project B31)", async () => {
  const { unmount } = await mount(state(null, "win"));
  expect(find("[data-add-project-browse]")?.getAttribute("data-add-project-browse")).toBe("list");
  expect((find("[data-add-project-browse]") as HTMLButtonElement).disabled).toBe(false);
  await unmount();
});
