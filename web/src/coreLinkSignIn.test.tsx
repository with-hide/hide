// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { ConnectionBadge } from "./badge";
import { TooltipProvider } from "./components/ui/tooltip";
import type { CoreLink } from "./coreLink";
import { useShellStore } from "./store";
import type { DispatchFn } from "./ws";

// The shell's modules reach xterm, which asks jsdom for a canvas it lacks.
vi.hoisted(() => {
  HTMLCanvasElement.prototype.getContext = () => null;
});

afterEach(() => {
  document.body.innerHTML = "";
});

async function mount(next: Record<string, unknown>, render: (actions: ReturnType<typeof createActions>) => React.ReactNode) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} unobserve() {} });
  const events: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => {
    events.push(event);
    return true;
  });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const saved = useShellStore.getState();
  await act(async () => {
    useShellStore.setState(next as never);
    root.render(<TooltipProvider>{render(actions)}</TooltipProvider>);
  });
  const q = (selector: string) => document.body.querySelector(selector) as HTMLElement | null;
  const button = (text: string) => [...document.body.querySelectorAll("button")].find((element) => element.textContent === text) ?? null;
  return { events, q, button, set: (state: Record<string, unknown>) => act(async () => useShellStore.setState(state as never)), unmount: async () => { await act(async () => root.unmount()); useShellStore.setState(saved, true); } };
}

const sent = (events: Parameters<DispatchFn>[0][], kind: string) => events.filter((event) => event.kind === kind);

const typeInto = async (field: HTMLInputElement, text: string) => {
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(field, text);
    field.dispatchEvent(new Event("input", { bubbles: true }));
  });
};

const link = (over: Partial<CoreLink>): CoreLink => ({ phase: "connecting", machine: null, alias: "mini", ...over });

// PRD system-ssh-device-link B6, B8, B10, B33: what ssh asks the operator's own dial to the core shows under the
// window's strip as on a device's row, and the answer goes to the question it names and is cleared from the field.
it("answers what ssh asks the core link on the window's strip and clears the field", async () => {
  const sign_in = { question: 4, kind: "secret" as const, prompt: "al@mini's password: ", key_command: null };
  const view = await mount({ connection: "reconnecting", refused: false, coreLink: link({ sign_in }), rest: null }, (actions) => <ConnectionBadge actions={actions} />);
  try {
    expect(view.q('[data-connection="sign_in"]')?.textContent).toContain("Signing in to the core on");
    const field = view.q('[data-device-sign-in-field="core-link"]') as HTMLInputElement;
    expect(field.type).toBe("password");
    await typeInto(field, "hunter2");
    await act(async () => view.q('[data-device-sign-in-submit="core-link"]')?.click());
    expect(sent(view.events, "core_link")).toEqual([{ schema_version: 2, kind: "core_link", payload: { action: "sign_in", question: 4, reply: "text", text: "hunter2" } }]);
    expect(field.value).toBe("");
    await act(async () => view.q('[data-device-sign-in-cancel="core-link"]')?.click());
    expect(sent(view.events, "core_link")[1]).toEqual({ schema_version: 2, kind: "core_link", payload: { action: "sign_in", question: 4, reply: "cancel" } });
  } finally {
    await view.unmount();
  }
});

// A link that stopped offers the device row's three actions: Sign in, the terminal command for a changed key, Retry.
it("offers Sign in for a stopped sign-in and the terminal check for a changed server key", async () => {
  const view = await mount({ connection: "reconnecting", refused: false, coreLink: link({ phase: "stopped", problem: "sign_in_needed" }), rest: null }, (actions) => <ConnectionBadge actions={actions} />);
  try {
    await act(async () => view.q('[data-core-retry="sign_in"]')?.click());
    expect(view.q('[data-core-retry="sign_in"]')?.textContent).toBe("Sign in");
    expect(sent(view.events, "core_link")).toEqual([{ schema_version: 2, kind: "core_link", payload: { action: "retry" } }]);
    await view.set({ coreLink: link({ phase: "stopped", problem: "host_key_changed" }) });
    expect(view.q('[data-core-link-command="ssh mini"]')?.textContent).toContain("ssh mini");
    expect(view.q("[data-core-retry]")).toBeNull();
    await view.set({ coreLink: link({ phase: "stopped", problem: "hide_updated" }) });
    expect(view.q("[data-core-retry]")).toBeNull();
    expect(view.q("[data-core-link-command]")).toBeNull();
    await view.set({ coreLink: link({ phase: "stopped", problem: "cannot_connect" }) });
    expect(view.q('[data-core-retry="retry"]')?.textContent).toBe("Retry");
  } finally {
    await view.unmount();
  }
});
