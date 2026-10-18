import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "./components/ui/select";
import { useInterfaceTranslation } from "./i18n/client";
import { destinationOf, githubSource, type BacklogDestination } from "./overviewBacklog";
import type { Workspace } from "./snapshot";

/** Local is the default. A GitHub choice names its repository and visibility before typing. */
export function BacklogDestinationPicker({ project, value, disabled, onChange }: { project: Workspace; value: BacklogDestination; disabled?: boolean; onChange: (value: BacklogDestination) => void }) {
  const { t } = useInterfaceTranslation();
  const source = githubSource(project);
  const repository = source?.name;
  const scope = destinationOf(source);
  return <Select value={value.kind === "github" ? value.repository : "local"} disabled={disabled} onValueChange={(choice) => onChange(choice === "local" ? { kind: "local" } : { kind: "github", repository: choice })}>
    <SelectTrigger className="w-full" aria-label={t("backlogView.destination")} data-backlog-destination={value.kind}><SelectValue /></SelectTrigger>
    <SelectContent>
      <SelectItem value="local">{t("backlogView.localScope")}</SelectItem>
      {repository ? <SelectItem value={repository}>{repository} · {t(scope === "public" ? "backlogView.public" : scope === "private" ? "backlogView.private" : "backlogView.unknown")}</SelectItem> : null}
    </SelectContent>
  </Select>;
}
