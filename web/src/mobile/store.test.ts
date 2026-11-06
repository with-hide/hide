import { describe, expect, it } from "vitest";
import type { ServerFrame } from "./protocol";
import { applyFrame, usePhone } from "./store";

// A Hide ahead of this bundle under the same phone protocol may send what the
// bundle predates (docs/ARCHITECTURE.md, the phone app): the bundle passes
// over it and keeps drawing what it knows.
describe("a bundle against a newer Hide of its phone protocol", () => {
  it("passes over a frame type it predates and leaves its screens as they were", () => {
    applyFrame({ type: "meta", push_mode: "always", other_phones: 1 });
    const before = usePhone.getState();
    expect(() => applyFrame({ type: "later_screen", items: [{ id: "x" }] } as unknown as ServerFrame)).not.toThrow();
    expect(usePhone.getState()).toBe(before);
  });

  it("reads a frame it knows that carries a field it predates", () => {
    applyFrame({ type: "meta", push_mode: "off", other_phones: 0 });
    applyFrame({ type: "meta", push_mode: "always", other_phones: 2, later_field: { on: true } } as unknown as ServerFrame);
    expect(usePhone.getState()).toMatchObject({ pushMode: "always", otherPhones: 2 });
  });
});
