import { ChevronLeftIcon, ChevronRightIcon } from "lucide-react";
import { useState } from "react";
import { glanceGroup, glanceSessions, type DeskJob } from "../../desk";
import { useInterfaceTranslation } from "../../i18n/client";
import { cn } from "../../lib/utils";
import type { DeskGlance, DeskGlanceFile } from "../../snapshot";
import { Button } from "../ui/button";
import { RowButton } from "../ui/row-button";
import { Separator } from "../ui/separator";
import { Hint } from "../ui/tooltip";
import { JobLead, jobStateWords, UnseenMark } from "./desk-job-card";
import { ViewMark } from "./output-shelf";
import { PreviewTile, tileOf } from "./preview-tile";

/** The file rows 한눈에 shows before `… n개 더`. */
export const GLANCE_FILE_ROWS = 8;

/**
 * 한눈에, Desk's right side while no session is open beside it: the sessions
 * Desk started and the ones the operator opened beside Desk (내 차례 first,
 * then at work, then ended), and the files they and Desk's own agent opened
 * (볼 것 first, each row naming its session at its end). A session row opens
 * that session beside Desk; a file row opens it in front of Desk's desk and
 * turns it seen. `⟩` folds it, so Desk's chat takes the room.
 */
export function GlancePanel({ glance, cards, onSession, onFile, onFold }: {
  glance: DeskGlance;
  /** Every card Desk has (`laneCards`): each listed session's row reads its own. */
  cards: readonly DeskJob[];
  onSession: (pane: string) => void;
  onFile: (file: DeskGlanceFile) => void;
  onFold: () => void;
}) {
  const { t } = useInterfaceTranslation();
  const [allFiles, setAllFiles] = useState(false);
  const sessions = glanceSessions(glance.sessions, cards);
  const turn = sessions.filter((job) => glanceGroup(job) === "turn").length;
  const totalFiles = glance.files.length + glance.files_more;
  const toLook = glance.files.filter((file) => !file.seen).length;
  const shown = allFiles ? glance.files : glance.files.slice(0, GLANCE_FILE_ROWS);
  const more = totalFiles - shown.length;
  const nameOf = (pane: string | null) => (pane === null ? t("desk.name") : (cards.find((card) => card.pane_id === pane)?.title ?? ""));
  return (
    <div className="flex min-h-0 flex-1 flex-col" data-desk-glance={sessions.length}>
      <div className="flex h-(--size-tab-strip) shrink-0 items-center gap-xs border-b border-border px-md text-caption">
        <span className="flex-1 font-semibold text-foreground">{t("desk.glance.title")}</span>
        <Hint label={t("desk.glance.fold")}>
          <Button variant="ghost" size="icon-sm" className="text-muted-foreground" aria-label={t("desk.glance.fold")} data-desk-glance-fold="true" onClick={onFold}><ChevronRightIcon /></Button>
        </Hint>
      </div>
      <div className="flex min-h-0 flex-1 flex-col overflow-y-auto px-xs py-xs">
        {sessions.length === 0 && totalFiles === 0 ? <p className="px-xs py-sm text-caption text-muted-foreground" data-desk-glance-empty="true">{t("desk.glance.empty")}</p> : null}
        {sessions.length > 0 ? (
          <section aria-label={t("desk.glance.sessions", { count: sessions.length })} data-desk-glance-sessions={sessions.length}>
            <GroupHead start={t("desk.glance.sessions", { count: sessions.length })} end={turn > 0 ? t("desk.glance.turn", { count: turn }) : null} />
            {sessions.map((job) => (
              <RowButton key={job.pane_id} className={cn("gap-xs text-body", job.disconnected && "opacity-(--opacity-dimmed)")} data-desk-glance-session={job.pane_id} data-desk-glance-group={glanceGroup(job)} data-desk-glance-disconnected={job.disconnected ? "true" : undefined} onClick={() => onSession(job.pane_id!)}>
                <JobLead job={job} />
                <span className="min-w-0 flex-1 truncate text-foreground">{job.title}</span>
                {/* A device that is not connected says so, as its card in the conversation does. */}
                <span className={cn("shrink-0 text-caption", glanceGroup(job) === "turn" && !job.disconnected ? "text-warning" : "text-muted-foreground")}>{job.disconnected ? t("desk.job.disconnected") : jobStateWords(t, job)}</span>
              </RowButton>
            ))}
          </section>
        ) : null}
        {sessions.length > 0 && totalFiles > 0 ? <Separator className="my-xs" /> : null}
        {totalFiles > 0 ? (
          <section aria-label={t("desk.glance.files", { count: totalFiles })} data-desk-glance-files={totalFiles}>
            <GroupHead start={t("desk.glance.files", { count: totalFiles })} end={toLook > 0 ? t("desk.views.toLook", { count: toLook }) : null} />
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
            {more > 0 && !allFiles ? (
              <RowButton className="text-caption text-muted-foreground" data-desk-glance-more={more} onClick={() => setAllFiles(true)}>
                <span className="size-(--size-icon) shrink-0" aria-hidden="true" />
                {t("desk.glance.more", { count: more })}
              </RowButton>
            ) : null}
            {allFiles && glance.files_more > 0 ? <p className="px-xs py-xs text-caption text-muted-foreground" data-desk-glance-older="true">{t("desk.views.older")}</p> : null}
          </section>
        ) : null}
      </div>
    </div>
  );
}

function GroupHead({ start, end }: { start: string; end: string | null }) {
  return (
    <div className="flex items-center px-xs pb-xxs pt-xs text-caption text-muted-foreground">
      <span className="flex-1">{start}</span>
      {end ? <span>{end}</span> : null}
    </div>
  );
}

/**
 * The way back above the session open beside Desk: `‹ 한눈에 › <session>`.
 * `‹ 한눈에` closes the session, as Esc does, and 한눈에 stands there again.
 */
export function GlanceCrumb({ title, lead = null, onBack }: { title: string; lead?: React.ReactNode; onBack: () => void }) {
  const { t } = useInterfaceTranslation();
  return (
    <nav aria-label={t("desk.glance.back")} className="flex h-(--size-control-lg) shrink-0 items-center gap-xxs border-b border-border px-xs text-caption text-muted-foreground" data-desk-glance-crumb="true">
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
