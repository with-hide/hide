import type { ReactNode } from "react";
import { readFailureText } from "./fileFailure";
import { useEffect, useRef, useState } from "react";
import type { Actions } from "./actions";
import { FileMark } from "./components/file-mark";
import { Command, CommandDialog, CommandInput, CommandItem, CommandList } from "./components/ui/command";
import { Kbd } from "./components/ui/kbd";
import { changedFiles } from "./newTab";
import { fileIcon } from "./fileIcons";
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

// The file and diff palettes (PRD B12, B13) on the System command palette: a
// query field, a list cmdk's own arrow keys and Enter walk, and Escape closes
// through the shell's one layer owner. ⌘P lists what hided's index ranked for
// the typed query, and ⌘↵ on a row opens it beside the active View area (S7
// B4, PRD cmdk-navigation B23). The screens already rank and filter their own
// entries, so `shouldFilter` stays off and cmdk is used only for the list's
// selection and keyboard behavior. ⌘K is its own surface (`SearchPalette`).

export function Palette({ actions }: { actions: Actions }) {
  const overlay = useUiStore((s) => s.overlay);
  if (overlay === "file_palette") return <FilePalette actions={actions} />;
  if (overlay === "diff_palette") return <DiffPalette actions={actions} />;
  if (overlay === "search") return <SearchPalette actions={actions} />;
  return null;
}

function PaletteShell({
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
function PaletteRow({ icon, title }: { icon?: ReactNode; title: string }) {
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

function FilePalette({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  // The checkout in front on the device in front, as the Explorer shows it.
  const device = useShellStore((s) => explorerContext(s.rest).device);
  const root = useShellStore((s) => explorerContext(s.rest).checkout?.path ?? null);
  // An answer for another device or checkout is not this list.
  const fileIndex = useShellStore((s) =>
    s.fileIndex && s.fileIndex.device_id === device && s.fileIndex.root_path === root ? s.fileIndex : null,
  );
  const [query, setQuery] = useState("");
  const [highlighted, setHighlighted] = useState("");
  // Why ⌘↵ opened nothing, said in the footer where its hint stands.
  const [besideReason, setBesideReason] = useState<string | null>(null);
  const timer = useRef<number | undefined>(undefined);

  useEffect(() => {
    if (!root) return undefined;
    window.clearTimeout(timer.current);
    // A palette keystroke is not one event per character: the index answer is
    // what the list draws, so the query is debounced to one request.
    timer.current = window.setTimeout(() => actions.requestFileIndex(root, query, device), 120);
    return () => window.clearTimeout(timer.current);
  }, [root, query, device, actions]);

  // The first query for a checkout starts the walk and answers `indexing`; ask
  // again while it says so, so the list fills without another keystroke.
  useEffect(() => {
    if (!root || !fileIndex?.indexing) return undefined;
    const poll = window.setTimeout(() => actions.requestFileIndex(root, query, device), 250);
    return () => window.clearTimeout(poll);
  }, [root, query, device, fileIndex, actions]);

  const entries = fileIndex?.files ?? [];
  // The row Enter would choose: the one the pointer or arrows moved to, else the first.
  const current = entries.find((entry) => entry.path === highlighted) ?? entries[0];

  return (
    <PaletteShell
      label={t("documents.openFile")}
      placeholder={t("documents.searchFiles")}
      query={query}
      onQuery={(next) => {
        setBesideReason(null);
        setQuery(next);
      }}
      value={current?.path ?? ""}
      onValue={setHighlighted}
      onKeyDown={(event) => {
        // ⌘↵ (Ctrl+Enter off macOS) opens the highlighted file beside the active View area (S7 B4).
        if (event.key !== "Enter" || !holdsCommandKey(event) || event.nativeEvent.isComposing || !current) return;
        event.preventDefault();
        event.stopPropagation();
        const reason = besideUnavailable(workspaceViewOf(useShellStore.getState().rest)?.layout, drawnViews());
        if (reason) return setBesideReason(reason);
        actions.openIndexEntryBeside(current.path);
      }}
      footer={
        <span className="flex items-center gap-md">
          <span data-palette-hint="beside">{besideReason ?? t("documents.openBeside", { shortcut: fieldLabel("Enter") })}</span>
          {fileIndex?.truncated ? <span>{t("documents.indexTruncated", { limit: 50000 })}</span> : null}
        </span>
      }
    >
      {fileIndex?.unavailable ? (
        <div className="px-md py-sm text-caption text-muted-foreground" role="alert" data-palette-state="unavailable">
          {t("documents.filesUnavailable", { reason: readFailureText(fileIndex.unavailable, t) })}
        </div>
      ) : fileIndex?.indexing && entries.length === 0 ? (
        <div className="px-md py-sm text-caption text-muted-foreground" data-palette-state="indexing">
          {t("documents.indexing")}
        </div>
      ) : entries.length === 0 ? (
        <div className="px-md py-sm text-caption text-muted-foreground" data-palette-state="empty">
          {query ? t("documents.noMatchingFiles") : t("documents.typeToSearch")}
        </div>
      ) : (
        entries.map((entry) => (
          <CommandItem key={entry.path} asChild value={entry.path} onSelect={() => actions.openIndexEntry(entry.path)}>
            <button type="button" data-palette-row={entry.path} className="group/palette-row w-full text-left">
              <PaletteRow
                icon={<FileMark icon={fileIcon(entry.relative_path)} />}
                title={entry.relative_path}
              />
            </button>
          </CommandItem>
        ))
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
