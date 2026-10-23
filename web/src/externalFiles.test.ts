import { afterEach, describe, expect, it } from "vitest";
import { PENDING_GRANT_MS, notePendingGrant, pendingDeadline, settlePendingGrants, syncedGrants, useExternalStore } from "./externalFiles";

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

  it("stops naming an open the core never answered once its deadline passes, so its grant is let go (B14)", () => {
    notePendingGrant(A, "follow:1", 1_000);
    expect(pendingDeadline()).toBe(1_000 + PENDING_GRANT_MS);
    settlePendingGrants([], [], 1_000 + PENDING_GRANT_MS - 1);
    expect(syncedGrants([])).toEqual([A]);
    settlePendingGrants([], [], 1_000 + PENDING_GRANT_MS);
    expect(syncedGrants([])).toEqual([]);
    expect(pendingDeadline()).toBeNull();
  });

  it("forgets what the host said of a grant no tab stands on, so a grant handed out again is asked about again", () => {
    useExternalStore.setState({ described: { [A]: { path: "/x/a.txt", kind: "text" }, [B]: null } });
    settlePendingGrants([A], []);
    expect(useExternalStore.getState().described).toEqual({ [A]: { path: "/x/a.txt", kind: "text" } });
  });
});
