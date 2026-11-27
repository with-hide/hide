import { ChevronDownIcon, ChevronLeftIcon, ChevronRightIcon, PanelRightIcon, SearchIcon } from "lucide-react";
import { useState } from "react";
import { glanceGroup, glanceSessions, type DeskJob, type GlanceGroup } from "../../desk";
import type { MessageKey } from "../../i18n/catalogs";
import { useInterfaceTranslation } from "../../i18n/client";
import { cn } from "../../lib/utils";
import type { DeskGlance, DeskGlanceFile, GlanceFolds, GlanceGroupKey, GlanceTab } from "../../snapshot";
import { Elapsed } from "../elapsed";
import { Button } from "../ui/button";
import { Input } from "../ui/input";
import { RowButton } from "../ui/row-button";
import { Tabs, TabsList, TabsTrigger } from "../ui/tabs";
import { Hint } from "../ui/tooltip";
import { JobLead, JobLine, UnseenMark } from "./desk-job-card";
import { ViewMark } from "./output-shelf";
import { PreviewTile, tileOf } from "./preview-tile";

/** The file rows the 파일 tab shows before `… n개 더`. */
export const GLANCE_FILE_ROWS = 8;

/** How 한눈에's session groups start before the operator folds one: 끝남 folded. */
export const GLANCE_FOLDS: GlanceFolds = { turn: false, working: false, ended: true };

/** Each card group under the key the core keeps its fold by, in the order 한눈에 lists them. */
const GROUPS: readonly { group: GlanceGroup; key: GlanceGroupKey; label: MessageKey }[] = [
  { group: "turn", key: "turn", label: "desk.glance.group.turn" },
  { group: "working", key: "working", label: "desk.glance.group.working" },
  { group: "idle", key: "ended", label: "desk.glance.group.ended" },
];

/** Whether `name` holds what the operator typed in 한눈에's search box, ignoring case and the spaces around it. */
export function glanceMatches(name: string, query: string): boolean {
  const wanted = query.trim().toLocaleLowerCase();
  return wanted === "" || name.toLocaleLowerCase().includes(wanted);
}

/**
 * 한눈에, Desk's right side while no session is open beside it, in two tabs
 * the core keeps: `세션`, the sessions Desk started and the ones the
 * operator opened beside Desk in three groups (내 차례, 일하는 중, 끝남), each
 * head folding its group, and `파일 n`, the Views they and Desk's own agent
 * opened (볼 것 first, each row naming its session at its end). A search box
 * under the tabs keeps the open tab's rows whose name holds its words. A
 * session row opens that session beside Desk; a file row opens it in front
 * of Desk's desk and turns it seen. `▯|` folds the whole panel, so Desk's
 * chat takes the room.
 */
export function GlancePanel({ glance, cards, tab = "sessions", folds = GLANCE_FOLDS, onTab, onGroup, onSession, onFile, onFold }: {
  glance: DeskGlance;
  /** Every card Desk has (`laneCards`): each listed session's row reads its own. */
  cards: readonly DeskJob[];
  tab?: GlanceTab;
  folds?: GlanceFolds;
  onTab: (tab: GlanceTab) => void;
  onGroup: (group: GlanceGroupKey, folded: boolean) => void;
  onSession: (pane: string) => void;
  onFile: (file: DeskGlanceFile) => void;
  onFold: () => void;
}) {
  const { t } = useInterfaceTranslation();
  // What the search box holds is the window's, for the tab it was typed on.
  const [query, setQuery] = useState({ tab, text: "" });
  const text = query.tab === tab ? query.text : "";
  const sessions = glanceSessions(glance.sessions, cards);
  const totalFiles = glance.files.length + glance.files_more;
  const toLook = glance.files.filter((file) => !file.seen).length;
  const listed = tab === "sessions" ? sessions.length : totalFiles;
  const search = tab === "sessions" ? t("desk.glance.findSessions") : t("desk.glance.findFiles");
  return (
    <Tabs value={tab} onValueChange={(next) => onTab(next as GlanceTab)} className="flex min-h-0 flex-1 flex-col gap-0" data-desk-glance={sessions.length} data-desk-glance-tab={tab}>
      <div className="flex h-(--size-tab-strip) shrink-0 items-center gap-xs border-b border-border px-xs text-caption">
        <TabsList aria-label={t("desk.glance.title")} className="flex-1 bg-transparent">
          <TabsTrigger value="sessions" className="text-caption" data-desk-glance-tab-trigger="sessions">
            {t("desk.glance.sessionsTab")}
            {/* A session asking the operator marks the tab while the other one is open. */}
            {tab !== "sessions" && sessions.some((job) => glanceGroup(job) === "turn") ? <span aria-hidden="true" className="size-(--size-tab-status-dot) rounded-full bg-warning" data-desk-glance-tab-turn="true" /> : null}
          </TabsTrigger>
          <TabsTrigger value="files" className="text-caption" data-desk-glance-tab-trigger="files">
            {t("desk.glance.filesTab", { count: totalFiles })}
            {tab !== "files" && toLook > 0 ? <UnseenMark /> : null}
          </TabsTrigger>
        </TabsList>
        <Hint label={t("desk.glance.fold")}>
          <Button variant="ghost" size="icon-sm" className="text-subtle-foreground" aria-label={t("desk.glance.fold")} data-desk-glance-fold="true" onClick={onFold}><PanelRightIcon /></Button>
        </Hint>
      </div>
      {listed > 0 ? (
        <div className="relative mx-sm mt-sm flex shrink-0 items-center">
          <SearchIcon aria-hidden="true" className="pointer-events-none absolute left-sm size-(--size-icon-sm) text-muted-foreground" />
          <Input type="search" maxLength={200} value={text} placeholder={search} aria-label={search} onChange={(event) => setQuery({ tab, text: event.target.value })} className="h-(--size-control-sm) pl-[calc(var(--spacing-sm)+var(--size-icon-sm)+var(--spacing-xs))] text-caption" data-desk-glance-search={tab} />
        </div>
      ) : null}
      <div className="flex min-h-0 flex-1 flex-col overflow-y-auto px-xs py-xs">
        {tab === "sessions" ? <SessionGroups sessions={sessions} query={text} folds={folds} onGroup={onGroup} onSession={onSession} /> : <FileRows glance={glance} cards={cards} query={text} onFile={onFile} />}
      </div>
    </Tabs>
  );
}

