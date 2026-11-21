import { beforeEach, describe, expect, it } from "vitest";
import { appliedIn, caughtUp, isOperatorFocus, numbered, operatorFocusClientId } from "./operatorFocus";
import { useShellStore } from "./store";

const focus = (origin: string) => ({ schema_version: 2, kind: "focus_pane", payload: { pane_id: "p1", origin } });

function snapshotFrame(operator_focus: { client_id: string; sequence: number }[]) {
  return { type: "snapshot", payload: { revision: 1, rest: { focused: { pane_id: "p1", operator_focus } } } };
}

describe("operator focus numbering", () => {
  beforeEach(() => useShellStore.setState({ operatorFocusSent: 0, operatorFocusApplied: 0 }));

  it("numbers only this machine's operator focus and a card the operator opens", () => {
    expect(isOperatorFocus(focus("operator"))).toBe(true);
    expect(isOperatorFocus(focus("restore"))).toBe(false);
    expect(isOperatorFocus({ kind: "key", payload: { origin: "operator" } })).toBe(false);
    // A card open may focus its owner's pane; the card takes the keys only from the snapshot that includes it.
    expect(isOperatorFocus({ kind: "detail_open", payload: { item: { kind: "mission", mission: "m1" } } })).toBe(true);
    expect(isOperatorFocus({ kind: "pr_open", payload: { workspace_id: "w1", pr_number: 3 } })).toBe(true);
    expect(numbered(focus("operator"), 7).payload).toMatchObject({ client_id: operatorFocusClientId, sequence: 7, origin: "operator" });
  });

  it("reads only this page's number from the snapshot", () => {
    const rest = snapshotFrame([{ client_id: "someone-else", sequence: 9 }, { client_id: operatorFocusClientId, sequence: 3 }]).payload.rest;
    expect(appliedIn(rest)).toBe(3);
    expect(appliedIn({ focused: { pane_id: "p1" } })).toBeNull();
  });

  it("holds DOM focus back until the snapshot includes the last number sent", () => {
    const { noteOperatorFocusSent, applyFrame } = useShellStore.getState();
    expect(caughtUp(useShellStore.getState())).toBe(true);
    noteOperatorFocusSent(2);
    expect(caughtUp(useShellStore.getState())).toBe(false);
    applyFrame(snapshotFrame([{ client_id: operatorFocusClientId, sequence: 1 }]));
    expect(caughtUp(useShellStore.getState())).toBe(false);
    applyFrame(snapshotFrame([{ client_id: operatorFocusClientId, sequence: 2 }]));
    expect(caughtUp(useShellStore.getState())).toBe(true);
    // An older answer arriving late never lowers what was applied.
    applyFrame(snapshotFrame([{ client_id: operatorFocusClientId, sequence: 1 }]));
    expect(caughtUp(useShellStore.getState())).toBe(true);
  });

  it("does not wait on a number a closed socket lost", () => {
    const { noteOperatorFocusSent, releaseOperatorFocus } = useShellStore.getState();
    noteOperatorFocusSent(4);
    releaseOperatorFocus();
    expect(caughtUp(useShellStore.getState())).toBe(true);
  });

  it("reads its own entry gone after it was seen as dropped by the core, not as still waiting", () => {
    const { noteOperatorFocusSent, applyFrame } = useShellStore.getState();
    // Before any answer, an absent entry only means the core has not applied one.
    noteOperatorFocusSent(1);
    applyFrame(snapshotFrame([{ client_id: "someone-else", sequence: 4 }]));
    expect(caughtUp(useShellStore.getState())).toBe(false);
    applyFrame(snapshotFrame([{ client_id: operatorFocusClientId, sequence: 1 }]));
    expect(caughtUp(useShellStore.getState())).toBe(true);

    // Later clicks are sent; the core then drops this page's entry (16 other pages spoke).
    noteOperatorFocusSent(3);
    expect(caughtUp(useShellStore.getState())).toBe(false);
    applyFrame(snapshotFrame([{ client_id: "someone-else", sequence: 5 }]));
    expect(caughtUp(useShellStore.getState())).toBe(true);
  });
});
