// The files of a reply as the phone draws them (Doop 08, 09): on the reply bar while they upload, and over the operator's bubble once
// sent. A photo is its thumbnail; any other file is its mark, name and size. Drawn from values only, so a practice chapter draws the
// same parts.

import { CircleAlertIcon, ImageIcon, PlusIcon, XIcon } from "lucide-react";
import { FileMark } from "../components/file-mark";
import { Button } from "../components/ui/button";
import { fileIcon } from "../fileIcons";
import { formatBytes } from "../i18n/format";
import { requireInterfaceLanguage } from "../i18n/locale";
import { useInterfaceTranslation } from "../i18n/translator";
import { cn } from "../lib/utils";
import type { ComposerFile, FileNotice, NamedFile, SentFile } from "./uploads";

function useSize(): (bytes: number) => string {
  const { i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  return (bytes) => formatBytes(language, bytes);
}

/** The `+` that opens the system's picker, at the start of the reply field. */
export function AttachButton({ onPick, disabled }: { onPick: () => void; disabled: boolean }) {
  const { t } = useInterfaceTranslation();
  return (
    <Button type="button" variant="ghost" size="icon" aria-label={t("mobile.attach.add")} disabled={disabled} data-phone-attach="true" className="size-(--size-touch-target) shrink-0 rounded-full text-muted-foreground" onClick={onPick}>
      <PlusIcon aria-hidden="true" className="size-(--size-icon-lg)" />
    </Button>
  );
}

/** The band under a file's mark while its bytes go up. */
function Band({ file }: { file: ComposerFile }) {
  if (file.state !== "uploading" && file.state !== "preparing") return null;
  const share = file.size > 0 ? Math.round((file.sent / file.size) * 100) : 0;
  return (
    <span className="absolute inset-x-xs bottom-xs h-xxs overflow-hidden rounded-full bg-border" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={share}>
      <span className="block h-full rounded-full bg-agent-working" style={{ width: `${share}%` }} />
    </span>
  );
}

/** The `×` at a file's corner: only that file leaves. Its press reaches the touch target past the small circle it draws. */
function RemoveButton({ name, onRemove }: { name: string; onRemove: () => void }) {
  const { t } = useInterfaceTranslation();
  return (
    <Button type="button" variant="ghost" size="icon" aria-label={t("mobile.attach.remove", { name })} data-phone-attach-remove={name} className="absolute -top-md -right-md size-(--size-touch-target) rounded-full hover:bg-transparent" onClick={onRemove}>
      <span className="flex size-(--size-icon-lg) items-center justify-center rounded-full border border-border bg-secondary text-foreground">
        <XIcon aria-hidden="true" className="size-(--size-icon-sm)" />
      </span>
    </Button>
  );
}

function ComposerItem({ file, onRemove, onRetry }: { file: ComposerFile; onRemove: () => void; onRetry: () => void }) {
  const { t } = useInterfaceTranslation();
  const size = useSize();
  const failed = file.state === "failed";
  const mark = failed ? (
    <CircleAlertIcon aria-hidden="true" className="size-(--size-icon-lg) text-destructive" />
  ) : file.kind === "photo" ? (
    <ImageIcon aria-hidden="true" className="size-(--size-icon-lg) text-muted-foreground" />
  ) : (
    <FileMark icon={fileIcon(file.name)} className="size-(--size-icon-lg)" />
  );
  const face =
    file.kind === "photo" && file.thumb && !failed ? (
      <span className="relative block size-(--size-mobile-attach-thumb) overflow-hidden rounded-md border border-border bg-secondary">
        <img src={file.thumb} alt="" className={cn("size-full object-cover", file.state !== "ready" && "opacity-(--opacity-dimmed)")} />
        <Band file={file} />
      </span>
    ) : (
      <span className={cn("flex h-(--size-mobile-attach-thumb) max-w-(--size-mobile-attach-chip) items-center gap-sm rounded-lg border bg-card pr-md pl-xs", failed ? "border-destructive" : "border-border")}>
        <span className="relative flex size-(--size-touch-target) shrink-0 items-center justify-center rounded-md bg-secondary">
          {mark}
          <Band file={file} />
        </span>
        <span className="min-w-0 text-left">
          <span className="block truncate text-body text-foreground">{file.name}</span>
          <span className="block font-mono text-caption text-muted-foreground">{failed ? t("mobile.attach.retry") : file.size > 0 ? size(file.size) : " "}</span>
        </span>
      </span>
    );
  return (
    <li className="relative shrink-0 pt-xs pr-xs" data-phone-attached={file.state}>
      {failed ? (
        <Button type="button" variant="ghost" className="h-auto rounded-lg p-none hover:bg-transparent" aria-label={t("mobile.attach.retryNamed", { name: file.name })} onClick={onRetry}>
          {face}
        </Button>
      ) : (
        <span aria-label={file.name} role="img">{face}</span>
      )}
      <RemoveButton name={file.name} onRemove={onRemove} />
    </li>
  );
}

/** The files on the reply bar, in the order picked, scrolled sideways when they pass the width. */
export function ComposerFiles({ files, onRemove, onRetry }: { files: readonly ComposerFile[]; onRemove: (id: string) => void; onRetry: (id: string) => void }) {
  const { t } = useInterfaceTranslation();
  if (files.length === 0) return null;
  return (
    <ul className="-mx-lg flex gap-md overflow-x-auto px-lg pb-xs" aria-label={t("mobile.attach.files")} data-phone-attachments="true">
      {files.map((file) => (
        <ComposerItem key={file.id} file={file} onRemove={() => onRemove(file.id)} onRetry={() => onRetry(file.id)} />
      ))}
    </ul>
  );
}

/** The one line under the files: what was left out and why, or why an upload stopped. */
export function FileNoticeLine({ notice }: { notice: FileNotice | null }) {
  const { t } = useInterfaceTranslation();
  const size = useSize();
  if (!notice) return null;
  const text =
    notice.kind === "too_large"
      ? t("mobile.attach.tooLarge", { name: notice.name, size: size(notice.size) })
      : notice.kind === "unreadable"
        ? t("mobile.attach.unreadable", { name: notice.name })
        : t(`mobile.attach.${notice.kind === "too_many" ? "tooMany" : notice.kind === "too_much" ? "tooMuch" : notice.kind === "daily_limit" ? "dailyLimit" : "failed"}`);
  return (
    <p className={cn("pb-xs text-body", notice.kind === "daily_limit" || notice.kind === "failed" ? "text-destructive" : "text-muted-foreground")} role="status" data-phone-attach-notice={notice.kind}>
      {text}
    </p>
  );
}

function isSent(file: SentFile | NamedFile): file is SentFile {
  return "kind" in file;
}

/**
 * The files over the operator's bubble, right-aligned: the photos this phone sent as their thumbnails, any other file as its mark,
 * name and size; a file read back from the record alone by its name.
 */
export function SentFiles({ files }: { files: readonly (SentFile | NamedFile)[] }) {
  const size = useSize();
  if (files.length === 0) return null;
  return (
    <ul className="flex max-w-5/6 flex-wrap justify-end gap-sm pb-sm" data-phone-sent-files="true">
      {files.map((file, index) => {
        const sent = isSent(file) ? file : null;
        if (sent?.kind === "photo" && sent.thumb) {
          return (
            <li key={`${file.name}-${index}`} className="overflow-hidden rounded-lg border border-border bg-secondary" data-phone-sent-photo={file.name}>
              <img src={sent.thumb} alt={file.name} className="h-(--size-mobile-sent-photo) w-auto max-w-(--size-mobile-attach-chip) object-cover" />
            </li>
          );
        }
        return (
          <li key={`${file.name}-${index}`} className="flex min-w-0 max-w-full items-center gap-sm rounded-xl bg-secondary px-lg py-sm" data-phone-sent-file={file.name}>
            {sent?.kind === "photo" ? <ImageIcon aria-hidden="true" className="size-(--size-icon-lg) shrink-0 text-muted-foreground" /> : <FileMark icon={fileIcon(file.name)} className="size-(--size-icon-lg)" />}
            <span className="min-w-0 truncate text-body text-foreground">{file.name}</span>
            {sent && sent.size > 0 ? <span className="shrink-0 font-mono text-body text-muted-foreground">{size(sent.size)}</span> : null}
          </li>
        );
      })}
    </ul>
  );
}