/** The 세션 tab: 내 차례, 일하는 중 and 끝남, each under a head with its count that folds it; a group with nothing in it is left out. */
function SessionGroups({ sessions, query, folds, onGroup, onSession }: { sessions: readonly DeskJob[]; query: string; folds: GlanceFolds; onGroup: (group: GlanceGroupKey, folded: boolean) => void; onSession: (pane: string) => void }) {
  const { t } = useInterfaceTranslation();
  if (sessions.length === 0) return <Quiet data-desk-glance-empty="true">{t("desk.glance.emptySessions")}</Quiet>;
  const found = sessions.filter((job) => glanceMatches(job.title, query));
  if (found.length === 0) return <Quiet data-desk-glance-none="sessions">{t("desk.glance.noSessions")}</Quiet>;
  return (
    <>
      {GROUPS.map(({ group, key, label }) => {
        const rows = found.filter((job) => glanceGroup(job) === group);
        if (rows.length === 0) return null;
        const folded = folds[key];
        const name = t(label, { count: rows.length });
        return (
          <section key={key} aria-label={name} data-desk-glance-sessions={key} data-desk-glance-folded={folded ? "true" : undefined}>
            <GroupHead label={name} folded={folded} onToggle={() => onGroup(key, !folded)} data={key} />
            {folded ? null : rows.map((job) => <SessionRow key={job.pane_id} job={job} onSession={onSession} />)}
          </section>
        );
      })}
    </>
  );
}

/** A session in 한눈에: its marks, its name and the time since it last moved, and under them its state in one line as its card says it. */
function SessionRow({ job, onSession }: { job: DeskJob; onSession: (pane: string) => void }) {
  return (
    <RowButton className={cn("items-start gap-xs py-xs", job.disconnected && "opacity-(--opacity-dimmed)")} data-desk-glance-session={job.pane_id} data-desk-glance-group={glanceGroup(job)} data-desk-glance-disconnected={job.disconnected ? "true" : undefined} onClick={() => onSession(job.pane_id!)}>
      <span className="flex h-(--size-sidebar-line) shrink-0 items-center gap-xs"><JobLead job={job} /></span>
      <span className="flex min-w-0 flex-1 flex-col">
        <span className="flex min-w-0 items-center gap-xs">
          <span className="min-w-0 flex-1 truncate text-body text-foreground">{job.title}</span>
          <Elapsed since={job.changed_at_unix_ms > 0 ? job.changed_at_unix_ms : null} className="shrink-0 font-mono text-caption text-muted-foreground" data-desk-glance-since="true" />
        </span>
        <JobLine job={job} place={false} />
      </span>
    </RowButton>
  );
}

