import { beforeEach, describe, expect, it } from "vitest";
import { initializeInterfaceI18n } from "../i18n/instance";
import { noticeText, type ServerFrame, type StartCatalog } from "./protocol";
import { NO_CHOICE, mayHaveStarted, modelsOf, selectionOf, startFailure, startProblem, targetText } from "./start";
import { CLOSED_SHEET, applyFrame, patch, usePhone } from "./store";

/** The frame hided/src/mobile/start.rs sends, as its own test builds it. */
const FRAME = `{
  "type": "start_catalog",
  "targets": [
    {"id": "home:local", "device_id": "local", "place": null, "device_label": null, "connected": true},
    {"id": "c1", "device_id": "local", "place": "hide · main", "device_label": null, "connected": true},
    {"id": "home:mini", "device_id": "mini", "place": null, "device_label": "mini", "connected": false}
  ],
  "core_platform": "macos aarch64",
  "kinds": [
    {"id": "claude", "models": ["opus", "sonnet"]},
    {"id": "codex", "models": []}
  ],
  "remembered": {"kind": "codex", "models": {"claude": "sonnet"}}
}`;

const catalog = JSON.parse(FRAME) as StartCatalog;
const korean = initializeInterfaceI18n("ko").getFixedT(null, "translation");
const english = initializeInterfaceI18n("en").getFixedT(null, "translation");

describe("selectionOf", () => {
  it("defaults to This Mac's Home with the remembered kind and its model", () => {
    const selection = selectionOf(catalog, NO_CHOICE);
    expect(selection.target?.id).toBe("home:local");
    expect(targetText(english, selection.target!, catalog.core_platform)).toBe("This Mac · Home");
    expect(targetText(korean, selection.target!, catalog.core_platform)).toBe("이 Mac · 홈");
    expect(selection.kind).toBe("codex");
    // Codex lists no model: the select is disabled on the default.
    expect(modelsOf(selection)).toEqual([]);
    expect(selection.model).toBe("");
  });

  it("names the core's machine as its system does, in the phone's language", () => {
    const home = catalog.targets[0]!;
    expect(targetText(english, home, "windows x86_64")).toBe("This PC · Home");
    expect(targetText(korean, home, "windows x86_64")).toBe("이 PC · 홈");
    expect(targetText(korean, home, "linux x86_64")).toBe("이 컴퓨터 · 홈");
    // A registered device keeps the name it was registered under.
    expect(targetText(korean, catalog.targets[2]!, "windows x86_64")).toBe("mini · 홈");
  });

  it("follows the operator's changes over the remembered choice", () => {
    const selection = selectionOf(catalog, { target: "c1", kind: "claude", model: undefined });
    expect(selection).toMatchObject({ kind: "claude", model: "sonnet" });
    expect(selection.target?.id).toBe("c1");
    expect(selectionOf(catalog, { target: null, kind: "claude", model: "" }).model).toBe("");
  });

  it("keeps a model the catalog no longer lists, and drops a target that left it", () => {
    expect(selectionOf(catalog, { target: "gone", kind: "claude", model: "haiku" })).toMatchObject({ model: "haiku" });
    expect(selectionOf(catalog, { target: "gone", kind: null, model: undefined }).target?.id).toBe("home:local");
  });

  it("shows the remembered model while the catalog is missing", () => {
    const remembered: StartCatalog = { targets: catalog.targets, core_platform: catalog.core_platform, kinds: [], remembered: { kind: "claude", models: { claude: "opus" } } };
    const selection = selectionOf(remembered, NO_CHOICE);
    expect(selection).toMatchObject({ kind: "claude", model: "opus", entry: null });
    expect(modelsOf(selection)).toEqual([]);
    expect(selectionOf(null, NO_CHOICE)).toMatchObject({ target: null, kind: "claude", model: "" });
  });
});

describe("startProblem", () => {
  it("matches what hided refuses, and lets a note run over lines", () => {
    expect(startProblem("  ")).toBe("empty");
    expect(startProblem("가".repeat(4001))).toBe("too_long");
    expect(startProblem("a\u001bb")).toBe("control_characters");
    expect(startProblem("테스트 고쳐줘\n그리고 PR")).toBeNull();
  });
});

describe("startFailure", () => {
  it("names the core's refusals and never shows a raw kind", () => {
    expect(noticeText(korean, startFailure("task_operation.busy"))).toContain("시작하는 중");
    expect(noticeText(korean, startFailure("home.conflict"))).toContain("~/hide");
    expect(noticeText(korean, startFailure("something.new"))).toBe("시작하지 못했어요. 다시 시도하세요.");
    expect(noticeText(korean, startFailure(null))).toBe("시작하지 못했어요. 다시 시도하세요.");
  });
});

describe("the store", () => {
  beforeEach(() => patch({ startCatalog: null, startSheet: CLOSED_SHEET }));

  it("keeps the catalog frame hided sends", () => {
    applyFrame(JSON.parse(FRAME) as ServerFrame);
    expect(usePhone.getState().startCatalog?.targets.map((target) => target.id)).toEqual(["home:local", "c1", "home:mini"]);
    expect(usePhone.getState().startCatalog?.remembered.kind).toBe("codex");
  });
});

describe("mayHaveStarted", () => {
  it("keeps the request id only while the outcome is unknown, so a tap after a refusal is a new start", () => {
    for (const reason of ["timeout", "in_flight", "offline"]) expect(mayHaveStarted(reason)).toBe(true);
    for (const reason of ["task_operation.busy", "home.conflict", "agent_failed", "start_failed", "unknown_target", null]) expect(mayHaveStarted(reason)).toBe(false);
  });
});
