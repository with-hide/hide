import { useEffect, useRef, useState } from "react";
import type { Actions } from "./actions";
import { CommandGroup } from "./components/ui/command";
import { readFailureText } from "./fileFailure";
import { holdsCommandKey } from "./host";
import { useInterfaceTranslation } from "./i18n/client";
import { composingKey } from "./keys";
import { deskFileScope, lastNamedWorkspace, projectFileRows, type DeskFileHit, type ProjectFileRow } from "./deskPalette";
import { PaletteFileRow, PaletteShell } from "./Palette";
import { fieldLabel } from "./shortcutLabels";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";

// ⌘P while Desk is in front (PRD desk-2-ui D-23, B37, B38): the app's file
// palette, over the checkout of the session open beside Desk, or over every
// project at once when none is open. ↵ opens the file in the tab panel and
// ⌘↵ keeps it beside what is open; the Explorer column is left as it was.

export function DeskFilePalette({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const session = useUiStore((s) => s.deskRoom.session);
  const rest = useShellStore((s) => s.rest);
  const scope = deskFileScope(rest, session);
  const device = scope.kind === "checkout" ? scope.device : null;
  const root = scope.kind === "checkout" ? scope.root : null;
  const checkoutAnswer = useShellStore((s) => (s.fileIndex && s.fileIndex.device_id === device && s.fileIndex.root_path === root ? s.fileIndex : null));
  const allAnswer = useShellStore((s) => s.fileIndexAll);
  const items = useShellStore((s) => s.desk?.conversation.items);
  const [query, setQuery] = useState("");
  const [highlighted, setHighlighted] = useState("");
  const timer = useRef<number | undefined>(undefined);

  const answer = scope.kind === "checkout" ? checkoutAnswer : allAnswer;
  const indexing = answer?.indexing ?? false;
  const unavailable = scope.kind === "checkout" ? (checkoutAnswer?.unavailable ?? null) : null;
  const ask = () => (scope.kind === "checkout" ? actions.requestFileIndex(scope.root, query, scope.device) : actions.requestFileIndexAll(query));
  // The query is debounced to one request, and asked again while a walk still runs, as the app's palette does.
  useEffect(() => {
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(ask, 120);
    return () => window.clearTimeout(timer.current);
  }, [device, root, query, actions]);
  useEffect(() => {
    if (!indexing) return undefined;
    const poll = window.setTimeout(ask, 250);
    return () => window.clearTimeout(poll);
  }, [device, root, query, indexing, answer, actions]);

  const hits: DeskFileHit[] = scope.kind === "checkout" ? (checkoutAnswer?.files ?? []).map((file) => ({ ...file, device_id: scope.device, root_path: scope.root })) : (allAnswer?.files ?? []);
  const groups = scope.kind === "all" ? projectFileRows(rest, hits, lastNamedWorkspace(rest, items ?? [])) : null;
  const rows: ProjectFileRow[] | null = groups ? [...groups.named, ...groups.other] : null;
  const paths = rows ? rows.map((row) => row.hit.path) : hits.map((hit) => hit.path);
  const current = hits.find((hit) => hit.path === highlighted && paths.includes(hit.path)) ?? hits.find((hit) => hit.path === paths[0]);

  const open = (hit: DeskFileHit, keep: boolean) => actions.openDeskIndexEntry(hit, keep);
  const row = (hit: DeskFileHit, title: React.ReactNode) => (
    <PaletteFileRow key={`${hit.device_id}:${hit.root_path}:${hit.path}`} path={hit.path} relativePath={hit.relative_path} title={title} onSelect={() => open(hit, false)} />
  );
  const projectRow = (one: ProjectFileRow) =>
    row(one.hit, (
      <>
        <span className="text-muted-foreground">{one.project} › </span>
        {one.name}
        {one.folder ? <span className="ml-xs text-caption font-normal text-muted-foreground">{one.folder}</span> : null}
      </>
    ));

  return (
    <PaletteShell
      label={t("documents.openFile")}
      placeholder={scope.kind === "all" ? t("documents.searchAllProjectFiles") : t("documents.searchFiles")}
      query={query}
      onQuery={setQuery}
      value={current?.path ?? ""}
      onValue={setHighlighted}
      onKeyDown={(event) => {
        // ⌘↵ (Ctrl+Enter off macOS) keeps the highlighted file as a tab of its own.
        if (event.key !== "Enter" || !holdsCommandKey(event) || composingKey(event) || !current) return;
        event.preventDefault();
        event.stopPropagation();
        open(current, true);
      }}
      footer={
        <span className="flex items-center gap-md">
          <span data-palette-hint="beside">{t("documents.openBeside", { shortcut: fieldLabel("Enter") })}</span>
          {answer?.truncated ? <span>{t("documents.indexTruncated", { limit: 50000 })}</span> : null}
        </span>
      }
    >
      {unavailable ? (
        <div className="px-md py-sm text-caption text-muted-foreground" role="alert" data-palette-state="unavailable">
          {t("documents.filesUnavailable", { reason: readFailureText(unavailable, t) })}
        </div>
      ) : indexing && paths.length === 0 ? (
        <div className="px-md py-sm text-caption text-muted-foreground" data-palette-state="indexing">
          {t("documents.indexing")}
        </div>
      ) : paths.length === 0 ? (
        <div className="px-md py-sm text-caption text-muted-foreground" data-palette-state="empty">
          {query ? t("documents.noMatchingFiles") : scope.kind === "all" ? t("documents.typeToSearchAll") : t("documents.typeToSearch")}
        </div>
      ) : groups ? (
        groups.named.length > 0 ? (
          <>
            <CommandGroup heading={`${groups.named[0]?.project ?? ""} · ${t("documents.namedLastInDesk")}`} data-palette-group="named">
              {groups.named.map(projectRow)}
            </CommandGroup>
            {groups.other.length > 0 ? <CommandGroup heading={t("documents.otherProjects")} data-palette-group="other">{groups.other.map(projectRow)}</CommandGroup> : null}
          </>
        ) : (
          groups.other.map(projectRow)
        )
      ) : (
        hits.map((hit) => row(hit, hit.relative_path))
      )}
    </PaletteShell>
  );
}
