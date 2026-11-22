// What's new scene for gitlab.com projects: a project whose remote is on
// gitlab.com is read like a GitHub one. The PR list holds a GitHub pull
// request, then the gitlab.com project's merge requests come in beside it as
// `!12` and `!14` with GitLab's mark on their project; the pointer opens `!12`
// and its card names GitLab, its failed pipeline and the page on gitlab.com,
// with no button GitLab does not offer. It is drawn from the screens' own
// parts and words (`PrMark`, `hostLook`, `work.card.github`,
// `recordCard.ciFailed`, `work.reason.failed`), and the projects and their work are examples, never
// the operator's.

import { hostLook } from "../../codeHost";
import { useInterfaceTranslation } from "../../i18n/translator";
import { cn } from "../../lib/utils";
import type { PrState } from "../../snapshot";
import { PrMark } from "../pr-mark";
import { ScenePointer } from "./pointer";
import { SCENE_CSS, type SceneProps } from "./scene";

/** One loop, in ms: the beats below fall inside it. */
export const GITLAB_LOOP_MS = 10_000;
/** The gitlab.com project's merge requests come into the list beside GitHub's pull request. */
const READ = 1_200;
/** The pointer comes to `!12` and presses it; its card opens. */
const POINT = 3_000;
const PRESS = 3_600;
const OPENED = 4_000;
/** The moment a still picture shows: both hosts in the list and the merge request's card open. */
export const GITLAB_STILL_MS = 6_500;

/** The example projects: one read from GitHub, one from gitlab.com. */
const GITHUB_PROJECT = "web";
const GITLAB_PROJECT = "shop-api";
const GITLAB_REPOSITORY = `acme/${GITLAB_PROJECT}`;
/** The pipeline job that failed on `!12`. */
const FAILED_JOB = "unit-test";

type Item = { host: "github" | "gitlab"; number: number; state: PrState; title: "tabs" | "retry" | "cache"; project: string };
const ITEMS: Item[] = [
  { host: "gitlab", number: 12, state: "failed", title: "retry", project: GITLAB_PROJECT },
  { host: "github", number: 2806, state: "mergeable", title: "tabs", project: GITHUB_PROJECT },
  { host: "gitlab", number: 14, state: "pending", title: "cache", project: GITLAB_PROJECT },
];

export function GitlabScene({ at }: SceneProps) {
  const { t } = useInterfaceTranslation();
  const read = at >= READ;
  const opened = at >= OPENED;
  const shown = ITEMS.filter((item) => item.host === "github" || read);
  return (
    <div className="relative flex aspect-[2/1] overflow-hidden bg-background text-foreground" data-whats-new-scene="gitlab" data-scene-beat={opened ? "card" : read ? "read" : "list"} aria-hidden>
      <style>{SCENE_CSS}</style>
      <div className="flex w-1/2 min-w-0 flex-col border-r border-border bg-sidebar">
        <span className="border-b border-border px-md py-xs text-caption font-medium text-muted-foreground">{t("projectTool.sidebar.prsCount", { count: shown.length })}</span>
        {shown.map((item) => <Row key={`${item.host}${item.number}`} item={item} rise={item.host === "gitlab"} selected={opened && item.number === 12} />)}
      </div>
      <div className="flex min-w-0 flex-1 flex-col p-md">
        {opened ? <Card /> : <span className="m-auto text-caption text-subtle-foreground">{t(`whatsNewScene.gitlab.${read ? "read" : "waiting"}`)}</span>}
      </div>
      <ScenePointer left={22} top={read ? 30 : 60} shown={at >= POINT && at < OPENED + 700} pressed={at >= PRESS} />
    </div>
  );
}

/** A list row as the PR list draws it: the mark with the host's number, the title, and the project with its host's mark. */
function Row({ item, rise, selected }: { item: Item; rise: boolean; selected: boolean }) {
  const { t } = useInterfaceTranslation();
  const { Mark, numberLabel } = hostLook(item.host);
  return (
    <span className={cn("flex min-w-0 flex-col gap-xxs border-b border-border px-md py-xs", rise && "wns-rise", selected && "bg-accent")}>
      <span className="flex min-w-0 items-center gap-xs text-caption">
        <PrMark state={item.state} label={numberLabel(item.number)} />
        <span className="truncate font-medium">{t(`whatsNewScene.gitlab.${item.title}`)}</span>
      </span>
      <span className="flex items-center gap-xxs text-micro text-muted-foreground">
        <Mark className="size-(--size-icon-sm) shrink-0" />
        {item.project}
      </span>
    </span>
  );
}

/** `!12`'s card: its number as GitLab writes it, its project, its failed pipeline and the page on gitlab.com; no merge or hand-over, which GitLab does not offer here. */
function Card() {
  const { t } = useInterfaceTranslation();
  const { Mark, name, item } = hostLook("gitlab");
  return (
    <div className="wns-rise flex min-w-0 flex-col gap-sm">
      <span className="flex items-center gap-xs text-micro text-muted-foreground">
        <Mark className="size-(--size-icon-sm) shrink-0" />
        {GITLAB_REPOSITORY}
      </span>
      <span className="flex min-w-0 items-center gap-xs">
        <PrMark state="failed" label={`${item} !12`} />
        <span className="truncate text-body font-medium">{t("whatsNewScene.gitlab.retry")}</span>
      </span>
      <span className="text-caption text-destructive">{t("recordCard.ciFailed", { count: 1, total: 2 })}</span>
      <span className="font-mono text-micro text-muted-foreground">{t("work.reason.failed", { name: FAILED_JOB })}</span>
      <span className="self-start rounded-md border border-border px-sm py-xxs text-micro">{t("work.card.github", { host: name })}</span>
    </div>
  );
}
