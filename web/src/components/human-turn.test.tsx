// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "../actions";
import { clientI18n } from "../i18n/translator";
import type { SnapshotRest } from "../snapshot";
import { useShellStore } from "../store";
import { HumanTurnDone, HumanTurnLine } from "./human-turn";
import { TooltipProvider } from "./ui/tooltip";

// PRD ask-human B10, B18, B19, B21: Done sends one event naming the
// declaration; while the declaring agent's device is away it waits with the
// reason beside it; the place is text, never a link.
afterEach(async () => {
  await clientI18n.changeLanguage("en");
  vi.unstubAllGlobals();
  document.body.innerHTML = "";
});

const turn = { id: "agent-7.1700", kind: "auth", place: "github.com/login", device_id: "mini" } as const;

async function render(state: "connected" | "not_connected", node: React.ReactNode) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  await clientI18n.changeLanguage("ko");
  const rest = { status: { remote: [{ target_id: "mini", state }] }, navigator: { devices: [{ id: "mac", kind: "local" }, { id: "mini", kind: "remote", label: "mini" }] } } as unknown as SnapshotRest;
  useShellStore.setState({ rest, connection: "live" });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<TooltipProvider>{node}</TooltipProvider>));
  return { container, unmount: () => act(async () => root.unmount()) };
}

it("sends one event naming the declaration, and is named by what it settles", async () => {
  const sent: unknown[] = [];
  const actions = createActions((event) => sent.push(event));
  const { container, unmount } = await render("connected", <HumanTurnDone turn={turn} actions={actions} />);
  try {
    const done = container.querySelector<HTMLButtonElement>(`[data-human-turn-done="${turn.id}"]`)!;
    expect(done.textContent).toBe("다 했어요");
    expect(done.getAttribute("aria-label")).toBe("다 했어요: 인증 · github.com/login");
    expect(done.disabled).toBe(false);
    await act(async () => done.click());
    expect(sent).toEqual([{ schema_version: 2, kind: "human_turn_done", payload: { declaration: turn.id } }]);
  } finally {
    await unmount();
  }
});

it("waits with the connection's reason while the declaring agent's device is away", async () => {
  const sent: unknown[] = [];
  const actions = createActions((event) => sent.push(event));
  const { container, unmount } = await render("not_connected", <HumanTurnDone turn={turn} actions={actions} />);
  try {
    const done = container.querySelector<HTMLButtonElement>(`[data-human-turn-done="${turn.id}"]`)!;
    expect(done.disabled).toBe(true);
    expect(container.querySelector(`[data-human-turn-unreachable="${turn.id}"]`)?.textContent).toBeTruthy();
    await act(async () => done.click());
    expect(sent).toEqual([]);
  } finally {
    await unmount();
  }
});

it("says the kind and place as plain text with the device's chip, never a link", async () => {
  const { container, unmount } = await render("connected", <HumanTurnLine turn={turn} />);
  try {
    const place = container.querySelector<HTMLElement>(`[data-human-turn-place="${turn.id}"]`)!;
    expect(place.textContent).toBe("인증 · github.com/login");
    expect(container.querySelector("a")).toBeNull();
    expect(container.querySelector('[data-device-chip="mini"]')).not.toBeNull();
  } finally {
    await unmount();
  }
});

it("copies the whole place as the agent wrote it, query included, and shows it took", async () => {
  const written: string[] = [];
  vi.stubGlobal("navigator", { ...navigator, clipboard: { writeText: async (text: string) => void written.push(text) } });
  const actions = createActions(() => {});
  const url = { ...turn, place: "https://github.com/login/device?user_code=TEST-1234" };
  const { container, unmount } = await render("connected", <HumanTurnLine turn={url} copy={actions.copyText} />);
  try {
    const copy = container.querySelector<HTMLButtonElement>(`[data-human-turn-copy="${turn.id}"]`)!;
    expect(copy.getAttribute("aria-label")).toBe("장소 복사: https://github.com/login/device?user_code=TEST-1234");
    await act(async () => copy.click());
    expect(written).toEqual(["https://github.com/login/device?user_code=TEST-1234"]);
    expect(copy.getAttribute("aria-label")).toBe("복사됨");
    expect(container.querySelector("a")).toBeNull();
  } finally {
    await unmount();
  }
});
