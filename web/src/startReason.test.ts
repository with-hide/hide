import { describe, expect, it } from "vitest";
import { startReasonText } from "./startReason";
import { initializeInterfaceI18n } from "./i18n/instance";

const english = initializeInterfaceI18n("en").getFixedT(null, "translation");
const korean = initializeInterfaceI18n("ko").getFixedT(null, "translation");

describe("a launch reason stored by an earlier build", () => {
  it("renders the engine refusal text for the old code, never the raw code", () => {
    const sentence = english("coreError.agent.refusedByEngine");
    expect(startReasonText("agent_start.refused_by_herdr", english)).toBe(sentence);
    expect(startReasonText("agent_start.refused_by_herdr", english)).not.toContain("refused_by_herdr");
    expect(startReasonText("agent_start.refused_by_herdr", korean)).toBe(korean("coreError.agent.refusedByEngine"));
    expect(startReasonText("agent_start.refused_by_engine", english)).toBe(sentence);
  });
});
