import { beforeEach, describe, expect, it } from "vitest";
import { initializeInterfaceI18n } from "../i18n/instance";
import { noticeText, type ServerFrame, type StartCatalog } from "./protocol";
import { NO_CHOICE, mayHaveStarted, modelsOf, selectionOf, startDevices, startFailure, startProblem } from "./start";
import { CLOSED_SHEET, applyFrame, patch, usePhone } from "./store";

/** The frame hided/src/mobile/start.rs sends, as its own test builds it, with a main checkout listed after another of its project. */
const FRAME = `{
  "type": "start_catalog",
  "targets": [
    {"id": "c2", "device_id": "local", "place": "hide · prd/a", "project": "hide", "project_id": "w1", "branch": "prd/a", "main": false, "device_label": null, "connected": true},
    {"id": "c1", "device_id": "local", "place": "hide · main", "project": "hide", "project_id": "w1", "branch": "main", "main": true, "device_label": null, "connected": true},
    {"id": "c4", "device_id": "local", "place": "notes", "project": "notes", "project_id": "w2", "branch": null, "main": true, "device_label": null, "connected": true},
    {"id": "rc1", "device_id": "mini", "place": "contong · main", "project": "contong", "project_id": "rw1", "branch": "main", "main": true, "device_label": "mini", "connected": false}
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

describe("startDevices", () => {
  it("lists each device's projects once, this Mac first, each with its main checkout first", () => {
    const devices = startDevices(catalog);
    expect(devices.map((device) => [device.deviceId, device.label, device.connected])).toEqual([["local", null, true], ["mini", "mini", false]]);
    expect(devices[0]!.projects.map((project) => [project.label, project.targets.map((target) => target.id)])).toEqual([["hide", ["c1", "c2"]], ["notes", ["c4"]]]);
    expect(devices[1]!.projects.map((project) => project.label)).toEqual(["contong"]);
  });

  it("keeps two projects of one name apart, and one id on two devices apart", () => {
    const twins: StartCatalog = { ...catalog, targets: [
      { id: "a", device_id: "local", place: "app · main", project: "app", project_id: "w1", branch: "main", main: true, device_label: null, connected: true },
      { id: "b", device_id: "local", place: "app · main", project: "app", project_id: "w2", branch: "main", main: true, device_label: null, connected: true },
      { id: "c", device_id: "mini", place: "app · main", project: "app", project_id: "w1", branch: "main", main: true, device_label: "mini", connected: true },
    ] };
    expect(startDevices(twins).map((device) => device.projects.length)).toEqual([2, 1]);
  });

  it("reads an older Mac's targets, which name no project, as a project each named by its place", () => {
    const older: StartCatalog = { ...catalog, targets: [
      { id: "c1", device_id: "local", place: "hide · main", device_label: null, connected: true },
      { id: "c2", device_id: "local", place: "hide · prd/a", device_label: null, connected: true },
    ] };
    const projects = startDevices(older)[0]!.projects;
    expect(projects.map((project) => [project.label, project.targets.map((target) => target.id)])).toEqual([["hide · main", ["c1"]], ["hide · prd/a", ["c2"]]]);
    expect(selectionOf(older, { ...NO_CHOICE, target: "c2" }).target?.branch).toBeUndefined();
  });

  it("lists nothing before the catalog arrives", () => {
    expect(startDevices(null)).toEqual([]);
  });
});

describe("selectionOf", () => {
  it("defaults to the first project's main checkout with the remembered kind and its model", () => {
    const selection = selectionOf(catalog, NO_CHOICE);
    expect(selection.target?.id).toBe("c1");
    expect(selection.project?.label).toBe("hide");
    expect(selection.kind).toBe("codex");
    // Codex lists no model: the model word is disabled on the default.
    expect(modelsOf(selection)).toEqual([]);
    expect(selection.model).toBe("");
  });

  it("follows a chosen checkout to its project", () => {
    expect(selectionOf(catalog, { ...NO_CHOICE, target: "c2" })).toMatchObject({ target: { id: "c2", branch: "prd/a" }, project: { label: "hide" } });
    expect(selectionOf(catalog, { ...NO_CHOICE, target: "rc1" }).project?.label).toBe("contong");
  });

  it("sends no model for an agent whose start takes none, even one remembered for it", () => {
    const opencode: StartCatalog = { ...catalog, remembered: { kind: "opencode", models: { opencode: "gpt-5" } } };
    expect(selectionOf(opencode, NO_CHOICE)).toMatchObject({ kind: "opencode", model: "" });
  });

  it("follows the operator's changes over the remembered choice", () => {
    const selection = selectionOf(catalog, { target: "c1", kind: "claude", model: undefined });
    expect(selection).toMatchObject({ kind: "claude", model: "sonnet" });
    expect(selection.target?.id).toBe("c1");
    expect(selectionOf(catalog, { target: null, kind: "claude", model: "" }).model).toBe("");
  });

  it("keeps a model the catalog no longer lists, and drops a target that left it", () => {
    expect(selectionOf(catalog, { target: "gone", kind: "claude", model: "haiku" })).toMatchObject({ model: "haiku" });
    expect(selectionOf(catalog, { target: "gone", kind: null, model: undefined }).target?.id).toBe("c1");
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
    expect(noticeText(korean, startFailure("something.new"))).toBe("시작하지 못했어요. 다시 시도하세요.");
    expect(noticeText(korean, startFailure(null))).toBe("시작하지 못했어요. 다시 시도하세요.");
  });
});

describe("the store", () => {
  beforeEach(() => patch({ startCatalog: null, startSheet: CLOSED_SHEET }));

  it("keeps the catalog frame hided sends", () => {
    applyFrame(JSON.parse(FRAME) as ServerFrame);
    expect(usePhone.getState().startCatalog?.targets.map((target) => target.id)).toEqual(["c2", "c1", "c4", "rc1"]);
    expect(usePhone.getState().startCatalog?.remembered.kind).toBe("codex");
  });
});

describe("mayHaveStarted", () => {
  it("keeps the request id only while the outcome is unknown, so a tap after a refusal is a new start", () => {
    for (const reason of ["timeout", "in_flight", "offline"]) expect(mayHaveStarted(reason)).toBe(true);
    for (const reason of ["task_operation.busy", "agent_failed", "start_failed", "unknown_target", null]) expect(mayHaveStarted(reason)).toBe(false);
  });
});
