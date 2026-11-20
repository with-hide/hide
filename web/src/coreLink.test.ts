import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { windowStrip } from "./connection";
import { createInterfaceI18n } from "./i18n/instance";
import type { DeviceProblem } from "./snapshot";

describe("the strip above a node's window while it waits for its core (B16)", () => {
  // This window runs on a Mac, whatever system runs the test (#972 names it by its system).
  beforeEach(() => vi.stubGlobal("navigator", { platform: "MacIntel", userAgent: "" }));
  afterEach(() => vi.unstubAllGlobals());

  it("names the node's update of its core and its failed link while the screen waits, and nothing once live", async () => {
    const { t } = await createInterfaceI18n("ko");
    const base = { refused: false, coreMachine: "Mac mini" };
    expect(windowStrip({ ...base, connection: "connecting", link: { phase: "updating", machine: null, alias: "mini" } }, t)?.text).toBe("Mac mini의 core를 이 앱의 빌드로 바꾸는 중 · 에이전트는 계속 실행돼요");
    // A build that offers connections between two Hides splits this machine from its core once it answers; any other only reconnects.
    expect(windowStrip({ ...base, connection: "reconnecting", link: { phase: "waiting", machine: "Mac mini", alias: "mini", splits: true } }, t)).toEqual({ kind: "unreachable", mark: "warn", text: "Mac mini의 core에 연결할 수 없어요 · 이전 대기 중" });
    expect(windowStrip({ ...base, connection: "reconnecting", link: { phase: "waiting", machine: "Mac mini", alias: "mini", splits: false } }, t)?.text).toBe("Mac mini의 core에 연결할 수 없어요 · 다시 연결하는 중");
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
    const base = { refused: false, coreMachine: "Mac mini", connection: "reconnecting" as const };
    const stopped = (problem: DeviceProblem) => windowStrip({ ...base, link: { phase: "stopped", machine: null, alias: "mini", problem } }, t);
    // A sign-in: its Retry reads Sign in and starts the dial that may ask.
    expect(stopped("sign_in_needed")).toEqual({ kind: "stopped", mark: "warn", text: "Mac mini core에 연결하려면 로그인이 필요해요", action: "sign_in" });
    expect(stopped("sign_in_refused")).toMatchObject({ text: "Mac mini core가 로그인을 거절했어요", action: "sign_in" });
    // A changed server key: the terminal command that checks it, and no Retry (#1840's decision for the device row).
    expect(stopped("host_key_changed")).toEqual({ kind: "stopped", mark: "warn", text: "Mac mini core의 서버 키가 바뀌었어요 · 터미널에서 확인해 주세요", command: "ssh mini" });
    // A Hide an update replaced: only opening Hide again mends it, so neither a Retry nor a command.
    expect(stopped("hide_updated")).toEqual({ kind: "stopped", mark: "warn", text: "Hide가 실행된 뒤에 업데이트됐어요 · Hide를 종료하고 다시 열면 Mac mini core에 연결돼요" });
    // Anything else is a retry.
    expect(stopped("cannot_connect")).toMatchObject({ text: "ssh로 Mac mini core에 연결하지 못했어요 · 터미널에서 ssh mini 명령으로 이유를 확인해 주세요", action: "retry" });
    expect(stopped("cannot_connect")?.command).toBeUndefined();
  });

  it("shows what ssh asks the operator's own dial under the strip, whatever the phase", async () => {
    const { t } = await createInterfaceI18n("ko");
    const base = { refused: false, coreMachine: "Mac mini", connection: "reconnecting" as const };
    const sign_in = { question: 3, kind: "host_key" as const, host: "[mini]:22", fingerprint: "SHA256:abc", key_command: null };
    expect(windowStrip({ ...base, link: { phase: "connecting", machine: null, alias: "mini", sign_in } }, t)).toEqual({ kind: "sign_in", mark: "pending", text: "Mac mini core에 로그인하는 중", signIn: sign_in });
    expect(windowStrip({ ...base, link: { phase: "updating", machine: "Mac mini", alias: "mini", sign_in } }, t)?.signIn).toBe(sign_in);
    // A live link draws the core; nothing of the strip stays.
    expect(windowStrip({ ...base, connection: "live", link: { phase: "connecting", machine: null, alias: "mini", sign_in } }, t)).toBeNull();
  });
});
