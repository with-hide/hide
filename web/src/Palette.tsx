import type { ReactNode } from "react";
import { readFailureText } from "./fileFailure";
import { useEffect, useRef, useState } from "react";
import type { Actions } from "./actions";
import { FileMark } from "./components/file-mark";
import { Command, CommandDialog, CommandGroup, CommandInput, CommandItem, CommandList } from "./components/ui/command";
import { Kbd } from "./components/ui/kbd";
import { changedFiles } from "./newTab";
import { fileIcon } from "./fileIcons";
import { deskFileScope, lastNamedWorkspace, preferredRoot, projectFileRows, type DeskFileHit, type DeskFileScope, type ProjectFileRow } from "./deskPalette";
import { SearchPalette } from "./SearchPalette";
import { explorerContext } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { drawnViews } from "./viewFocus";
import { besideUnavailable } from "./viewLayout";
import { workspaceViewOf } from "./workspace";
import { holdsCommandKey } from "./host";
import { fieldLabel } from "./shortcutLabels";
import { useInterfaceTranslation } from "./i18n/client";
import { composingKey } from "./keys";

// The file and diff palettes (PRD B12, B13) on the System command palette: a
// query field, a list cmdk's own arrow keys and Enter walk, and Escape closes
// through the shell's one layer owner. ⌘P lists what hided's index ranked for
// the typed query, and ⌘↵ on a row opens it beside the active View area (S7
// B4, PRD cmdk-navigation B23). The screens already rank and filter their own
// entries, so `shouldFilter` stays off and cmdk is used only for the list's
// selection and keyboard behavior. ⌘K is its own surface (`SearchPalette`).

export function Palette({ actions }: { actions: Actions }) {
  const overlay = useUiStore((s) => s.overlay);
  const onDesk = useUiStore((s) => s.screen?.kind === "desk");
  // On Desk ⌘P searches the open session's checkout or every project and opens in the tab panel (PRD desk-2-ui B37, B38); every other screen keeps its own.
  if (overlay === "file_palette") return <FilePalette actions={actions} desk={onDesk} />;
  if (overlay === "diff_palette") return <DiffPalette actions={actions} />;
  if (overlay === "search") return <SearchPalette actions={actions} />;
  return null;
}

export function PaletteShell({
  label,
  placeholder,
  query,
  onQuery,
  footer,
  children,
  value,
  onValue,
  onKeyDown,
}: {
  label: string;
  placeholder: string;
  query: string;
  onQuery: (query: string) => void;
  footer: ReactNode;
  children: ReactNode;
  /** The highlighted row, when the palette needs to know it. */
  value?: string;
  onValue?: (value: string) => void;
  onKeyDown?: (event: React.KeyboardEvent) => void;
}) {
  const close = useUiStore((s) => s.closeOverlay);
  return (
    <CommandDialog open title={label} description={placeholder} onOpenChange={(open) => { if (!open) close(); }}>
      <Command shouldFilter={false} loop label={label} data-palette={label} value={value} onValueChange={onValue} onKeyDown={onKeyDown}>
        <CommandInput value={query} placeholder={placeholder} data-palette-input="true" onValueChange={onQuery} trailing={<Kbd data-palette-esc="true">Esc</Kbd>} />
        <CommandList data-palette-list="true">{children}</CommandList>
        {footer ? (
          <div className="border-t border-border px-md py-xxs text-caption text-muted-foreground" data-palette-footer="true">
            {footer}
          </div>
        ) : null}
      </Command>
    </CommandDialog>
  );
}

/** One palette row: its icon or mark, the title, and ↵ while it is the row Enter would choose. */
export function PaletteRow({ icon, title }: { icon?: ReactNode; title: ReactNode }) {
  return (
    <>
      {icon}
      <span className="flex min-w-0 flex-1 flex-col">
        <span className="min-w-0 truncate font-medium">{title}</span>
      </span>
      <span aria-hidden="true" data-palette-enter="true" className="invisible shrink-0 text-caption text-muted-foreground group-data-[selected=true]/palette-row:visible">
        ↵
      </span>
    </>
  );
}

/** One file of a file palette: its mark and `title`, which ↵ opens through `onSelect`. */
export function PaletteFileRow({ id, path, relativePath, title, onSelect }: { id: string; path: string; relativePath: string; title: ReactNode; onSelect: () => void }) {
  return (
    <CommandItem asChild value={id} onSelect={onSelect}>
      <button type="button" data-palette-row={path} className="group/palette-row w-full text-left">
        <PaletteRow icon={<FileMark icon={fileIcon(relativePath)} />} title={title} />
      </button>
    </CommandItem>
  );
}

