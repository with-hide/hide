// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { emptyScope } from "../../test/legacyAgentScope";
import { createActions } from "../actions";
import { TooltipProvider } from "../components/ui/tooltip";
import type { AgentProfileAnswer, Device, KitAgent, KitPiece, MachineAgentProfiles } from "../snapshot";
import { useStartPanel } from "../startDraft";
import { useShellStore } from "../store";
import type { DispatchFn } from "../ws";
import { AgentsTab } from "./AgentsTab";

// The shell's modules reach xterm, which asks jsdom for a canvas it lacks.
vi.hoisted(() => {
  HTMLCanvasElement.prototype.getContext = () => null;
  Element.prototype.scrollIntoView = () => {};
});

const ok: KitPiece = { state: "installed", why: null, location: null };
const agent = (id: string, label: string, over: Partial<KitAgent> = {}): KitAgent => ({ id, label, availability: "available", enabled: true, chosen: true, skill: ok, hook: ok, doc_url: "", ...over });
const AGENTS = [agent("claude-code", "Claude Code"), agent("codex", "Codex"), agent("antigravity", "Antigravity"), agent("omp", "omp", { availability: "not_installed", enabled: false, chosen: false })];

const listed = (profiles: MachineAgentProfiles["profiles"] = [], defaults: Record<string, string> = {}): MachineAgentProfiles => ({ state: "listed", version: 3, profiles, defaults });
const work = { id: "p-work", kind: "claude-code", name: "work", account: "/home/me/accounts/work", program: null, env_keys: ["A"], version: 1, install: "own" as const, folder: "/home/me/accounts/work", hook: ok, skill: ok, startable: true };

function device(profiles: MachineAgentProfiles | undefined): Device {
  return { agent_scope: emptyScope(), id: "local", label: "mini", kind: "local", state: "local", message: null, ssh_alias: null, agent_count: 0, test: null, kit: { unavailable: null, busy: false, components: [], agents: AGENTS, offers_reinstall: false, shares_account_with: null, profiles } } as Device;
}

afterEach(() => {
  document.body.innerHTML = "";
});

async function mount(profiles: MachineAgentProfiles | undefined, workspaces: unknown[] = []) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} unobserve() {} });
  const events: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => { events.push(event); return true; });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const saved = useShellStore.getState();
  const rest = (answer?: AgentProfileAnswer) => ({ navigator: { focused_device_id: "local", devices: [device(profiles)], workspaces }, status: { agent_profile_answer: answer }, ui_state: {} });
  await act(async () => {
    useShellStore.setState({ connection: "live", rest: rest() } as never);
    root.render(<TooltipProvider><AgentsTab actions={actions} /></TooltipProvider>);
  });
  // Dialogs draw into the body, outside the tab.
  const q = (selector: string) => document.body.querySelector(selector) as HTMLElement | null;
  const all = (selector: string) => [...document.body.querySelectorAll(selector)] as HTMLElement[];
  const click = async (element: HTMLElement | null) => { await act(async () => { element!.click(); }); };
  const type = async (field: HTMLElement | null, value: string) => {
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(field, value);
      field!.dispatchEvent(new Event("input", { bubbles: true }));
    });
  };
  const changes = () => events.filter((event) => event.kind === "agent_profile_change").map((event) => event.payload as { device_id: string; request_id: string; change: Record<string, unknown> });
  /** The core's answer to the last change sent. */
  const answer = async (over: Partial<AgentProfileAnswer>) => {
    const last = changes().at(-1)!;
    await act(async () => { useShellStore.setState({ rest: rest({ request_id: last.request_id, device_id: "local", state: "done", ...over }) } as never); });
  };
  return { q, all, click, type, changes, answer, events, unmount: async () => { await act(async () => root.unmount()); useShellStore.setState(saved, true); } };
}

it("adds nothing to the tab on a machine whose profiles were never read, and only an add button per agent with no profile (B1)", async () => {
  const unread = await mount(undefined);
  expect(unread.all("[data-inline-profiles]")).toEqual([]);
  await unread.unmount();

  const empty = await mount(listed());
  // Installed agents that keep accounts apart, Antigravity included for its program and variables.
  expect(empty.all("[data-inline-profiles]").map((row) => row.dataset.inlineProfiles)).toEqual(["claude-code", "codex", "antigravity"]);
  expect(empty.all("[data-profile-row]")).toEqual([]);
  expect(empty.all("[data-profile-add]")).toHaveLength(3);
  await empty.unmount();
});

