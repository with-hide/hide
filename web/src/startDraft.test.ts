import { beforeEach, describe, expect, it } from "vitest";
import { endedSentence, startFailure, useStartPanel } from "./startDraft";

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

  it("keeps the core's own reason as sent and words a missing one by key", () => {
    expect(startFailure("claude: command not found")).toEqual({ message: "claude: command not found" });
    expect(startFailure(null)).toEqual({ key: "workspace.agentNotStarted" });
  });

  it("words a Codex the machine's kit has not read in the operator's language", () => {
    expect(startFailure("codex_capability_unknown")).toEqual({ key: "agents.codexUnread" });
    expect(startFailure("constructor")).toEqual({ message: "constructor" });
  });
});

describe("a reason followed by another sentence", () => {
  it("ends with one full stop, whether or not the core sent it", () => {
    expect(endedSentence("agent_trust_unavailable: the machine cannot be reached")).toBe("agent_trust_unavailable: the machine cannot be reached.");
    expect(endedSentence("It could not start. ")).toBe("It could not start.");
    expect(endedSentence("에이전트를 시작하지 못했어요.")).toBe("에이전트를 시작하지 못했어요.");
  });
});