/** The 파일 tab, today's file group: 볼 것 first, each row naming the session that opened it at its end. */
function FileRows({ glance, cards, query, onFile }: { glance: DeskGlance; cards: readonly DeskJob[]; query: string; onFile: (file: DeskGlanceFile) => void }) {
  const { t } = useInterfaceTranslation();
  const [allFiles, setAllFiles] = useState(false);
  const totalFiles = glance.files.length + glance.files_more;
  if (totalFiles === 0) return <Quiet data-desk-glance-empty="true">{t("desk.glance.emptyFiles")}</Quiet>;
  const searching = query.trim() !== "";
  const found = glance.files.filter((file) => glanceMatches(file.label, query));
  if (found.length === 0) return <Quiet data-desk-glance-none="files">{t("desk.glance.noFiles")}</Quiet>;
  const toLook = found.filter((file) => !file.seen).length;
  // A search looks through every file the core lists, not only the first rows.
  const shown = allFiles || searching ? found : found.slice(0, GLANCE_FILE_ROWS);
  const more = totalFiles - shown.length;
  const nameOf = (pane: string | null) => (pane === null ? t("desk.name") : (cards.find((card) => card.pane_id === pane)?.title ?? ""));
  return (
    <section aria-label={t("desk.glance.filesTab", { count: totalFiles })} data-desk-glance-files={totalFiles}>
      {toLook > 0 ? <div className="px-xs pb-xxs pt-xs text-caption text-muted-foreground">{t("desk.views.toLook", { count: toLook })}</div> : null}
      {shown.map((file) => (
        <Hint key={`${file.device_id}:${file.checkout}:${file.id}`} label={file.label} side="left" content={<PreviewTile file={tileOf(file)} />}>
          <RowButton className="gap-xs text-body" data-desk-glance-file={file.id} data-desk-glance-unseen={file.seen ? undefined : "true"} onClick={() => onFile(file)}>
            <span className="flex size-(--size-icon) shrink-0 items-center justify-center"><ViewMark output={file} /></span>
            <span className="min-w-0 flex-1 truncate text-foreground">{file.label}</span>
            <span className="max-w-[40%] shrink-0 truncate text-caption text-muted-foreground" data-desk-glance-by={file.pane_id ?? "desk"}>{nameOf(file.pane_id)}</span>
            {file.seen ? null : <UnseenMark />}
          </RowButton>
        </Hint>
      ))}
      {more > 0 && !allFiles && !searching ? (
        <RowButton className="text-caption text-muted-foreground" data-desk-glance-more={more} onClick={() => setAllFiles(true)}>
          <span className="size-(--size-icon) shrink-0" aria-hidden="true" />
          {t("desk.glance.more", { count: more })}
        </RowButton>
      ) : null}
      {(allFiles || searching) && glance.files_more > 0 ? <p className="px-xs py-xs text-caption text-muted-foreground" data-desk-glance-older="true">{t("desk.views.older")}</p> : null}
    </section>
  );
}

/** A group's head: its name and count, and the chevron that says whether it is folded; a press folds or unfolds it. */
function GroupHead({ label, folded, onToggle, data }: { label: string; folded: boolean; onToggle: () => void; data: string }) {
  const Chevron = folded ? ChevronRightIcon : ChevronDownIcon;
  return (
    <RowButton className="gap-xxs pb-xxs pt-xs text-caption text-muted-foreground" aria-expanded={!folded} data-desk-glance-group-head={data} onClick={onToggle}>
      <span>{label}</span>
      <Chevron aria-hidden="true" className="size-(--size-icon-sm) shrink-0" />
    </RowButton>
  );
}

/** One muted line where a tab has nothing to list. */
function Quiet({ children, ...props }: { children: React.ReactNode } & React.HTMLAttributes<HTMLParagraphElement>) {
  return <p className="px-xs py-sm text-caption text-muted-foreground" {...props}>{children}</p>;
}

/**
 * The way back above the session open beside Desk: `‹ 한눈에 › <session>`.
 * `‹ 한눈에` closes the session, as Esc does, and 한눈에 stands there again.
 */
export function GlanceCrumb({ title, lead = null, onBack }: { title: string; lead?: React.ReactNode; onBack: () => void }) {
  const { t } = useInterfaceTranslation();
  return (
    <nav aria-label={t("desk.glance.back")} className="flex h-(--size-tab-strip) shrink-0 items-center gap-xxs border-b border-border px-xs text-caption text-muted-foreground" data-desk-glance-crumb="true">
      <Hint label={t("desk.glance.back")} shortcut="Esc">
        <Button variant="ghost" size="sm" className="h-(--size-control-compact) gap-xxs px-xs text-caption font-normal text-subtle-foreground" data-desk-glance-back="true" onClick={onBack}>
          <ChevronLeftIcon aria-hidden="true" />
          {t("desk.glance.title")}
        </Button>
      </Hint>
      <ChevronRightIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0" />
      {lead}
      <span className="min-w-0 truncate text-foreground" aria-current="page">{title}</span>
    </nav>
  );
}
