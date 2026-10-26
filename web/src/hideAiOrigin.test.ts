import { describe, expect, it } from "vitest";
import { drawnInTree, hideAiCheckoutOf, hideAiPlace, hideAiWords, homeOpening, homeTurnLine, originLine, taskLine } from "./hideAiOrigin";
import { rowAccessibleName } from "./agentRow";
import { legacyAgentRow } from "../test/legacyAgentRow";
import { createInterfaceI18n } from "./i18n/instance";
import type { AgentRow, Checkout, HideAiCheckout, SnapshotRest } from "./snapshot";

const MEMORY = { kind: "memory_promote", target: "m72" };
const device = { device_id: "mac" };
const checkout = (path: string) => ({ id: path, path }) as unknown as Checkout;

describe("what Hide AI started is named from the core's codes (PRD hide-ai-origin-mark D-02, B10)", () => {
  it("words a known task in the interface language, and shows a task kind this build does not know as its code (version skew)", async () => {
    const ko = (await createInterfaceI18n("ko")).t;
    const en = (await createInterfaceI18n("en")).t;
    expect(taskLine(ko, MEMORY)).toBe("Memory 반영하기 · m72");
    expect(taskLine(en, MEMORY)).toBe("Promote Memory · m72");
    expect(taskLine(en, { kind: "label_repair" })).toBe("label_repair");
  });

  it("says Hide AI alone when the core names no task, and never invents one", async () => {
    const { t } = await createInterfaceI18n("en");
    expect(taskLine(t, undefined)).toBeNull();
    expect(originLine(t, {})).toBe("Hide AI");
    expect(hideAiPlace(t, "herdr-ide", {})).toBe("herdr-ide › Hide AI");
    expect(hideAiPlace(t, "herdr-ide", { task: MEMORY })).toBe("herdr-ide › Promote Memory · m72");
  });

  it("puts Hide AI and the task in the row's accessible name, and nothing in an operator's row (D-09, B9)", async () => {
    const { t } = await createInterfaceI18n("ko");
    expect(hideAiWords(t, { task: MEMORY })).toBe("Hide AI · Memory 반영하기 · m72");
    expect(hideAiWords(t, {})).toBe("Hide AI");
    const base = { id: "p", pane_id: "p", identity_label: "m72 결정을 문서로 옮기기", agent_kind: "claude", status_code: "stopped", detail: null } as const;
    expect(rowAccessibleName(t, legacyAgentRow({ ...base, hide_ai: { task: MEMORY } }), null)).toContain("Hide AI · Memory 반영하기 · m72");
    expect(rowAccessibleName(t, legacyAgentRow({ ...base }), null)).not.toContain("Hide AI");
  });
});

describe("what a home-owned row says and where it opens (PRD hide-ai-work-home D-14, D-16, B11, B12)", () => {
  it("words the home's turn and puts it in the accessible name after the task", async () => {
    const ko = (await createInterfaceI18n("ko")).t;
    const en = (await createInterfaceI18n("en")).t;
    expect(homeTurnLine(ko, { kind: "pr_review", pr_number: 924 })).toBe("PR #924 리뷰");
    expect(homeTurnLine(ko, { kind: "ci_failed", pr_number: 924 })).toBe("CI 실패");
    expect(homeTurnLine(en, { kind: "pr_review", pr_number: 924 })).toBe("PR #924 review");
    expect(hideAiWords(ko, { task: MEMORY, home: "memory", turn: { kind: "pr_review", pr_number: 924 } })).toBe("Hide AI · Memory 반영하기 · m72 · PR #924 리뷰");
    expect(hideAiWords(ko, { task: MEMORY, home: "memory" })).toBe("Hide AI · Memory 반영하기 · m72");
  });

  const rest = { navigator: { workspaces: [{ id: "ws-1", device_id: "mac" }] } } as unknown as SnapshotRest;
  const row = (hideAi: AgentRow["hide_ai"]) => ({ hide_ai: hideAi });

  it("opens the Project's Hide AI tool on the attempt's row, and nothing else until the core names the Project", () => {
    expect(homeOpening(rest, row({ task: MEMORY, home: "memory", workspace_id: "ws-1" }))).toEqual({ deviceId: "mac", workspaceId: "ws-1", row: "m72" });
    expect(homeOpening(rest, row({ task: { kind: "suggestion_apply", target: "s12" }, home: "suggestions", workspace_id: "ws-1" }))).toEqual({ deviceId: "mac", workspaceId: "ws-1", row: "s12" });
    const unnamed: [string, AgentRow["hide_ai"]][] = [
      ["no home (a kind this build does not know)", { task: MEMORY, workspace_id: "ws-1" }],
      ["no Project yet", { task: MEMORY, home: "memory" }],
      ["a Project the catalog lacks", { task: MEMORY, home: "memory", workspace_id: "other" }],
      ["a task naming no card", { task: { kind: "memory_promote" }, home: "memory", workspace_id: "ws-1" }],
      ["not a card name", { task: { kind: "memory_promote", target: "72" }, home: "memory", workspace_id: "ws-1" }],
    ];
    for (const [name, hideAi] of unnamed) expect(homeOpening(rest, row(hideAi)), name).toBeNull();
    expect(homeOpening(rest, undefined)).toBeNull();
  });
});

describe("a project's tree draws a checkout Hide AI made only while it matters (D-17, B3)", () => {
  const listed: HideAiCheckout[] = [{ device_id: "mac", path: "/repo.worktrees/hide-memory-m72", task: MEMORY }];
  const made = checkout("/repo.worktrees/hide-memory-m72");

  it("hides it unless it is in front or an operator's agent works in it, and never hides an operator's checkout", () => {
    expect(drawnInTree(listed, device, made, false, false)).toBe(false);
    expect(drawnInTree(listed, device, made, true, false)).toBe(true);
    expect(drawnInTree(listed, device, made, false, true)).toBe(true);
    expect(drawnInTree(listed, device, checkout("/repo.worktrees/feature"), false, false)).toBe(true);
  });

  it("joins by device and path, never by a name that looks like Hide AI's", () => {
    const lookalike = checkout("/repo.worktrees/hide-memory-m1-abc");
    expect(hideAiCheckoutOf(listed, device, lookalike)).toBeNull();
    expect(hideAiCheckoutOf(listed, { device_id: "mini" }, made)).toBeNull();
    expect(drawnInTree(undefined, device, lookalike, false, false)).toBe(true);
    expect(drawnInTree(listed, { device_id: "mini" }, made, false, false)).toBe(true);
  });
});
