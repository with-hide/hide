import type { DeskAgent, DeskAgentId, DeskLocation, DeskSettings } from "../../desk";
import { agentPickable } from "../../desk";
import { AgentMark } from "../../AgentMark";
import { deskAgentName } from "./desk-agent-name";
import { useInterfaceTranslation } from "../../i18n/client";
import { Group, Row } from "../settings-rows";
import { RadioGroup, RadioGroupItem } from "../ui/radio-group";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../ui/select";
import { Switch } from "../ui/switch";

const LOCATIONS = ["desk", "worktree", "checkout"] as const;
/** The cap's choices (D-23): 8 by default. */
const CAP_CHOICES = [1, 2, 4, 6, 8, 12, 16];

/**
 * Settings › Desk (D-20, D-23, D-76, B4, B19, B21, B41): the switch that
 * shows Desk, off by default, then the agent Desk starts with, where its
 * tasks run (Desk decides, by default) and how many run at once. An agent
 * that cannot ask before acting is not offered.
 */
export function DeskSettingsTab({ settings, agents, onChange }: { settings: DeskSettings; agents: readonly DeskAgent[]; onChange: (settings: DeskSettings) => void }) {
  const { t } = useInterfaceTranslation();
  return (
    <div className="flex min-w-0 flex-col" data-desk-settings="true" data-desk-enabled={String(settings.enabled)}>
      <Group data-settings-group="desk-use">
        <Row
          label={
            <span className="flex min-w-0 flex-col gap-xxs">
              <span>{t("desk.settings.use")}</span>
              <span className="text-body text-muted-foreground">{t("desk.settings.useDescription")}</span>
            </span>
          }
        >
          <Switch checked={settings.enabled} onCheckedChange={(enabled) => onChange({ ...settings, enabled })} aria-label={t("desk.settings.use")} data-desk-use={String(settings.enabled)} />
        </Row>
      </Group>
      {/* Off dims the rest and takes it out of reach; the values stay stored, as Hide AI's do. */}
      <div inert={!settings.enabled} className={settings.enabled ? undefined : "opacity-(--opacity-dimmed)"} data-desk-settings-body="true">
        <Group title={t("desk.settings.title")} note={t("desk.settings.accounts")}>
          <Row label={t("desk.settings.defaultAgent")} detail={<span className="text-caption text-muted-foreground">{t("desk.settings.defaultAgentDetail")}</span>}>
            <Select value={settings.default_agent ?? undefined} onValueChange={(value) => onChange({ ...settings, default_agent: value as DeskAgentId })}>
              <SelectTrigger size="sm" aria-label={t("desk.settings.defaultAgent")}><SelectValue placeholder={t("desk.settings.defaultAgentNone")} /></SelectTrigger>
              <SelectContent>
                {agents.filter(agentPickable).map((agent) => (
                  <SelectItem key={agent.id} value={agent.id}><AgentMark kind={agent.id} />{deskAgentName(t, agent.id)}</SelectItem>
                ))}
              </SelectContent>
            </Select>
          </Row>
          <Row label={t("desk.settings.location")} detail={(
            <RadioGroup value={settings.location} onValueChange={(value) => onChange({ ...settings, location: value as DeskLocation })} aria-label={t("desk.settings.location")} className="gap-xs">
              {LOCATIONS.map((location) => (
                <label key={location} className="flex items-start gap-sm text-body" data-desk-settings-location={location}>
                  <RadioGroupItem value={location} className="mt-xxs" />
                  <span className="flex min-w-0 flex-col">
                    <span className="text-foreground">{t(`desk.settings.location.${location}`)}</span>
                    {location === "desk" ? <span className="whitespace-normal break-keep text-caption text-muted-foreground">{t("desk.settings.location.deskDetail")}</span> : null}
                  </span>
                </label>
              ))}
            </RadioGroup>
          )} />
          <Row label={t("desk.settings.cap")} detail={<span className="text-caption text-muted-foreground">{t("desk.settings.capDetail")}</span>}>
            <Select value={String(settings.cap)} onValueChange={(value) => onChange({ ...settings, cap: Number(value) })}>
              <SelectTrigger size="sm" aria-label={t("desk.settings.cap")} className="w-auto" data-desk-settings-cap={settings.cap}><SelectValue /></SelectTrigger>
              <SelectContent>
                {CAP_CHOICES.map((cap) => <SelectItem key={cap} value={String(cap)}>{cap}</SelectItem>)}
              </SelectContent>
            </Select>
          </Row>
        </Group>
      </div>
    </div>
  );
}
