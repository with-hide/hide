import type { Actions } from "../actions";
import { hostLook } from "../codeHost";
import { Button } from "../components/ui/button";
import { Group, Row, Status } from "../components/settings-rows";
import { useInterfaceTranslation } from "../i18n/client";
import { coreMachineAway } from "../screenMachine";
import { githubAccessLine, hostAccessRows, type HostAccessRow } from "../settings";
import { readsHere } from "../devices";
import { catalogOf } from "../snapshot";
import { useShellStore } from "../store";
import { FixCommand } from "./FixCommand";

/**
 * Remote repositories: one row per code host the core reads with its own machine's programs, GitHub always and
 * another host once one of its projects was read, each with how its reads stand and a way to read them again.
 */
export function RemoteRepositoriesTab({ actions }: { actions: Actions }) {
  const workspaces = useShellStore((s) => s.rest?.navigator?.workspaces);
  const remote = useShellStore((s) => s.rest?.status?.remote);
  const devices = useShellStore((s) => s.rest?.navigator?.devices);
  // The projects whose reads run on the core's machine: its own Git projects and a node's that dials it, never a dialed device's.
  const projects = catalogOf(workspaces, remote).filter((workspace) => workspace.is_git && readsHere(workspace, devices));
  const rows = hostAccessRows(projects, devices);
  const refresh = () => actions.refreshGithub(projects.map((workspace) => workspace.id));
  return (
    <Group data-settings-group="github">
      {rows.map((row) => <HostRow key={row.key} row={row} actions={actions} onCheckAgain={refresh} />)}
    </Group>
  );
}

function HostRow({ row, actions, onCheckAgain }: { row: HostAccessRow; actions: Actions; onCheckAgain: () => void }) {
  const { t } = useInterfaceTranslation();
  const devices = useShellStore((s) => s.rest?.navigator?.devices);
  const look = hostLook(row.key);
  const { access, declaration } = row;
  const coreMachine = coreMachineAway(devices, t);
  const line = access ? githubAccessLine(access, t, coreMachine, declaration) : null;
  // Signed out: the command the host declared to sign its program in, and the core's machine to run it on when this window is on another.
  const signIn = access?.state === "failed" && access.category === "not_logged_in" ? declaration?.access.sign_in ?? null : null;
  return (
    <Row
      data-settings-host={row.key}
      label={
        <span className="flex min-w-0 flex-col gap-xxs">
          <span>{look.name}</span>
          {look.description ? <span className="text-body text-muted-foreground">{t(look.description)}</span> : null}
        </span>
      }
      detail={
        signIn ? (
          <span className="flex min-w-0 flex-wrap items-center gap-sm" data-github-fix-on={coreMachine ?? "here"}>
            <span className="text-body text-muted-foreground">{coreMachine ? t("settings.fixOn", { machine: coreMachine }) : t("githubReason.fixHere")}</span>
            <FixCommand command={signIn} what={`${declaration!.access.program} sign-in command`} actions={actions} data-github-fix-command="true" />
          </span>
        ) : null
      }
    >
      {line ? (
        <Status tone={line.tone} data-issue-source-github={access?.state === "failed" ? access.category : "connected"}>
          {line.text}
        </Status>
      ) : (
        <Status tone="muted" data-issue-source-github="unread">
          {t("settings.notChecked")}
        </Status>
      )}
      {access?.state === "failed" ? (
        <Button variant="secondary" onClick={onCheckAgain} data-github-check-again="true">
          {t("settings.checkAgain")}
        </Button>
      ) : null}
    </Row>
  );
}