it("adds a profile through the core, shows the machine's refusal under its field, and closes once it is added (B3, B4)", async () => {
  const tab = await mount(listed());
  await tab.click(tab.q('[data-inline-profiles="claude-code"] [data-profile-add]'));
  expect(tab.q("[data-profile-dialog='add']")).not.toBeNull();
  await tab.type(tab.q("#profile-add-name"), " work ");
  await tab.type(tab.q("#profile-add-account"), "~/.claude");
  await tab.click(tab.q("[data-profile-submit]"));
  expect(tab.changes().at(-1)).toMatchObject({ device_id: "local", change: { op: "add", kind: "claude-code", name: "work", account: "~/.claude", program: null, env: [] } });

  // The node's reason is worded by the screen, not in the node's English.
  await tab.answer({ state: "refused", code: "profile_refused", field: "account", reason: "account_is_default", text: "the folder is where the default profile reads" });
  expect(tab.q("[data-profile-dialog='add']")?.textContent).toContain("This is the agent's own folder. Use the Default profile for it.");
  expect(tab.q("[data-profile-dialog='add']")?.textContent).not.toContain("where the default profile reads");
  expect(tab.q("#profile-add-account")?.getAttribute("aria-invalid")).toBe("true");
  // A bound names its number; a node that sends no reason reads generically.
  await tab.click(tab.q("[data-profile-submit]"));
  await tab.answer({ state: "refused", code: "profile_refused", field: "name", reason: "name_long", limit: 64, text: "x" });
  expect(tab.q("[data-profile-dialog='add']")?.textContent).toContain("A name is at most 64 bytes.");
  await tab.click(tab.q("[data-profile-submit]"));
  await tab.answer({ state: "refused", code: "profile_refused", field: "name", text: "A profile needs a name" });
  expect(tab.q("[data-profile-dialog='add']")?.textContent).toContain("The machine refused this value.");

  await tab.type(tab.q("#profile-add-account"), "~/accounts/work");
  await tab.click(tab.q("[data-profile-submit]"));
  await tab.answer({ added: "p-work" });
  expect(tab.q("[data-profile-dialog]")).toBeNull();
  await tab.unmount();
});

it("edits only after the variables' values are read, and deletes after naming the projects that go back to the default (B26, B27)", async () => {
  const tab = await mount(listed([work], { "claude-code": "p-work" }));
  expect(tab.all('[data-inline-profiles="claude-code"] [data-profile-row]').map((row) => row.dataset.profileRow)).toEqual(["default", "p-work"]);
  await tab.click(tab.q('[data-profile-row="p-work"] button'));
  expect(tab.changes().at(-1)?.change).toEqual({ op: "values", id: "p-work" });
  // Until the values arrive the variables wait, and leaving a field saves nothing.
  expect(tab.q("[data-profile-env]")?.dataset.profileEnv).toBe("loading");
  await tab.answer({ values: [["A", "1"]] });
  expect(tab.q("[data-profile-env]")?.dataset.profileEnv).toBe("true");

  await tab.type(tab.q("#profile-edit-program"), "claude-gateway");
  await act(async () => { tab.q("#profile-edit-program")!.dispatchEvent(new FocusEvent("focusout", { bubbles: true })); });
  expect(tab.changes().at(-1)?.change).toEqual({ op: "edit", id: "p-work", name: "work", program: "claude-gateway", env: [["A", "1"]] });
  await tab.answer({});

  await tab.click(tab.q("[data-profile-delete]"));
  expect(tab.changes().at(-1)?.change).toEqual({ op: "remove_check", id: "p-work" });
  await tab.answer({ in_use: [], projects: ["blog"], candidates: 2, hide_ai: true });
  // What uses it, each with what the delete does: the project goes back to the default, the mission candidates stay as a missing profile (agent-profiles-phase2 B22).
  expect(tab.all("[data-profile-use]").map((row) => [row.dataset.profileUse, row.textContent])).toEqual([
    ["project", "blog · goes back to the agent default"],
    ["candidates", "2 mission worker candidates · stays as Missing profile"],
    ["hide_ai", "Hide AI · stays as Missing profile and is passed over"],
  ]);
  await tab.click(tab.q("[data-profile-delete-confirm]"));
  expect(tab.changes().at(-1)?.change).toEqual({ op: "remove", id: "p-work" });
  await tab.unmount();
});

