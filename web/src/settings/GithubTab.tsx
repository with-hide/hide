import type { Actions } from "../actions";
import { Button } from "../components/ui/button";
import { Group, Row, Status } from "../components/settings-rows";
import { useInterfaceTranslation } from "../i18n/client";
import { checkCommand } from "../coreMove";
import { coreMachineAway } from "../screenMachine";
import { githubAccess, githubAccessLine } from "../settings";
import { readsHere } from "../devices";
import { catalogOf } from "../snapshot";
import { useShellStore } from "../store";
import { FixCommand } from "./FixCommand";

/** GitHub through `gh`: the state the core resolved across the Git projects it reads with its own login, and a way to read them again. */
export function GithubTab({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const workspaces = useShellStore((s) => s.rest?.navigator?.workspaces);
  const remote = useShellStore((s) => s.rest?.status?.remote);
  const devices = useShellStore((s) => s.rest?.navigator?.devices);
  // The projects whose issues and pull requests are read with the core's `gh`:
  // its own Git projects and a node's that dials it, never a dialed device's.
  const projects = catalogOf(workspaces, remote).filter((workspace) => !workspace.is_home && workspace.is_git && readsHere(workspace, devices));
  const access = githubAccess(projects, devices);
  const coreMachine = coreMachineAway(devices, t);
  const line = access ? githubAccessLine(access, t, coreMachine) : null;
  // Signed out: the command that signs gh in, and the core's machine to run it on when this window is on another.
  const signedOut = access?.state === "failed" && access.category === "not_logged_in";
  const ghCommand = checkCommand("gh");
  return (
    <Group data-settings-group="github">
      <Row
        label={
          <span className="flex min-w-0 flex-col gap-xxs">
            <span>GitHub</span>
            <span className="text-body text-muted-foreground">{t("settings.githubDescription")}</span>
          </span>
        }
        detail={
          signedOut && ghCommand ? (
            <span className="flex min-w-0 flex-wrap items-center gap-sm" data-github-fix-on={coreMachine ?? "here"}>
              <span className="text-body text-muted-foreground">{coreMachine ? t("settings.fixOn", { machine: coreMachine }) : t("githubReason.fixHere")}</span>
              <FixCommand command={ghCommand} what="gh sign-in command" actions={actions} data-github-fix-command="true" />
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
          <Button variant="secondary" onClick={() => actions.refreshGithub(projects.map((workspace) => workspace.id))} data-github-check-again="true">
            {t("settings.checkAgain")}
          </Button>
        ) : null}
      </Row>
    </Group>
  );
}