/** What a file palette lists and where a pick goes: a checkout's index, or on Desk every project's (PRD desk-2-ui B37, B38). */
type FileHit = DeskFileHit;
const hitKey = (hit: FileHit) => `${hit.device_id}:${hit.root_path}:${hit.path}`;

/**
 * ⌘P: the file palette over hided's index of the checkout in front. On Desk
 * (`desk`) it lists the checkout of the session open beside Desk, or every
 * project at once, and a pick opens in the tab panel; the list, the debounce,
 * the polling and the keys are the app's own (PRD desk-2-ui D-23).
 */
function FilePalette({ actions, desk }: { actions: Actions; desk: boolean }) {
  const { t } = useInterfaceTranslation();
  const rest = useShellStore((s) => s.rest);
  const session = useUiStore((s) => s.deskRoom.session);
  const items = useShellStore((s) => s.desk?.conversation.items);
  // The checkout in front on the device in front, as the Explorer shows it; on Desk the session's, else every project.
  const frontDevice = useShellStore((s) => explorerContext(s.rest).device);
  const frontRoot = useShellStore((s) => explorerContext(s.rest).checkout?.path ?? null);
  const scope: DeskFileScope | null = desk ? deskFileScope(rest, session) : frontRoot ? { kind: "checkout", device: frontDevice, root: frontRoot } : null;
  const device = scope?.kind === "checkout" ? scope.device : null;
  const root = scope?.kind === "checkout" ? scope.root : null;
  const named = desk && scope?.kind === "all" ? lastNamedWorkspace(rest, items ?? []) : null;
  const prefer = preferredRoot(rest, named);
  // An answer for another device or checkout is not this list, and one left from an earlier open is not this palette's.
  const checkoutAnswer = useShellStore((s) => (s.fileIndex && s.fileIndex.device_id === device && s.fileIndex.root_path === root ? s.fileIndex : null));
  const allAnswer = useShellStore((s) => s.fileIndexAll);
  // On Desk an answer left from an earlier open is not this palette's; the app's own palette keeps showing the checkout's cached list.
  const left = useRef({ checkout: checkoutAnswer, all: allAnswer });
  const fresh = <T,>(held: T | null, was: T | null): T | null => (desk && held === was ? null : held);
  const allRead = scope?.kind === "all" ? fresh(allAnswer, left.current.all) : null;
  const answer = scope?.kind === "all" ? allRead : fresh(checkoutAnswer, left.current.checkout);
  const [query, setQuery] = useState("");
  const [highlighted, setHighlighted] = useState("");
  // Why ⌘↵ opened nothing, said in the footer where its hint stands.
  const [besideReason, setBesideReason] = useState<string | null>(null);
  const timer = useRef<number | undefined>(undefined);

  const ask = () => {
    if (scope?.kind === "checkout") actions.requestFileIndex(scope.root, query, scope.device);
    else if (scope?.kind === "all") actions.requestFileIndexAll(query, prefer ?? undefined);
  };
  useEffect(() => {
    if (!scope) return undefined;
    window.clearTimeout(timer.current);
    // A palette keystroke is not one event per character: the index answer is
    // what the list draws, so the query is debounced to one request.
    timer.current = window.setTimeout(ask, 120);
    return () => window.clearTimeout(timer.current);
  }, [scope?.kind, root, query, device, prefer?.root_path, actions]);

  // The first query for a checkout starts the walk and answers `indexing`; ask
  // again while it says so, so the list fills without another keystroke.
  const indexing = answer?.indexing ?? false;
  useEffect(() => {
    if (!scope || !indexing) return undefined;
    const poll = window.setTimeout(ask, 250);
    return () => window.clearTimeout(poll);
  }, [scope?.kind, root, query, device, prefer?.root_path, answer, actions]);

  const hits: FileHit[] = !answer ? [] : scope?.kind === "all" ? (allRead?.files ?? []) : (checkoutAnswer?.files ?? []).map((file) => ({ ...file, device_id: scope?.kind === "checkout" ? scope.device : "", root_path: scope?.kind === "checkout" ? scope.root : "" }));
  const groups = scope?.kind === "all" ? projectFileRows(rest, hits, named) : null;
  const listed = groups ? [...groups.named, ...groups.other].map((row) => row.hit) : hits;
  // The row Enter would choose: the one the pointer or arrows moved to, else the first.
  const current = listed.find((hit) => hitKey(hit) === highlighted) ?? listed[0];
  const unavailable = scope?.kind === "checkout" ? (checkoutAnswer?.unavailable ?? null) : null;
  const skipped = allRead?.unavailable.length ?? 0;

  const open = (hit: FileHit, beside: boolean) => {
    if (desk) return actions.openDeskIndexEntry(hit, beside);
    return beside ? actions.openIndexEntryBeside(hit.path) : actions.openIndexEntry(hit.path);
  };
  const row = (hit: FileHit, title: ReactNode) => (
    <PaletteFileRow key={hitKey(hit)} id={hitKey(hit)} path={hit.path} relativePath={hit.relative_path} title={title} onSelect={() => open(hit, false)} />
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
      placeholder={scope?.kind === "all" ? t("documents.searchAllProjectFiles") : t("documents.searchFiles")}
      query={query}
      onQuery={(next) => {
        setBesideReason(null);
        setQuery(next);
      }}
      value={current ? hitKey(current) : ""}
      onValue={setHighlighted}
      onKeyDown={(event) => {
        // ⌘↵ (Ctrl+Enter off macOS) opens the highlighted file beside the active View area (S7 B4); on Desk it keeps it as a tab of the tab panel.
        if (event.key !== "Enter" || !holdsCommandKey(event) || composingKey(event) || !current) return;
        event.preventDefault();
        event.stopPropagation();
        if (!desk) {
          const reason = besideUnavailable(workspaceViewOf(useShellStore.getState().rest)?.layout, drawnViews());
          if (reason) return setBesideReason(reason);
        }
        open(current, true);
      }}
      footer={
        <span className="flex items-center gap-md">
          <span data-palette-hint="beside">{besideReason ?? t(desk ? "documents.keepTab" : "documents.openBeside", { shortcut: fieldLabel("Enter") })}</span>
          {(scope?.kind === "all" ? allRead?.index_truncated : checkoutAnswer?.truncated) ? <span>{t("documents.indexTruncated", { limit: 50000 })}</span> : null}
          {allRead?.more ? <span data-palette-state="more">{t("documents.moreMatches", { count: 80 })}</span> : null}
          {skipped > 0 ? <span data-palette-state="skipped">{t("documents.projectsUnavailable", { count: skipped })}</span> : null}
        </span>
      }
    >
      {unavailable ? (
        <div className="px-md py-sm text-caption text-muted-foreground" role="alert" data-palette-state="unavailable">
          {t("documents.filesUnavailable", { reason: readFailureText(unavailable, t) })}
        </div>
      ) : indexing && listed.length === 0 ? (
        <div className="px-md py-sm text-caption text-muted-foreground" data-palette-state="indexing">
          {t("documents.indexing")}
        </div>
      ) : listed.length === 0 ? (
        <div className="px-md py-sm text-caption text-muted-foreground" data-palette-state="empty">
          {query ? t("documents.noMatchingFiles") : scope?.kind === "all" ? t("documents.typeToSearchAll") : t("documents.typeToSearch")}
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
        listed.map((hit) => row(hit, hit.relative_path))
      )}
    </PaletteShell>
  );
}

function DiffPalette({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const root = useShellStore((s) => s.rest?.navigator?.changes_root_path ?? null);
  const changes = useShellStore((s) => s.changes);
  const [query, setQuery] = useState("");
  const entries = changedFiles(changes, root, query);
  return (
    <PaletteShell label={t("documents.openDiff")} placeholder={t("documents.searchChangedFiles")} query={query} onQuery={setQuery} footer="">
      {entries.length ? entries.map((entry) => (
        <CommandItem key={entry.path} asChild value={entry.path} onSelect={() => {
          useUiStore.getState().closeOverlay();
          actions.selectChange(entry.path, false, false);
        }}>
          <button type="button" data-palette-row={entry.path} className="group/palette-row w-full text-left">
            <PaletteRow title={entry.relative_path} />
          </button>
        </CommandItem>
      )) : <div className="px-md py-sm text-caption text-muted-foreground">{t("documents.noMatchingChangedFiles")}</div>}
    </PaletteShell>
  );
}
