import { afterEach, describe, expect, it } from "vitest";
import { emptyScope } from "../test/legacyAgentScope";
import { createActions } from "./actions";
import type { SnapshotRest } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";

// One rule opens a pane for the operator: the event that focuses it also
// brings its device forward when that device is not in front. A parent row's
// child row, a child chip, a band's Open, a relationship step and a retry all
// reach a pane through `followRelation`, so they share this table with
// `openAgent` (docs/UI_BEHAVIOR.md, parent row opens a device child).
const MINI_PANE = "remote:mini:pane:p1";
const LOCAL_PANE = "t1-pane";

function world(front: "local" | "mini"): SnapshotRest {
  const miniTab = { id: "remote:mini:tab:m1", label: "m1", panes: [{ id: MINI_PANE, device_id: "mini" }], workspace_id: "remote:mini:workspace:w", checkout_id: "remote:mini:checkout:c", empty: false, delegated: false };
  const miniCheckout = { agent_scope: emptyScope(), id: "remote:mini:checkout:c", workspace_id: "remote:mini:workspace:w", path: "/mini", label: "api", tabs: [miniTab], active_tab_id: miniTab.id, strip: [] };
  return {
    navigator: { focused_device_id: front, devices: [{ id: "local", label: "This Mac", kind: "local", state: "local" }, { id: "mini", label: "mini", kind: "remote", state: "connected" }], workspaces: [] },
    status: {
      pane_focus_request: null,
      remote: [{ target_id: "mini", state: "connected", session: { workspaces: [{ agent_scope: emptyScope(), id: "remote:mini:workspace:w", label: "api", device_id: "mini", checkouts: [miniCheckout] }], agents: [{ id: "b", pane_id: MINI_PANE, identity_label: "reviewer", agent_kind: "codex", mark: "going", status_code: "working" }], active_tab_ids: {}, focused_workspace_id: null, focused_checkout_id: null, focused_tab_id: null, focused_pane_id: null, pane_layouts: [] } }],
    },
  } as unknown as SnapshotRest;
}

const savedShell = useShellStore.getState();
const savedUi = useUiStore.getState();
afterEach(() => {
  useShellStore.setState(savedShell, true);
  useUiStore.setState(savedUi, true);
});

type Sent = { kind: string; payload: Record<string, unknown> };

const cases = [
  { name: "a mini pane while this Mac is in front", front: "local", pane: MINI_PANE, forward: true },
  { name: "a mini pane while the mini is in front", front: "mini", pane: MINI_PANE, forward: false },
  { name: "a pane of this Mac while the mini is in front", front: "mini", pane: LOCAL_PANE, forward: true },
  { name: "a pane of this Mac while this Mac is in front", front: "local", pane: LOCAL_PANE, forward: false },
] as const;

const ways = {
  "a child row or chip, a band Open, a relationship step": (pane: string) => (actions: ReturnType<typeof createActions>) => actions.followRelation("parent", pane, "child"),
  "an agent row": (pane: string) => (actions: ReturnType<typeof createActions>) => actions.openAgent(pane),
};

describe.each(Object.entries(ways))("opening a pane from %s", (_way, make) => {
  it.each(cases)("brings the device forward only when needed: $name", ({ front, pane, forward }) => {
    useShellStore.setState({ rest: world(front) });
    const sent: Sent[] = [];
    make(pane)(createActions((event) => { sent.push(event as Sent); return true; }));
    expect(sent).toHaveLength(1);
    const event = sent[0]!;
    if (pane === MINI_PANE) expect(event).toMatchObject({ kind: "remote_control", payload: { target_id: "mini", action: "focus_pane", pane_id: pane } });
    else expect(event).toMatchObject({ kind: "focus_pane", payload: { pane_id: pane, origin: "operator" } });
    expect(Boolean(event.payload.focus_device)).toBe(forward);
    if (_way !== "an agent row") expect(event.payload.request_id).toEqual(expect.any(String));
    if (_way !== "an agent row" && pane === MINI_PANE) expect(event.payload.report_pane_focus_outcome).toBe(true);
  });
});

it("asks again as a new request after a failed answer, still bringing the device forward", () => {
  useShellStore.setState({ rest: world("local") });
  const sent: Sent[] = [];
  const actions = createActions((event) => { sent.push(event as Sent); return true; });
  actions.followRelation("parent", MINI_PANE, "child");
  const first = useUiStore.getState().relation!;
  const answered = world("local");
  answered.status = { ...answered.status, pane_focus_request: { request_id: first.requestId, target_pane_id: MINI_PANE, phase: "failed", message: "refused", retryable: true } } as never;
  useShellStore.setState({ rest: answered });
  actions.retryRelation();
  expect(sent).toHaveLength(2);
  expect(sent[1]!.payload).toMatchObject({ target_id: "mini", action: "focus_pane", pane_id: MINI_PANE, focus_device: true });
  expect(sent[1]!.payload.request_id).not.toBe(first.requestId);
});
