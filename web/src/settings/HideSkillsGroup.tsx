import { BookOpenTextIcon, ListChecksIcon, WaypointsIcon } from "lucide-react";
import { Group, Row } from "../components/settings-rows";
import { useInterfaceTranslation } from "../i18n/client";

/**
 * The skills Hide installs for every agent that is on (PRD hide-plan B20,
 * D-13, D-16): each with its mark, one line on what it is for and the help it
 * points at. They come and go with each agent's switch above; the list is the
 * kit's own (`hide_kit::agents::SKILLS`).
 */
const SKILLS = [
  { name: "hide", icon: BookOpenTextIcon, help: "hide guide", what: "agents.skills.hide" },
  { name: "hide-conductor", icon: WaypointsIcon, help: "hide mission help", what: "agents.skills.conductor" },
  { name: "hide-plan", icon: ListChecksIcon, help: "hide plan help", what: "agents.skills.plan" },
] as const;

export function HideSkillsGroup() {
  const { t } = useInterfaceTranslation();
  return (
    <Group title={t("agents.skills.title")} data-agents-skills="true">
      {SKILLS.map((skill) => (
        <Row
          key={skill.name}
          data-agents-skill={skill.name}
          label={
            <span className="flex min-w-0 flex-wrap items-center gap-x-sm gap-y-xxs">
              <skill.icon aria-hidden="true" className="size-(--size-icon) shrink-0 text-muted-foreground" />
              <span className="font-mono font-semibold">{skill.name}</span>
              <span className="min-w-0 text-body text-muted-foreground">
                {t(skill.what)} · <span className="font-mono">{skill.help}</span>
              </span>
            </span>
          }
        />
      ))}
    </Group>
  );
}
