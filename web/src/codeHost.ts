// How the screen draws a code host (docs/ARCHITECTURE.md, Code hosts): its mark, its name, how it writes a change
// request's number and what it calls one, made in this one place from the host key the core carries. No screen
// spells a host's name, a `#` before a number or a host's address itself: it asks here, and opens the pages the
// core's unit made (`ChangeRequestRef`, a status's `repository`).

import { FolderGit2Icon, type LucideIcon } from "lucide-react";
import { GithubIcon } from "./components/github-icon";
import { GitlabIcon } from "./components/gitlab-icon";
import type { MessageKey } from "./i18n/catalogs";
import type { ChangeRequestRef, GithubStatus, HostDeclaration, HostOperation, Workspace } from "./snapshot";

/**
 * What a host looks like on the screen. `item` is what it calls a change request (`PR`, `MR`), which no language
 * translates; `description` is its row's line in Settings › Remote repositories.
 */
export type HostLook = { key: string; name: string; Mark: LucideIcon; item: string; noun: MessageKey; numberLabel: (number: number) => string; description: MessageKey | null };

const LOOKS: Readonly<Record<string, HostLook>> = {
  github: { key: "github", name: "GitHub", Mark: GithubIcon, item: "PR", noun: "settings.hostNoun.github", numberLabel: (number) => `#${number}`, description: "settings.githubDescription" },
  gitlab: { key: "gitlab", name: "GitLab", Mark: GitlabIcon, item: "MR", noun: "settings.hostNoun.gitlab", numberLabel: (number) => `!${number}`, description: "settings.gitlabDescription" },
};

/** The host a value without a host is from: a core older than hosts read GitHub alone. */
const UNDECLARED = "github";

/** A host as a reading names it: its key, its name and the program the core reads it with. */
export type NamedHost = { key: string; name: string; program: string };

/**
 * The host `declaration` names, or the one a status without a declaration is from: GitHub, read with `gh`, as by a
 * core before hosts. Every screen that names a status's host or program asks here.
 */
export function namedHost(declaration: HostDeclaration | null | undefined): NamedHost {
  return declaration ? { key: declaration.key, name: declaration.name, program: declaration.access.program } : { key: UNDECLARED, name: LOOKS[UNDECLARED]!.name, program: "gh" };
}

/**
 * The look of the host `key` names; no key is GitHub's, as a core before hosts sent none. A host the core
 * registered and this table does not know yet draws a plain repository mark under its key, and its numbers come
 * from the references its unit labelled.
 */
export function hostLook(key: string | null | undefined): HostLook {
  const found = LOOKS[key ?? UNDECLARED];
  if (found) return found;
  return { ...LOOKS[UNDECLARED]!, key: key!, name: key!, Mark: FolderGit2Icon, description: null };
}

/** Whether `declaration` offers `operation`: an absent declaration is GitHub's from before hosts, which offered every one. */
export function offers(declaration: HostDeclaration | null | undefined, operation: HostOperation): boolean {
  return !declaration || declaration.operations.includes(operation);
}

/** A change request as a list or a mark holds it: its number and, from a core that carries one, its reference. */
type Pull = { number: number; reference?: ChangeRequestRef | null; /** A bare reference is not a pull: hand `{ number, reference }`. */ repository?: never };

/** The host a pull request is on. */
export function pullHost(pull: Pull): HostLook {
  return hostLook(pull.reference?.repository.host);
}

/** How its host writes the pull request's number (`#12`, `!12`). */
export function pullLabel(pull: Pull): string {
  return pull.reference?.label || pullHost(pull).numberLabel(pull.number);
}

/** What its host calls it, with its number: `PR #12`, `MR !12`. */
export function pullName(pull: Pull): string {
  return `${pullHost(pull).item} ${pullLabel(pull)}`;
}

/**
 * Whether two addresses name the same pull request, by the keys their host's unit gave them; an address no
 * registered host reads has no reference and is the same as nothing.
 */
export function samePull(left: ChangeRequestRef | null | undefined, right: ChangeRequestRef | null | undefined): boolean {
  return !!left?.key && left.key === right?.key;
}

/** The statuses of a project's checkouts that a host answered for. */
function hostStatuses(workspace: Workspace): GithubStatus[] {
  return workspace.checkouts.flatMap((checkout) => (checkout.github?.host ? [checkout.github] : []));
}

/** The host that reads `workspace`, as its last read declared it; null before any host answered and for a project no host reads. */
export function projectHost(workspace: Workspace): HostDeclaration | null {
  return statusesHost(hostStatuses(workspace));
}

/** The host the first of `statuses` that a host answered for declares; null when none did. */
export function statusesHost(statuses: readonly GithubStatus[]): HostDeclaration | null {
  return statuses.find((status) => status.host)?.host ?? null;
}

/**
 * The host whose operations decide a pull request's buttons: the one its card's read declared, else its project's,
 * so a list row or a page drawn before its card arrives offers no button its host refuses. The desktop and the
 * phone both ask here.
 */
export function actionHost(cardHost: HostDeclaration | null | undefined, projectStatuses: readonly GithubStatus[]): HostDeclaration | null {
  return cardHost ?? statusesHost(projectStatuses);
}



/** The repository the host's read named for `workspace`, with its page; null before a read named one. */
export function hostRepository(workspace: Workspace): { name: string; url: string } | null {
  return workspace.checkouts.find((checkout) => checkout.github?.repository)?.github?.repository ?? null;
}

/** The words a sentence about the pull request takes, whichever its language uses: `pull` (`PR #12`), `label` (`#12`) and `host` (`GitHub`). */
export function pullWords(pull: Pull): { pull: string; label: string; host: string } {
  return { pull: pullName(pull), label: pullLabel(pull), host: pullHost(pull).name };
}

/**
 * The words of `workspace`'s pull request `number` where a caller holds only its number: the list's reference
 * when the list holds it, else the number as the project's host writes it.
 */
export function projectPullWords(workspace: Workspace | null | undefined, number: number): { pull: string; label: string; host: string } {
  const listed = workspace?.pull_requests?.find((pr) => pr.number === number);
  if (listed) return pullWords(listed);
  const look = hostLook(workspace ? projectHost(workspace)?.key : null);
  const label = look.numberLabel(number);
  return { pull: `${look.item} ${label}`, label, host: look.name };
}

/** The host of the project that holds the checkout at `path` on `deviceId`; GitHub's look while no project holds it, as a core without hosts. */
export function checkoutHost(workspaces: readonly Workspace[], deviceId: string, path: string): HostLook {
  const workspace = workspaces.find((candidate) => candidate.device_id === deviceId && candidate.checkouts.some((checkout) => checkout.path === path));
  return hostLook(workspace ? projectHost(workspace)?.key : null);
}
