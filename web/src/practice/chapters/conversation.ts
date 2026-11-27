import { chapter } from "../chapter";
import { CONVERSATION_PANE, CONVERSATION_WORLD, type ConversationModel } from "../worlds/conversation";

export default chapter<ConversationModel>({
  id: "conversation", number: 13, version: 1, since: "1.2.13", world: CONVERSATION_WORLD, quest: true, preview: "switch",
  steps: [
    { id: "open", quest: true, target: `[data-pane-conversation-toggle="${CONVERSATION_PANE}"]`, side: "bottom", command: "desk_terminal", gesture: { click: true }, done: (model) => model.opened },
    { id: "read", target: "[data-pane-conversation]", side: "inside" },
    { id: "input", target: "[data-desk-composer-input]", side: "inside" },
    { id: "back", quest: true, target: `[data-pane-conversation-toggle="${CONVERSATION_PANE}"]`, side: "bottom", command: "desk_terminal", gesture: { click: true }, done: (model) => !model.opened },
  ],
});
