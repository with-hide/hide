// One of a job's 결과물 in Desk's 보기 column (PRD session-docs-and-assistant
// D-40, D-65, B30, B31): read-only, drawn with Hide's own viewers from the
// file the session's View showed, read through hided under that View's
// checkout, so nothing joins the checkout's Files list. A page shows the
// picture its View last drew and its address (the live page stays in its
// checkout until Desk has a place for one); a pull request is its mark and
// its link. Every output can be opened where its session opened it.

import { ExternalLinkIcon, GlobeIcon } from "lucide-react";
import { useEffect, useMemo, type ReactNode } from "react";
import type { Actions } from "./actions";
import { addressShown, browserStillOf } from "./browserViews";
import { PrMark } from "./components/pr-mark";
import { Button } from "./components/ui/button";
import type { DeskOutput } from "./desk";
import type { FileSource } from "./fileBytes";
import { useInterfaceTranslation } from "./i18n/client";
import { MarkdownText } from "./MarkdownText";
import { localDeviceId } from "./snapshot";
import { useShellStore } from "./store";
import { ImageBody, PdfBody, VideoBody, ViewerNotice } from "./viewers/FileViewer";
import { useFileBytesFrom } from "./viewers/useFileBytes";
import { isVideoPath } from "./viewers/video";

const PDF = /\.pdf$/i;
const IMAGE = /\.(png|jpe?g|gif|webp|bmp|avif|svg)$/i;

/** The checkout an output's file is read under: none for this machine, its device and root for a device's. */
function useOutputSource(output: DeskOutput): FileSource {
  const node = useShellStore((s) => localDeviceId(s.rest));
  return output.device_id && output.checkout && output.device_id !== node ? { device: output.device_id, root: output.checkout } : null;
}

export function DeskOutputView({ output, viewKey, actions }: { output: DeskOutput; viewKey: string; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  // Shown is seen: the View's mark where the session opened it goes off (B31).
  useEffect(() => {
    actions.deskOutputSeen(output);
  }, [actions, output]);
  const openAt = output.device_id && output.checkout && output.kind !== "pr" ? () => actions.openViewAt(output.device_id!, output.checkout!, output.id) : null;
  let body: ReactNode;
  if (output.kind === "pr") body = <PullOutput output={output} actions={actions} />;
  else if (output.path) body = <FileOutput path={output.path} output={output} viewKey={viewKey} />;
  else if (output.url) body = <PageOutput output={output} url={output.url} />;
  else body = <ViewerNotice state="unreadable" reason={t("desk.outputs.unreadable")} />;
  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col" data-desk-output-view={output.kind}>
      {openAt ? (
        <div className="flex shrink-0 items-center justify-end border-b border-border px-xs py-xxs">
          <Button variant="text" size="inline" className="gap-xxs text-caption text-muted-foreground" onClick={openAt} data-desk-output-open-at="true">
            {t("desk.outputs.openWhere")}
            <ExternalLinkIcon aria-hidden="true" className="size-(--size-icon-sm)" />
          </Button>
        </div>
      ) : null}
      {body}
    </div>
  );
}

/** A file the session showed: the image, PDF or video viewer, or a document's text read as Markdown. */
function FileOutput({ path, output, viewKey }: { path: string; output: DeskOutput; viewKey: string }) {
  const { t } = useInterfaceTranslation();
  const state = useFileBytesFrom(path, useOutputSource(output));
  const displayId = `desk:${viewKey}`;
  if (isVideoPath(path)) return <VideoBody path={path} state={state} />;
  if (IMAGE.test(path)) return <ImageBody path={path} state={state} displayId={displayId} />;
  if (PDF.test(path)) return <PdfBody path={path} state={state} displayId={displayId} />;
  if (state.status === "failed") return <ViewerNotice state="failed" reason={t("desk.outputs.unavailable", { reason: state.reason })} />;
  if (state.status === "loading") return <ViewerNotice state="loading" reason={t("desk.outputs.reading")} />;
  return <DocumentText bytes={state.bytes} />;
}

/** A document's words, read-only, as Desk's own answers draw Markdown. */
function DocumentText({ bytes }: { bytes: Uint8Array }) {
  const text = useMemo(() => new TextDecoder().decode(bytes), [bytes]);
  return (
    <div className="min-h-0 flex-1 overflow-auto px-lg py-md" data-desk-output-document="true">
      <div className="mx-auto max-w-(--size-editor-measure)"><MarkdownText text={text} /></div>
    </div>
  );
}

/** A page: the picture its View last drew, else its globe, and its address. */
function PageOutput({ output, url }: { output: DeskOutput; url: string }) {
  const still = browserStillOf(output.id, url);
  return (
    <div className="flex min-h-0 flex-1 flex-col items-center gap-sm overflow-auto p-md" data-desk-output-page="true">
      {still ? <img src={still} alt={output.label} className="max-w-full rounded-sm border border-border object-contain" data-desk-output-still="true" /> : <GlobeIcon aria-hidden="true" className="mt-xl size-(--size-icon-lg) text-muted-foreground" />}
      <p className="max-w-full truncate text-caption text-muted-foreground" title={url}>{addressShown(url)}</p>
    </div>
  );
}

/** A pull request: its mark, its title and its page. */
function PullOutput({ output, actions }: { output: DeskOutput; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  return (
    <div className="flex flex-col gap-sm p-md" data-desk-output-pr="true">
      <p className="flex min-w-0 items-center gap-xs text-body text-foreground">
        {output.pr_state ? <PrMark state={output.pr_state} compact /> : null}
        <span className="min-w-0 truncate">{output.label}</span>
      </p>
      {output.url ? (
        <Button variant="text" size="inline" className="w-fit gap-xxs text-caption text-muted-foreground" onClick={() => actions.openLink(output.url!, true)}>
          {t("desk.outputs.openPr")}
          <ExternalLinkIcon aria-hidden="true" className="size-(--size-icon-sm)" />
        </Button>
      ) : null}
    </div>
  );
}