it("keeps what was typed while the values are read, and closes only once the last edit is saved (B26)", async () => {
  const tab = await mount(listed([work]));
  await tab.click(tab.q('[data-profile-row="p-work"] button'));
  // Typed before the values arrive: kept when they do, and saved after.
  await tab.type(tab.q("#profile-edit-name"), "work-2");
  await tab.answer({ values: [["A", "1"]] });
  expect((tab.q("#profile-edit-name") as HTMLInputElement).value).toBe("work-2");
  expect(tab.changes().at(-1)?.change).toEqual({ op: "edit", id: "p-work", name: "work-2", program: null, env: [["A", "1"]] });

  // A second change while the first is on its way, then a close.
  await tab.type(tab.q("#profile-edit-program"), "claude-gateway");
  await act(async () => { document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })); });
  expect(tab.q("[data-profile-dialog='edit']")).not.toBeNull();
  await tab.answer({});
  expect(tab.changes().at(-1)?.change).toEqual({ op: "edit", id: "p-work", name: "work-2", program: "claude-gateway", env: [["A", "1"]] });
  expect(tab.q("[data-profile-dialog='edit']")).not.toBeNull();
  await tab.answer({});
  expect(tab.q("[data-profile-dialog]")).toBeNull();
  await tab.unmount();
});

it("keeps the dialog open with its values and the reason when the last edit is refused on close", async () => {
  const tab = await mount(listed([work]));
  await tab.click(tab.q('[data-profile-row="p-work"] button'));
  await tab.answer({ values: [["A", "1"]] });
  await tab.type(tab.q("#profile-edit-program"), "nope");
  await act(async () => { document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })); });
  expect(tab.changes().at(-1)?.change).toMatchObject({ op: "edit", program: "nope" });
  await tab.answer({ state: "refused", code: "profile_refused", field: "program", reason: "program_long", limit: 4096, text: "too long" });
  expect(tab.q("[data-profile-dialog='edit']")?.textContent).toContain("A program is at most 4,096 bytes.");
  expect((tab.q("#profile-edit-program") as HTMLInputElement).value).toBe("nope");
  await tab.unmount();
});

it("saves nothing on close when the values could not be read", async () => {
  const tab = await mount(listed([work]));
  await tab.click(tab.q('[data-profile-row="p-work"] button'));
  await tab.type(tab.q("#profile-edit-name"), "work-2");
  await tab.answer({ state: "refused", code: "profile_gone" });
  const sent = tab.changes().length;
  await act(async () => { document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })); });
  expect(tab.changes()).toHaveLength(sent);
  expect(tab.q("[data-profile-dialog]")).toBeNull();
  await tab.unmount();
});

it("keeps the delete dialog while the delete runs, and lists an agent that started with the profile since the check (B27)", async () => {
  const tab = await mount(listed([work]));
  await tab.click(tab.q('[data-profile-row="p-work"] button'));
  await tab.answer({ values: [["A", "1"]] });
  await tab.click(tab.q("[data-profile-delete]"));
  await tab.answer({ in_use: [], projects: [] });
  await tab.click(tab.q("[data-profile-delete-confirm]"));
  expect(tab.changes().at(-1)?.change).toEqual({ op: "remove", id: "p-work" });
  // Waiting on the core, the dialog stays and does not delete twice.
  expect(tab.q("[data-profile-delete-dialog]")).not.toBeNull();
  expect(tab.q("[data-profile-delete-confirm]")?.hasAttribute("disabled")).toBe(true);
  await tab.answer({ state: "refused", code: "profile_in_use", in_use: [{ pane_id: "w2:p1", title: "late-start", place: "blog", asleep: false }] });
  expect(tab.q("[data-profile-delete-dialog]")?.dataset.profileDeleteDialog).toBe("refused");
  expect(tab.q("[data-profile-in-use]")?.textContent).toContain("late-start");
  expect(tab.q('[data-profile-in-use-open="w2:p1"]')).not.toBeNull();
  expect(tab.q("[data-profiles-error]")).toBeNull();
  await tab.unmount();
});

it("closes the delete dialog once the profile is deleted", async () => {
  const tab = await mount(listed([work]));
  await tab.click(tab.q('[data-profile-row="p-work"] button'));
  await tab.answer({ values: [["A", "1"]] });
  await tab.click(tab.q("[data-profile-delete]"));
  await tab.answer({ in_use: [], projects: [] });
  await tab.click(tab.q("[data-profile-delete-confirm]"));
  await tab.answer({});
  expect(tab.q("[data-profile-delete-dialog]")).toBeNull();
  await tab.unmount();
});

