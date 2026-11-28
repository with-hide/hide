// A single agent and its conversation, answered through the same thread events as the core.
import type { PracticeWorld, WorldSnapshot } from "../world";
import type { SessionThreadSection } from "../../snapshot";
import { agent, applyDesk, deskSnapshot, oneTabDesk, shellScreen, type DeskModel } from "./kit";

export const CONVERSATION_PANE = "conversation-agent";
const PLACE = { project: "hide-practice", label: "hide-practice", branch: "main" };
export type ConversationModel = DeskModel & { opened: boolean; generation: number; sent: string | null; request: string | null };
export const conversationThread = (model: ConversationModel): SessionThreadSection => ({
  generation: model.generation, pane_id: model.opened ? CONVERSATION_PANE : null, machine: "practice", state: model.opened ? "ready" : "closed", failure: null,
  pane: "same", connected: true, has_older: false, older: "idle", choice: {}, queue: null, input: model.request ? { request_id: model.request, state: "sent" } : null,
  tail: { start: 0, updates: [
    { sessionUpdate: "user_message_chunk", content: { type: "text", text: "Read the project and list the next steps." } },
    { sessionUpdate: "agent_message_chunk", content: { type: "text", text: "I read the project. The next step is to check the sign-in flow." } },
    ...(model.sent ? [{ sessionUpdate: "user_message_chunk", content: { type: "text", text: model.sent } }, { sessionUpdate: "agent_message_chunk", content: { type: "text", text: "I will check that next." } }] : []),
  ], parts: {}, sessions: {} },
});
function initial(): ConversationModel {
  return { ...oneTabDesk({ id: CONVERSATION_PANE, title: "Check the sign-in flow", agent: agent({ pane_id: CONVERSATION_PANE, identity_label: "Check the sign-in flow", agent_kind: "claude", group: "done", mark: "done", status_code: "done", elapsed: "1m", thread_capability: "ready", thread_identity: "practice-conversation", session_id: "practice-conversation" }) }), opened: false, generation: 0, sent: null, request: null };
}
function snapshot(model: ConversationModel, now: number): WorldSnapshot {
  const drawn = deskSnapshot([{ place: PLACE, model }], PLACE.project, now);
  return { ...drawn, frames: { sessionThread: conversationThread(model), sessionThreadHistory: { generation: model.generation, pages: [] }, threadInputResult: model.request ? { request_id: model.request, state: "sent" } : null } };
}
export const CONVERSATION_WORLD: PracticeWorld<ConversationModel> = {
  initial, snapshot,
  apply: (model, event) => {
    if (event.kind === "session_thread_open" && event.payload.pane_id === CONVERSATION_PANE) return { ...model, opened: true, generation: model.generation + 1 };
    if (event.kind === "session_thread_close") return { ...model, opened: false };
    if (event.kind === "thread_input" && event.payload.pane_id === CONVERSATION_PANE && typeof event.payload.text === "string") return { ...model, sent: event.payload.text, request: String(event.payload.request_id) };
    const desk = applyDesk(model, event);
    return desk ? { ...model, ...desk } : null;
  },
  panes: (model) => model.panes.map((pane) => pane.id),
  screen: () => `${shellScreen(PLACE)}\r\n> Read the project and list the next steps.\r\nI read the project. The next step is to check the sign-in flow.\r\n`,
};
