import { describe, expect, it } from "vitest";
import { agentReference, mentionCandidates, referenceLands, referenceText } from "./agentReference";
import type { SnapshotRest } from "./snapshot";

const reviewer = { pane_id: "w9J:p52", herdr_name: "reviewer", agent_kind: "codex", identity_label: "가입 페이지 문구 수정" };

describe("an agent reference", () => {
  it("names the agent, its pane id and its task on one line", () => {
    expect(agentReference(reviewer)).toBe("@reviewer (pane w9J:p52 · 가입 페이지 문구 수정)");
  });

  it("falls back to the agent's kind when Herdr knows it by no name", () => {
    expect(agentReference({ ...reviewer, herdr_name: undefined })).toBe("@codex (pane w9J:p52 · 가입 페이지 문구 수정)");
  });

  it("writes a device pane's id as that device's Herdr knows it", () => {
    expect(agentReference({ ...reviewer, pane_id: "remote:mini:pane:w2:p7" })).toBe("@reviewer (pane w2:p7 · 가입 페이지 문구 수정)");
  });

  it("never carries a line break, which would submit the prompt it is typed into", () => {
    const text = agentReference({ ...reviewer, identity_label: "첫 줄\n둘째 줄\r\t끝" });
    expect(text).toBe("@reviewer (pane w9J:p52 · 첫 줄 둘째 줄 끝)");
  });

  it("cuts a long task to one short line", () => {
    const text = agentReference({ ...reviewer, identity_label: "가".repeat(200) });
    expect(text).toBe(`@reviewer (pane w9J:p52 · ${"가".repeat(79)}…)`);
  });

  it("names a shell pane by its title, and by its id alone when that is its title", () => {
    expect(referenceText("build", "w9J:p3")).toBe("@build (pane w9J:p3)");
    expect(referenceText("w9J:p3", "w9J:p3")).toBe("@pane w9J:p3");
  });
});

describe("where a reference can be typed", () => {
  const rest = { navigator: { devices: [{ id: "local", kind: "local" }] }, status: { remote: [{ target_id: "mini" }] } } as unknown as SnapshotRest;

  it("lands in another pane on the same device", () => {
    expect(referenceLands(rest, "w9J:p52", "w9J:p1")).toBe(true);
    expect(referenceLands(rest, "remote:mini:pane:w2:p7", "remote:mini:pane:w2:p1")).toBe(true);
  });

  it("refuses the pane it names and a pane on another device, where its id names nothing", () => {
    expect(referenceLands(rest, "w9J:p52", "w9J:p52")).toBe(false);
    expect(referenceLands(rest, "remote:mini:pane:w2:p7", "w9J:p1")).toBe(false);
  });

  it("offers a pane's @ every other agent on its device and none elsewhere", () => {
    const agents = [{ pane_id: "w9J:p1" }, { pane_id: "w9J:p52" }, { pane_id: "remote:mini:pane:w2:p7" }];
    expect(mentionCandidates(rest, agents, "w9J:p1").map((agent) => agent.pane_id)).toEqual(["w9J:p52"]);
  });
});
