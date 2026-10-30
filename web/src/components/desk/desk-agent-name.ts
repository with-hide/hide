import type { DeskAgentId } from "../../desk";
import type { useInterfaceTranslation } from "../../i18n/client";
import type { MessageKey } from "../../i18n/catalogs";

const NAME: Record<DeskAgentId, MessageKey> = {
  claude: "desk.agentName.claude",
  codex: "desk.agentName.codex",
  pi: "desk.agentName.pi",
};

/** A Desk agent's name, from its wire id. */
export function deskAgentName(t: ReturnType<typeof useInterfaceTranslation>["t"], id: DeskAgentId): string {
  return t(NAME[id]);
}
