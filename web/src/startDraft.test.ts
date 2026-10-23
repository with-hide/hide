import { beforeEach, describe, expect, it } from "vitest";
import { agentFailure, endedSentence, startFailure, startRefusal, useStartPanel } from "./startDraft";

describe("the start panel's draft", () => {
  beforeEach(() => useStartPanel.setState({ isOpen: false, text: "", target: null, overSettings: false, request: null, failure: null, sent: "", spent: null }));

  it("is spent by a start and comes back with the reason when the agent then fails to start", () => {
    const panel = useStartPanel.getState();
    panel.setText("테스트 고쳐줘");
    panel.begin("start-1");
    panel.finish(7);
    expect(useStartPanel.getState().text).toBe("");
    useStartPanel.getState().restore({ message: "claude: command not found" });
    expect(useStartPanel.getState()).toMatchObject({ text: "테스트 고쳐줘", failure: { message: "claude: command not found" }, spent: null });
    // The next open shows both, and editing the text clears the reason.
    useStartPanel.getState().open(false);
    expect(useStartPanel.getState().failure).toEqual({ message: "claude: command not found" });
    useStartPanel.getState().setText("테스트 고쳐줘 다시");
    expect(useStartPanel.getState().failure).toBeNull();
  });

  it("never overwrites a new draft with the failed start's text", () => {
    const panel = useStartPanel.getState();
    panel.setText("첫 지시");
    panel.begin("start-1");
    panel.finish(7);
    useStartPanel.getState().setText("다음 지시");
    useStartPanel.getState().restore({ message: "failed" });
    expect(useStartPanel.getState().text).toBe("다음 지시");
  });

  it("forgets the text once the agent started", () => {
    const panel = useStartPanel.getState();
    panel.setText("첫 지시");
    panel.begin("start-1");
    panel.finish(7);
    useStartPanel.getState().settle();
    useStartPanel.getState().restore({ message: "late" });
    expect(useStartPanel.getState()).toMatchObject({ text: "", failure: null });
  });

  it("words a code this build does not know, or a missing one, as the plain did-not-start", () => {
    expect(startFailure("task_operation.failed")).toEqual({ key: "workspace.agentNotStarted", code: "task_operation.failed" });
    expect(startFailure(null)).toEqual({ key: "workspace.agentNotStarted" });
  });

  it("words a Codex the machine's kit has not read in the operator's language", () => {
    expect(startFailure("codex_capability_unknown")).toEqual({ key: "agents.codexUnread", code: "codex_capability_unknown" });
    expect(startFailure("session_checkpoint_invalid:cursor_launch_root_changed")).toEqual({ key: "coreError.startReason.cursor_launch_root_changed", code: "cursor_launch_root_changed" });
    expect(startRefusal("home.conflict")).toEqual({ key: "coreError.home.conflict", code: "home.conflict" });
    expect(startRefusal("agent_start.unknown_provider")).toEqual({ key: "workspace.agentNotStarted", code: "agent_start.unknown_provider" });
    expect(startFailure("constructor")).toEqual({ key: "workspace.agentNotStarted", code: "constructor" });
  });

  it("words why a task's agent did not start by its code, quoting the shell's last line beside it", () => {
    expect(agentFailure("agent_start.shell_starting", "Trust this folder? [y/N]")).toEqual({ key: "coreError.agent.shellStarting", code: "agent_start.shell_starting", line: "Trust this folder? [y/N]" });
    expect(agentFailure("agent_start.not_installed", null)).toEqual({ key: "coreError.agent.notInstalled", code: "agent_start.not_installed" });
    expect(agentFailure("session_route_reader_unavailable", null)).toEqual({ key: "coreError.startReason.session_route_unconfirmed", code: "session_route_reader_unavailable" });
    // A code from a daemon of another build is never drawn as text.
    expect(agentFailure("agent_start.from_the_future", null)).toEqual({ key: "workspace.agentNotStarted", code: "agent_start.from_the_future" });
    expect(agentFailure(null, null)).toEqual({ key: "workspace.agentNotStarted" });
  });
});

describe("a reason followed by another sentence", () => {
  it("ends with one full stop, whether or not the core sent it", () => {
    expect(endedSentence("agent_trust_unavailable: the machine cannot be reached")).toBe("agent_trust_unavailable: the machine cannot be reached.");
    expect(endedSentence("It could not start. ")).toBe("It could not start.");
    expect(endedSentence("에이전트를 시작하지 못했어요.")).toBe("에이전트를 시작하지 못했어요.");
  });
});
