import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { windowStrip } from "./connection";
import { stepMarks, type MoveView } from "./coreMove";
import { createInterfaceI18n } from "./i18n/instance";
import { moveForm } from "./settings/CoreMoveDialog";
import { coreMachineName, moveMachines } from "./screenMachine";
import type { DeviceProblem } from "./snapshot";

const view = (over: Partial<MoveView>): MoveView => ({ state: "idle", direction: "forward", device: "mini", intent: null, sent: 0, total: 0, failed: [], step: null, cause: null, node: null, ...over });

describe("the move's steps as the dialog marks them (B3, B4)", () => {
  it("marks the steps before the current one done, the current running and the rest to do", () => {
    expect(stepMarks(view({ state: "copying", step: "copy" })).map((row) => row.mark)).toEqual(["done", "done", "run", "todo", "todo"]);
  });

  it("marks the step that failed when the move is undone", () => {
    expect(stepMarks(view({ state: "rolled_back", step: "start_target" })).map((row) => row.mark)).toEqual(["done", "done", "done", "fail", "todo"]);
  });
});

describe("which form the dialog shows", () => {
  const request = { direction: "forward" as const, device: "mini" };

  it("waits on its checks until a frame newer than the one it asked beside answers, and for a frame of another move", () => {
    const before = view({ state: "rolled_back", step: "copy" });
    expect(moveForm(before, before, request)).toBe("checking");
    expect(moveForm(view({ state: "ready", device: "box" }), before, request)).toBe("checking");
    expect(moveForm(view({ state: "ready", direction: "back" }), before, request)).toBe("checking");
  });

  it("follows the supervisor's state once it answers", () => {
    const before = view({});
    expect(moveForm(view({ state: "checks_failed" }), before, request)).toBe("checks_failed");
    expect(moveForm(view({ state: "ready" }), before, request)).toBe("confirm");
    expect(moveForm(view({ state: "waiting" }), before, request)).toBe("moving");
    expect(moveForm(view({ state: "rolled_back" }), before, request)).toBe("failed");
    expect(moveForm(view({ state: "done" }), before, request)).toBe("done");
    expect(moveForm(view({ state: "abandoned" }), before, request)).toBe("abandoned");
  });
});

