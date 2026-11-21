// The window after an update and the update history (PRD whats-new), drawn
// from the releases file built into the app (`web/src/releases.json`), so a
// draft the release skill adds there is previewed exactly as the app
// will draw it (B27). `version` picks the version (the newest when absent);
// `state` picks what is drawn:
//   dialog       the window for that version
//   dialog-more  the same with its changes and fixes open
//   dialog-skip  the window after skipping from `from` (the third newest when absent)
//   history      the history at that version, the version in use
//   history-empty  the history of a version the file has nothing up to
import { useLayoutEffect } from "react";
import { WhatsNewDialogBody, UpdateHistoryBody, historyRows } from "../components/whats-new";
import { Dialog, DialogContent } from "../components/ui/dialog";
import { TooltipProvider } from "../components/ui/tooltip";
import { requireInterfaceLanguage } from "../i18n/locale";
import { useFollowInterfaceLanguage } from "../i18n/translator";
import { historyVersions, parseReleaseFeed, whatsNewOffer } from "../releases";
import releases from "../releases.json";

export function WhatsNewScene({ params }: { params: URLSearchParams }) {
  const language = requireInterfaceLanguage(params.get("lang") ?? "ko");
  useFollowInterfaceLanguage(language);
  const theme = params.get("theme") === "light" ? "light" : "dark";
  useLayoutEffect(() => {
    document.documentElement.classList.toggle("dark", theme === "dark");
    document.documentElement.classList.toggle("light", theme === "light");
  }, [theme]);
  const { feed } = parseReleaseFeed(releases as unknown);
  const newest = feed.versions.find((row) => row.highlights.length > 0)?.version ?? feed.versions[0]?.version ?? "0.0.0";
  const version = params.get("version") ?? newest;
  const state = params.get("state") ?? "dialog";
  const noop = () => undefined;
  let body = null;
  if (state.startsWith("dialog")) {
    const from = params.get("from") ?? feed.versions[feed.versions.findIndex((row) => row.version === version) + 2]?.version ?? null;
    const result = whatsNewOffer(feed, version, { shown: state === "dialog-skip" ? from : (feed.versions[feed.versions.findIndex((row) => row.version === version) + 1]?.version ?? null), seen: null, opened: [] });
    body = result.kind === "show" ? (
      <Dialog open>
        <WhatsNewDialogBody offer={result.offer} language={language} auto={false} open={state === "dialog-more"} onTry={noop} onHistory={noop} onClose={noop} />
      </Dialog>
    ) : (
      <p className="p-lg text-body text-muted-foreground" data-whats-new-scene-empty>{`${version}: ${result.kind}`}</p>
    );
  } else {
    const app = version;
    const record = { shown: app, seen: feed.versions[feed.versions.findIndex((row) => row.version === app) + 1]?.version ?? null, opened: [] };
    const rows = state === "history-empty" ? [] : historyRows(historyVersions(feed, app), record);
    body = (
      <Dialog open>
        <DialogContent className="h-(--size-settings-sheet-h) w-(--size-settings-sheet-w) p-0" showCloseButton>
          <UpdateHistoryBody
            rows={rows}
            appVersion={app}
            chosen={app}
            language={language}
            onChoose={noop}
            onTry={noop}
            onLink={noop}
          />
        </DialogContent>
      </Dialog>
    );
  }
  return (
    <TooltipProvider>
      <div className="h-full bg-background" data-gallery-scene="whats-new">{body}</div>
    </TooltipProvider>
  );
}
