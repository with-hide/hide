import { describe, expect, it } from "vitest";
import { pushOpenAddress } from "./pushOpen";

const ROOM = "A".repeat(43);

describe("pushOpenAddress", () => {
  it("opens the agent a tapped notice named on its computer", () => {
    expect(pushOpenAddress({ room: ROOM, tag: "mini|w1:p1" })).toBe(`/?host=${ROOM}&open=mini%7Cw1%3Ap1`);
  });

  it("opens the computer's list for a notice about the computer itself", () => {
    expect(pushOpenAddress({ room: ROOM, tag: "" })).toBe(`/?host=${ROOM}`);
  });
});