describe("the strip above the window (W1 to W3)", () => {
  // This window runs on a Mac, whatever system runs the test (#972 names it by its system).
  beforeEach(() => vi.stubGlobal("navigator", { platform: "MacIntel", userAgent: "" }));
  afterEach(() => vi.unstubAllGlobals());
  const machines = { from: "This Mac", to: "Mac mini" };

  it("offers to give up the move's wait only while the move offers it, and acts on nothing itself (amendment 4)", async () => {
    const { t } = await createInterfaceI18n("en");
    const strip = (move: MoveView) => windowStrip({ connection: "moving", refused: false, move, link: null, machines, coreMachine: "This Mac" }, t);
    expect(strip(view({ state: "waiting", step: "reattach", abandon: true }))?.action).toBe("give_up");
    expect(strip(view({ state: "waiting", step: "reattach" }))?.action).toBeUndefined();
    expect(strip(view({ state: "rolling_back", step: "reattach" }))?.action).toBeUndefined();
  });

  // A window reloaded during a long wait gets the move's frame before any
  // snapshot, so it has no device rows to name the other machine from.
  it("names the other machine by the move's alias in a window the move holds before it drew the core", async () => {
    const { t } = await createInterfaceI18n("ko");
    const firstFrame = (move: MoveView) => windowStrip({ connection: "moving", refused: false, move, link: null, machines: moveMachines(undefined, move, t), coreMachine: coreMachineName(undefined, t) }, t);
    const forward = firstFrame(view({ state: "waiting", step: "reattach", device: "mini-2", alias: "mini", abandon: true }));
    expect(forward?.text).toContain("core를 mini 기기로 옮기는 중");
    expect(forward?.text).not.toContain("mini-2");
    // A move back's strip names this Mac only; its take-here dialog names the core's machine.
    expect(moveMachines(undefined, view({ state: "waiting", direction: "back", device: "mini-2", alias: "mini", abandon: true }), t)).toEqual({ from: "mini", to: "이 Mac" });
  });

  it("names the move's step while a move holds the window", async () => {
    const { t } = await createInterfaceI18n("ko");
    const strip = windowStrip({ connection: "moving", refused: false, move: view({ state: "copying", step: "copy" }), link: null, machines: { from: "이 Mac", to: "Mac mini" }, coreMachine: "이 Mac" }, t);
    expect(strip).toEqual({ kind: "moving", mark: "pending", text: "core를 Mac mini 기기로 옮기는 중 · 3/5 프로젝트와 기록 복사 · 다시 연결될 때까지 입력을 받지 않아요" });
    // A move back goes to this machine, whose name already says what it is: no 기기 after it.
    const back = windowStrip({ connection: "moving", refused: false, move: view({ state: "copying", step: "copy", direction: "back" }), link: null, machines: { from: "Mac mini", to: "이 Mac" }, coreMachine: "Mac mini" }, t);
    expect(back?.text).toBe("core를 이 Mac 쪽으로 되돌리는 중 · 3/5 프로젝트와 기록 복사 · 다시 연결될 때까지 입력을 받지 않아요");
  });

  it("names the node's update of its core and its failed link while the screen waits, and nothing once live", async () => {
    const { t } = await createInterfaceI18n("ko");
    const base = { refused: false, move: null, machines, coreMachine: "Mac mini" };
    expect(windowStrip({ ...base, connection: "connecting", link: { phase: "updating", machine: null, alias: "mini" } }, t)?.text).toBe("Mac mini의 core를 이 앱의 빌드로 바꾸는 중 · 에이전트는 계속 실행돼요");
    expect(windowStrip({ ...base, connection: "reconnecting", link: { phase: "waiting", machine: "Mac mini", alias: "mini" } }, t)).toEqual({ kind: "unreachable", mark: "warn", text: "Mac mini의 core에 연결할 수 없어요 · 다시 연결하는 중" });
    expect(windowStrip({ ...base, connection: "live", link: { phase: "waiting", machine: "Mac mini", alias: "mini" } }, t)).toBeNull();
    // The operator ended the link (B16): the strip offers it back.
    expect(windowStrip({ ...base, connection: "connecting", link: { phase: "disconnected", machine: null, alias: "mini" } }, t)).toEqual({ kind: "disconnected", mark: "warn", text: "Mac mini core와 연결을 끊었어요", action: "reconnect" });
    expect(windowStrip({ ...base, connection: "reconnecting", link: { phase: "connecting", machine: null, alias: "mini" } }, t)?.kind).toBe("reconnecting");
    // The core's hello names its machine by its host name; a window that drew the core names it as its rows do.
    expect(windowStrip({ ...base, connection: "reconnecting", link: { phase: "waiting", machine: "Mac-mini.local", alias: "mini-ssh" } }, t)?.text).toBe("Mac mini의 core에 연결할 수 없어요 · 다시 연결하는 중");
    expect(windowStrip({ ...base, coreMachine: "", connection: "reconnecting", link: { phase: "waiting", machine: "Mac-mini.local", alias: "mini-ssh" } }, t)?.text).toBe("Mac-mini.local의 core에 연결할 수 없어요 · 다시 연결하는 중");
    // A window opened while its link is down has drawn no core: it names the machine by the alias it is dialed by.
    const fresh = { ...base, coreMachine: "" };
    expect(windowStrip({ ...fresh, connection: "reconnecting", link: { phase: "waiting", machine: null, alias: "mini" } }, t)?.text).toBe("mini의 core에 연결할 수 없어요 · 다시 연결하는 중");
    expect(windowStrip({ ...fresh, connection: "connecting", link: { phase: "disconnected", machine: null, alias: "mini" } }, t)?.text).toBe("mini core와 연결을 끊었어요");
  });

  it("offers the device row's three actions when the link stopped on what only the operator can fix", async () => {
    const { t } = await createInterfaceI18n("ko");
    const base = { refused: false, move: null, machines, coreMachine: "Mac mini", connection: "reconnecting" as const };
    const stopped = (problem: DeviceProblem) => windowStrip({ ...base, link: { phase: "stopped", machine: null, alias: "mini", problem } }, t);
    // A sign-in: its Retry reads Sign in and starts the dial that may ask.
    expect(stopped("sign_in_needed")).toEqual({ kind: "stopped", mark: "warn", text: "Mac mini core에 연결하려면 로그인이 필요해요", action: "sign_in" });
    expect(stopped("sign_in_refused")).toMatchObject({ text: "Mac mini core가 로그인을 거절했어요", action: "sign_in" });
    // A changed server key: the terminal command that checks it, and no Retry (#1840's decision for the device row).
    expect(stopped("host_key_changed")).toEqual({ kind: "stopped", mark: "warn", text: "Mac mini core의 서버 키가 바뀌었어요 · 터미널에서 확인해 주세요", command: "ssh mini" });
    // Anything else is a retry.
    expect(stopped("cannot_connect")).toMatchObject({ text: "ssh로 Mac mini core에 연결하지 못했어요 · 터미널에서 ssh mini 명령으로 이유를 확인해 주세요", action: "retry" });
    expect(stopped("cannot_connect")?.command).toBeUndefined();
  });

  it("shows what ssh asks the operator's own dial under the strip, whatever the phase", async () => {
    const { t } = await createInterfaceI18n("ko");
    const base = { refused: false, move: null, machines, coreMachine: "Mac mini", connection: "reconnecting" as const };
    const sign_in = { question: 3, kind: "host_key" as const, host: "[mini]:22", fingerprint: "SHA256:abc", key_command: null };
    expect(windowStrip({ ...base, link: { phase: "connecting", machine: null, alias: "mini", sign_in } }, t)).toEqual({ kind: "sign_in", mark: "pending", text: "Mac mini core에 로그인하는 중", signIn: sign_in });
    expect(windowStrip({ ...base, link: { phase: "updating", machine: "Mac mini", alias: "mini", sign_in } }, t)?.signIn).toBe(sign_in);
    // A live link draws the core; nothing of the strip stays.
    expect(windowStrip({ ...base, connection: "live", link: { phase: "connecting", machine: null, alias: "mini", sign_in } }, t)).toBeNull();
  });
});
