import { describe, expect, it } from "vitest";
import type { StoreAsk } from "./snapshot";
import { storeAskNotice, storeAskNoticeChanges, storeAskNoticeKey } from "./storeAskNotice";
import { withNotice, type ShownNotice } from "./ui";

const install = (id: string, names: string[] = ["structure-check"]): StoreAsk => ({ id, device_id: "mac", pane_id: "w1:p1", since_unix_ms: 1, what: "install", preview: "p", names });
const t = ((key: string, options?: Record<string, unknown>) => `${key}:${JSON.stringify(options ?? {})}`) as never;

describe("a skill or MCP ask in the corner", () => {
  it("raises a notice for an ask that was not there before and none for one already shown", () => {
    const first = install("a");
    expect(storeAskNoticeChanges([first], undefined)).toEqual({ raise: [first], gone: [] });
    expect(storeAskNoticeChanges([first, install("b")], [first]).raise.map((ask) => ask.id)).toEqual(["b"]);
    expect(storeAskNoticeChanges([first], [first])).toEqual({ raise: [], gone: [] });
  });

  it("names the asks that were answered or gone so their notices leave with them", () => {
    expect(storeAskNoticeChanges([install("b")], [install("a"), install("b")])).toEqual({ raise: [], gone: ["a"] });
    expect(storeAskNoticeChanges(undefined, [install("a")]).gone).toEqual(["a"]);
  });

  it("is an important notice keyed by the ask, so the same ask raised again replaces it", () => {
    const ask = install("a");
    const notice = storeAskNotice(ask, t);
    expect(notice).toMatchObject({ level: "important", key: storeAskNoticeKey("a"), storeAsk: "a", refreshable: false });
    const shown = withNotice(withNotice([], notice), notice);
    expect(shown).toHaveLength(1);
    expect((shown as ShownNotice[])[0]!.storeAsk).toBe("a");
  });

  it("keeps a second ask beside the first rather than replacing it", () => {
    const shown = withNotice(withNotice([], storeAskNotice(install("a"), t)), storeAskNotice(install("b"), t));
    expect(shown.map((notice) => notice.storeAsk)).toEqual(["a", "b"]);
  });
});