it("refuses a delete while an agent runs or starts with the profile and lists it (B27)", async () => {
  const tab = await mount(listed([work]));
  await tab.click(tab.q('[data-profile-row="p-work"] button'));
  await tab.answer({ values: [["A", "1"]] });
  await tab.click(tab.q("[data-profile-delete]"));
  await tab.answer({ in_use: [{ pane_id: "w1:p1", title: "fix-bell", place: "blog", asleep: true }, { pane_id: "", title: "Claude Code", place: "", asleep: false, starting: true }], projects: [] });
  expect(tab.q("[data-profile-delete-dialog]")?.dataset.profileDeleteDialog).toBe("refused");
  expect(tab.q("[data-profile-in-use]")?.textContent).toContain("fix-bell");
  expect(tab.q("[data-profile-delete-confirm]")).toBeNull();
  // A start holding the profile has no pane yet: it says so and links nowhere.
  const starting = tab.q("[data-profile-in-use-starting]");
  expect(starting?.textContent).toContain("Starting");
  expect(starting?.querySelector("[data-profile-in-use-open]")).toBeNull();
  await tab.unmount();
});

it("starts the profile's agent in a new tab to sign in, with the start the start panel sends, and opens the start panel where there is no checkout (B10)", async () => {
  const signedOut = { ...work, id: "p-new", name: "new", login: "signed_out" as const };
  const blog = { agent_scope: emptyScope(), id: "w-blog", label: "blog", path: "/home/me/blog", device_id: "local", temporary: false, registered: true, checkouts: [{ id: "c-blog", workspace_id: "w-blog", label: "main", path: "/home/me/blog", exists: true, is_primary: true }] };
  const tab = await mount(listed([signedOut]), [blog]);
  await tab.click(tab.q('[data-profile-row="p-new"] [data-profile-action="open"]'));
  const starts = tab.events.filter((event) => event.kind === "agent_start_in_checkout").map((event) => event.payload as Record<string, unknown>);
  expect(starts).toEqual([expect.objectContaining({ checkout_path: "/home/me/blog", provider: "claude", profile: "p-new" })]);
  expect(useStartPanel.getState().isOpen).toBe(false);
  await tab.unmount();

  const none = await mount(listed([signedOut]));
  await none.click(none.q('[data-profile-row="p-new"] [data-profile-action="open"]'));
  expect(none.events.filter((event) => event.kind === "agent_start_in_checkout")).toEqual([]);
  expect(useStartPanel.getState().isOpen).toBe(true);
  useStartPanel.getState().close();
  await none.unmount();
});

it("keeps the last device profiles dimmed without add or edit when its connection goes away (B30)", async () => {
  const tab = await mount(listed([work], { "claude-code": "p-work" }));
  const before = tab.all('[data-inline-profiles="claude-code"] [data-profile-row]').map((row) => row.dataset.profileRow);
  expect(before).toEqual(["default", "p-work"]);
  await act(async () => {
    const rest = useShellStore.getState().rest!;
    useShellStore.setState({ rest: { ...rest, navigator: { ...rest.navigator, devices: rest.navigator!.devices!.map((row) => ({ ...row, kind: "remote", state: "unavailable" })) } } } as never);
  });
  expect(tab.all('[data-inline-profiles="claude-code"] [data-profile-row]').map((row) => row.dataset.profileRow)).toEqual(before);
  expect(tab.q('[data-inline-profiles="claude-code"]')?.className).toContain("opacity-(--opacity-dimmed)");
  expect(tab.all("[data-profile-add], [data-profile-row] button, [data-agent-switch], [data-agent-permission-select]")).toHaveLength(0);
  expect(tab.q("[data-agents-retry]")).not.toBeNull();
  expect(tab.changes()).toEqual([]);
  await tab.unmount();
});

it("keeps only profile names and the default mark when the device disconnects", async () => {
  const tab = await mount(listed([{ ...work, login: "signed_out" }], { "claude-code": "p-work" }));
  expect(tab.q('[data-profile-row="p-work"] [data-profile-state="signedOut"]')).not.toBeNull();
  await act(async () => {
    const rest = useShellStore.getState().rest!;
    useShellStore.setState({ rest: { ...rest, navigator: { ...rest.navigator, devices: rest.navigator!.devices!.map((row) => ({ ...row, kind: "remote", state: "unavailable" })) } } } as never);
  });
  expect(tab.q('[data-profile-row="p-work"]')?.textContent).toContain("work");
  expect(tab.q('[data-profile-row="p-work"] [data-profile-default]')).not.toBeNull();
  expect(tab.all("[data-profile-state], [data-profile-usage]")).toHaveLength(0);
  expect(tab.changes()).toEqual([]);
  await tab.unmount();
});
