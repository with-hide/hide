// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { AddProjectDialog } from "./AddProjectDialog";
import { TooltipProvider } from "./components/ui/tooltip";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import type { DispatchFn } from "./ws";

// The shell's modules reach xterm, which asks jsdom for a canvas it lacks.
vi.hoisted(() => {
  HTMLCanvasElement.prototype.getContext = () => null;
});

type Sent = Parameters<DispatchFn>[0];

const machines = (drives: string | null) => [
  { id: "local", kind: "local", label: null, host: { platform: "macos aarch64" } },
  { id: "ubuntu", kind: "remote", label: "Ubuntu", host: { platform: "linux x86_64", state: "ready", windows_drives: drives } },
];

const state = (drives: string | null = null, focused = "local") => ({
  connection: "live" as const,
  revision: 1,
  pathRefusal: null,
  rest: {
    ui_state: { workspace_registrations: [] },
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
  await unmount();

  // The dialog remembers no confirmation: opening it again sends the folder without one (B20).
  const again = await mount(state());
  await act(async () => find("[data-add-project-browse]")!.click());
  expect(creates(again.events)).toEqual([{ path: picked, label: "app", initialize_git: false }]);
  await again.unmount();
});

it("asks about the project a folder outside home belongs to and confirms that project, not the folder", async () => {
  const picked = "/srv/repo/public";
  window.hideHost = { kind: "electron", platform: "linux", pickFolder: vi.fn(async () => picked) } as never;
  const { events, unmount } = await mount(state());
  await act(async () => find("[data-add-project-browse]")!.click());
  await refuse(picked, "needs_confirmation");
  await act(async () => find("[data-registration-confirm-add]")!.click());
  expect(creates(events).at(-1)).toEqual({ path: picked, label: "public", initialize_git: false, confirmed_outside_home: picked });

  await refuse(picked, "needs_confirmation", "/srv/repo");
  expect(find("[data-registration-confirm]")?.getAttribute("data-registration-confirm")).toBe("/srv/repo");
  await act(async () => find("[data-registration-confirm-add]")!.click());
  expect(creates(events).at(-1)).toEqual({ path: "/srv/repo", label: "repo", initialize_git: false, confirmed_outside_home: "/srv/repo" });
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

it("asks the same question when a device's helper answers that its folder is outside that device's home", async () => {
  window.hideHost = { kind: "electron", platform: "darwin", pickFolder: async () => null } as never;
  const { events, unmount } = await mount(state("/mnt", "ubuntu"));
  const field = find("[data-add-project-path]") as HTMLInputElement;
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(field, "/mnt/c/work/app");
    field.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await act(async () => find("[data-add-project-submit]")!.click());
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

  // The helper names the repository the folder belongs to; that is what is asked about and confirmed.
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

/** Pastes `text` into the path field, as the browser hands a paste to it. */
const paste = (field: HTMLInputElement, text: string) =>
  act(async () => {
    const event = new Event("paste", { bubbles: true, cancelable: true }) as Event & { clipboardData: { getData: () => string } };
    event.clipboardData = { getData: () => text };
    field.dispatchEvent(event);
  });

it("shows a Windows path pasted on a WSL device as that device's own path (PRD wsl-device B16)", async () => {
  window.hideHost = { kind: "electron", platform: "darwin", pickFolder: async () => null } as never;
  const { events, unmount } = await mount(state("/mnt", "ubuntu"));
  const field = find("[data-add-project-path]") as HTMLInputElement;
  await paste(field, "C:\\work\\app");
  expect(field.value).toBe("/mnt/c/work/app");
  await act(async () => find("[data-add-project-submit]")!.click());
  expect(creates(events).at(-1)?.path).toBe("/mnt/c/work/app");
  await unmount();
});

it("keeps a Windows path as written on a Linux device that is not WSL", async () => {
  window.hideHost = { kind: "electron", platform: "darwin", pickFolder: async () => null } as never;
  const { unmount } = await mount(state(null, "ubuntu"));
  const field = find("[data-add-project-path]") as HTMLInputElement;
  await paste(field, "C:\\work\\app");
  expect(field.value).toBe("~/");
  await unmount();
});
