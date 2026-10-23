import { afterEach, describe, expect, it } from "vitest";
import { notePendingGrant, settlePendingGrants, syncedGrants, useExternalStore } from "./externalFiles";

const A = "a".repeat(32);
const B = "b".repeat(32);

afterEach(() => useExternalStore.setState({ pending: {}, described: {} }));

// The host lets go of every grant a sync leaves out (B14), so what a sync
// names decides which outside files stay readable.
describe("the grants a browser sync names", () => {
  it("names nothing before the shell has the core's tabs, so a reload lets go of nothing (B15)", () => {
    notePendingGrant(A, "follow:1");
    expect(syncedGrants(null)).toBeNull();
  });

  it("names an open on its way to the core with the tabs the core holds", () => {
    notePendingGrant(B, "follow:2");
    expect(syncedGrants([A])).toEqual([A, B]);
  });

  it("stops naming an open once the core shows it or answers it, so a refused open is let go", () => {
    notePendingGrant(A, "follow:1");
    notePendingGrant(B, "follow:2");
    settlePendingGrants([A], [{ request_id: "follow:2" } as never]);
    expect(useExternalStore.getState().pending).toEqual({});
    expect(syncedGrants([])).toEqual([]);
  });
});
