import { describe, expect, it } from "vitest";
import { cardOwnerPane } from "./cardTitle";

type State = Parameters<typeof cardOwnerPane>[0];

const agent = (pane_id: string, session_id: string) => ({ pane_id, session_id, device_id: "mac" });

// The card's owner is the core's (`detail_owner_pane`, PRD lane-desk-pair-view
// D-30): the agent a card pressed where no agent is opens beside, so its logo
// and that tab never name two agents.
describe("cardOwnerPane", () => {
  const state = {
    agents: [agent("t1:p", "made"), agent("t2:p", "lead"), agent("t3:p", "conducts")],
    rest: {
      navigator: {
        workspaces: [{
          id: "w", device_id: "mac",
          // The checkout's representative is t2, but t1 made the pull request.
          checkouts: [{ pull_request: { number: 7 }, agent_summary: { representative_pane_id: "t2:p" } }],
          agent_scope: { prs: { rows: [{ number: 7, agents: [{ pane_id: "t1:p", occurrence: 0 }] }, { number: 8, agents: [{ pane_id: "gone", occurrence: 0 }] }] } },
        }],
      },
      missions: { items: [{ id: "m", device_id: "mac", conductor: { pane_id: "t3:p", present: true } }] },
    },
  } as unknown as State;

  it("names a pull request's maker, a mission's Conductor and a session's pane, and none without a pane here", () => {
    expect([
      cardOwnerPane(state, { kind: "pull_request", workspace_id: "w", number: 7 }),
      cardOwnerPane(state, { kind: "pull_request", workspace_id: "w", number: 8 }),
      cardOwnerPane(state, { kind: "mission", mission: "m" }),
      cardOwnerPane(state, { kind: "record", session: { device_id: "mac", agent: "claude", session_id: "lead" } }),
      cardOwnerPane(state, { kind: "record", session: { device_id: "phone", agent: "claude", session_id: "lead" } }),
    ]).toEqual(["t1:p", null, "t3:p", "t2:p", null]);
  });
});
