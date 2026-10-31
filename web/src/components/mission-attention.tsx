import { useState } from "react";
import type { Actions } from "../actions";
import { useInterfaceTranslation } from "../i18n/client";
import { attentionChoices } from "../missions";
import type { Mission, MissionAttention, MissionNode } from "../snapshot";
import { Button } from "./ui/button";

// The operator's choices for a node that waits on someone to look (PRD
// mission-node-simple B15, B16, B20, B26), the same in 내 차례 and the node's
// panel: each button says what it does, sends one `mission_attention` event,
// and the core settles the node and tells the Conductor (B17). Once one is
// pressed every choice is off, so a second press sends nothing; the item
// goes when the core's next snapshot settles it. `터미널로 이동` brings the
// worker's pane forward and settles nothing.

export function AttentionChoices({ mission, node, attention, actions }: { mission: Mission; node: MissionNode; attention: MissionAttention; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const [sent, setSent] = useState(false);
  return (
    <span className="flex shrink-0 flex-wrap items-center gap-xs" data-attention-choices={attention.reason}>
      {attentionChoices(attention.reason).map((choice) =>
        choice === "terminal" ? (
          node.pane_id ? (
            <Button key={choice} size="sm" variant="secondary" onClick={() => actions.openAgent(node.pane_id!)} data-attention-choice="terminal">
              {t("mission.toTerminal")}
            </Button>
          ) : null
        ) : (
          <Button
            key={choice}
            size="sm"
            variant="secondary"
            disabled={sent}
            onClick={() => {
              setSent(true);
              actions.settleMissionNode(mission.id, node.key, choice, attention.since_unix_ms);
            }}
            data-attention-choice={choice}
          >
            {t(`mission.operator.${choice}`)}
          </Button>
        ),
      )}
    </span>
  );
}
