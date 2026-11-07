// Add a project's folder list (PRD device-add-project D-03 to D-08): one
// machine's folders under its home, read from that machine's node through
// hided (`home_folder_list`), shown inside the dialog wherever the system's
// own folder window cannot show them: a connected device, the core's machine
// seen from a node's window, and a browser tab. It starts in the folder it
// last showed on that machine, which the core keeps (`add_project_folder_set`),
// else home. One field filters the folder shown, moves by a `~/` path with
// Tab completion, and takes a pasted absolute or Windows path, which the node
// places against its home; a path outside home is not listed and is added
// through Add a project's one question. The same list picks a parent folder
// for Clone from URL and Create new project (`mode` `pick`).

import { ArrowLeftIcon, ChevronRightIcon, FileIcon, FolderIcon, GitBranchIcon } from "lucide-react";
import { useEffect, useId, useMemo, useRef, useState, type ReactNode } from "react";
import type { Actions } from "./actions";
import { alreadyRegistered, childPath, filterEntries, folderCrumbs, TYPED_ASK_DELAY_MS, typedPlace, type TypedPlace } from "./addProject";
import { DeviceChip } from "./components/device-chip";
import { Note, Status } from "./components/settings-rows";
import { Badge } from "./components/ui/badge";
import { Button } from "./components/ui/button";
import { DialogBody, DialogHeader, DialogTitle } from "./components/ui/dialog";
import { Input } from "./components/ui/input";
import { Kbd } from "./components/ui/kbd";
import { useInterfaceTranslation } from "./i18n/client";
import { composingKey } from "./keys";
import type { WorkspaceRegistration } from "./snapshot";
import { homeFolderKey, useShellStore, type HomeEntry, type HomeFolderAnswer } from "./store";


/** What moved the list: its opening, the operator's click or key, or a typed path. */
type Via = "start" | "click" | "typed";

/** The folder asked for last, and the answer count when it was asked, so an older answer for it is not this one. */
type Asked = { request: string; via: Via; after: number };

/** The refusals that say the folder asked for is not there, which a remembered start folder falls back to home for. */
const GONE = new Set(["not_found", "not_a_directory", "outside_home", "invalid_path"]);

export type FolderBrowserProps = {
  actions: Actions;
  /** The machine listed, and its name as the dialog shows it. */
  device: string;
  deviceLabel: string;
  /** The machine is the one this window runs on (the chip's laptop mark). */
  local: boolean;
  /** Where a WSL device shows the PC's drives, so a pasted `C:\` path is spelled as the device's own. */
  drives: string | null;
  /** A Windows device, whose field says it takes a `C:\` path. */
  windows: boolean;
  /** `add` registers the folder; `pick` hands a parent folder back to Clone or Create. */
  mode: "add" | "pick";
  /** The folder the list opens in: `~`, `~/...` or the machine's own absolute path. */
  start: string;
  /** The folder the core remembers for this machine; a list that shows another tells the core. */
  remembered: string | null;
  registrations: readonly WorkspaceRegistration[];
  /** The machine's connection is down (PRD device-add-project B31). */
  offline: boolean;
  /** A registration is on its way; adding again waits. */
  busy: boolean;
  onAdd: (path: string) => void;
  onPick: (label: string) => void;
  onBack: () => void;
  /** What the registration says, under the footer. */
  status?: ReactNode;
};

export function FolderBrowser(props: FolderBrowserProps) {
  const { actions, device, deviceLabel, local, drives, windows, mode, start, remembered, registrations, offline, busy, onAdd, onPick, onBack, status } = props;
  const { t } = useInterfaceTranslation();
  const answers = useShellStore((s) => s.homeFolders);
  const [asked, setAsked] = useState<Asked>(() => ({ request: start, via: "start", after: useShellStore.getState().homeFolderSeq }));
  const [shown, setShown] = useState<HomeFolderAnswer | null>(null);
  const [text, setText] = useState("");
  const [selected, setSelected] = useState<string | null>(null);
  const told = useRef(remembered);
  const fieldRef = useRef<HTMLInputElement>(null);
  const listId = useId();

  const ask = (request: string, via: Via) => {
    setAsked({ request, via, after: useShellStore.getState().homeFolderSeq });
    actions.listHomeFolder(device, request);
  };

  // The opening asks for its start folder once.
  useEffect(() => {
    actions.listHomeFolder(device, start);
    fieldRef.current?.focus();
    // Only on mount: every later move asks for itself.
  }, []);

  const fresh = answers[homeFolderKey(device, asked.request)];
  const answer = fresh && fresh.seq > asked.after ? fresh : null;

  // An answer for the folder asked for: a listing becomes the list, a remembered folder that is gone opens home.
  useEffect(() => {
    if (!answer) return;
    // A typed ask the field no longer makes (cleared or changed before it was answered) moves nothing.
    if (asked.via === "typed" && typedPlaceRequest(typedPlace(text, drives, windows)) !== asked.request) return;
    if (answer.listing) {
      setShown(answer);
      setSelected(null);
      if (asked.via === "typed" && typedPlace(text, drives, windows).kind === "path") setText(`${answer.listing.label}/`);
      if (told.current !== answer.listing.path) {
        told.current = answer.listing.path;
        actions.setAddProjectFolder(device, answer.listing.path);
      }
      return;
    }
    if (asked.via === "start" && asked.request !== "~" && answer.reason && GONE.has(answer.reason)) ask("~", "start");
    // An answer is read once, when it arrives.
  }, [answer?.seq]);

  const place = typedPlace(text, drives, windows);
  // A typed folder or path moves the list once typing settles.
  const typedRequest = typedPlaceRequest(place);
  useEffect(() => {
    if (typedRequest === null || typedRequest === shown?.listing?.label || typedRequest === asked.request) return;
    const timer = window.setTimeout(() => ask(typedRequest, "typed"), TYPED_ASK_DELAY_MS);
    return () => window.clearTimeout(timer);
  }, [typedRequest]);

  const listing = shown?.listing ?? null;
  const filter = place.kind === "filter" ? place.text : place.kind === "folder" && typedRequest === listing?.label ? place.filter : "";
  const items = useMemo(() => filterEntries(listing?.entries ?? [], filter), [listing, filter]);
  // A typed name that is a child's whole name chooses it, so `~/projects/app` names app, not projects.
  const named = selected ?? items.find((entry) => entry.name === filter.trim())?.name ?? null;
  const index = named === null ? -1 : items.findIndex((entry) => entry.name === named);
  const chosen = index >= 0 ? items[index]! : null;

  const typedRefusal = asked.via === "typed" && answer?.reason ? answer.reason : null;
  const outside = typedRefusal === "outside_home" && place.kind === "path" && place.path === asked.request ? place.path : null;
  const disconnected = offline || answer?.reason === "disconnected";
  const loading = !disconnected && asked.via !== "typed" && !answer && shown?.request !== asked.request;
  const failed = !disconnected && asked.via !== "typed" && answer?.reason && !answer.listing ? answer.reason : null;

  const enter = (label: string) => {
    setText("");
    ask(label, "click");
    fieldRef.current?.focus();
  };

  // What Add (or Use this location) takes, and why it cannot.
  const target = (() => {
    if (outside) return { path: outside, label: outside, problem: mode === "pick" ? t("addProject.refusal.outsideHome") : null, note: t("addProject.browse.outsideHome") };
    if (!listing) return null;
    if (chosen && !chosen.is_directory) return { path: "", label: "", problem: t("addProject.browse.file"), note: null };
    const path = chosen ? childPath(listing.path, chosen.name) : listing.path;
    const label = chosen ? childPath(listing.label, chosen.name) : listing.label;
    if (mode === "add" && !chosen && listing.label === "~") return { path, label, problem: t("addProject.browse.homeRoot"), note: null };
    if (mode === "add" && alreadyRegistered(path, device, registrations)) return { path, label, problem: t("addProject.browse.added"), note: null };
    return { path, label, problem: null, note: null };
  })();
  // A typed folder the list does not show yet, still settling or named nowhere, is not what Add would take.
  const settling = typedRequest !== null && typedRequest !== listing?.label && outside === null;
  const ready = target !== null && target.problem === null && !disconnected && !busy && !(loading || failed) && !settling;

  const submit = () => {
    if (!ready || !target) return;
    if (mode === "pick") onPick(target.label);
    else onAdd(target.path);
  };

  const move = (step: number) => {
    if (items.length === 0) return;
    const next = (index + step + items.length) % items.length;
    setSelected(items[next]!.name);
    document.getElementById(`${listId}-${next}`)?.scrollIntoView({ block: "nearest" });
  };

  const complete = () => {
    if (!listing) return false;
    const folder = chosen?.is_directory ? chosen : items.find((entry) => entry.is_directory);
    if (!folder) return false;
    const label = childPath(listing.label, folder.name);
    setText(`${label}/`);
    ask(label, "typed");
    return true;
  };

  const hint =
    outside !== null
      ? target?.note
      : typedRefusal === "not_found" || typedRefusal === "not_a_directory" || typedRefusal === "invalid_path"
        ? t("addProject.browse.noSuchFolder")
        : null;

  return (
    <>
      <DialogHeader>
        <Button type="button" variant="ghost" size="sm" className="-ml-sm mb-xs self-start" onClick={onBack} data-folder-browser-back="true">
          <ArrowLeftIcon />
          {t("common.back")}
        </Button>
        <DialogTitle>{mode === "pick" ? t("addProject.browse.pickTitle") : t("addProject.browseTitle")}</DialogTitle>
      </DialogHeader>
      <DialogBody className="flex min-w-0 flex-col gap-sm" data-folder-browser={device} data-folder-browser-mode={mode}>
        <nav aria-label={t("addProject.browse.listLabel", { host: deviceLabel })} className="flex min-w-0 flex-wrap items-center gap-xxs text-body">
          {listing
            ? folderCrumbs(listing.label).map((crumb, at, all) => (
                <span key={crumb.label} className="flex min-w-0 items-center gap-xxs">
                  {at > 0 ? (
                    <span aria-hidden="true" className="text-muted-foreground">
                      /
                    </span>
                  ) : null}
                  <Button
                    type="button"
                    variant="text"
                    size="inline"
                    className="max-w-full truncate"
                    aria-current={at === all.length - 1 ? "location" : undefined}
                    onClick={() => enter(crumb.label)}
                    data-folder-crumb={crumb.label}
                  >
                    {crumb.label === "~" ? `~ ${t("addProject.browse.home")}` : crumb.name}
                  </Button>
                </span>
              ))
            : null}
        </nav>
        <Input
          ref={fieldRef}
          value={text}
          role="combobox"
          aria-expanded="true"
          aria-controls={listId}
          aria-activedescendant={index >= 0 ? `${listId}-${index}` : undefined}
          aria-label={t("addProject.browse.listLabel", { host: deviceLabel })}
          placeholder={windows ? t("addProject.browse.placeholderWindows") : t("addProject.browse.placeholder")}
          autoComplete="off"
          spellCheck={false}
          data-folder-browser-field="true"
          onChange={(event) => {
            setText(event.target.value);
            setSelected(null);
          }}
          onKeyDown={(event) => {
            if (composingKey(event)) return;
            if (event.key === "ArrowDown" || event.key === "ArrowUp") {
              event.preventDefault();
              move(event.key === "ArrowDown" ? 1 : -1);
            } else if (event.key === "ArrowRight" && text === "" && chosen?.is_directory && listing) {
              event.preventDefault();
              enter(childPath(listing.label, chosen.name));
            } else if (event.key === "ArrowLeft" && text === "" && listing && listing.label !== "~") {
              event.preventDefault();
              enter(listing.label.slice(0, listing.label.lastIndexOf("/")) || "~");
            } else if (event.key === "Tab" && !event.shiftKey) {
              if (complete()) event.preventDefault();
            } else if (event.key === "Enter") {
              event.preventDefault();
              submit();
            }
          }}
        />
        {hint ? (
          <p className="text-caption text-muted-foreground" data-folder-browser-hint={typedRefusal ?? ""}>
            {hint}
          </p>
        ) : null}
        <div
          id={listId}
          role="listbox"
          aria-label={t("addProject.browse.listLabel", { host: deviceLabel })}
          className="h-(--size-folder-list-h) min-w-0 overflow-y-auto rounded-md border border-border p-xxs"
          data-folder-list={listing?.label ?? ""}
        >
          {disconnected ? (
            <ListState data-folder-list-state="disconnected">
              <Status tone="error">{t("addProject.browse.disconnected")}</Status>
            </ListState>
          ) : loading ? (
            <ListState data-folder-list-state="loading">
              <Status tone="pending">{t("addProject.browse.loading")}</Status>
            </ListState>
          ) : failed ? (
            <ListState data-folder-list-state="failed">
              <Status tone="error">{t("addProject.browse.unreadable")}</Status>
              <Button type="button" variant="secondary" size="sm" onClick={() => ask(asked.request, asked.via)} data-folder-list-retry="true">
                {t("workspace.tryAgain")}
              </Button>
            </ListState>
          ) : listing && items.length === 0 ? (
            <ListState data-folder-list-state={filter ? "no-match" : "empty"}>
              <Note tone="muted">{filter ? t("addProject.browse.noMatch") : t("addProject.browse.empty")}</Note>
            </ListState>
          ) : listing ? (
            <>
              {items.map((entry, at) => (
                <FolderRow
                  key={entry.name}
                  id={`${listId}-${at}`}
                  entry={entry}
                  selected={at === index}
                  added={entry.is_directory && alreadyRegistered(childPath(listing.path, entry.name), device, registrations)}
                  onSelect={() => {
                    setSelected(entry.name);
                    fieldRef.current?.focus();
                  }}
                  onEnter={() => {
                    if (entry.is_directory) enter(childPath(listing.label, entry.name));
                  }}
                />
              ))}
              {listing.truncated ? (
                <p className="px-sm py-xs text-caption text-muted-foreground" data-folder-list-truncated="true">
                  {t("addProject.browse.truncated")}
                </p>
              ) : null}
            </>
          ) : null}
        </div>
        <div className="flex min-w-0 items-center gap-sm">
          <span className="flex min-w-0 flex-1 items-center gap-xs text-caption text-muted-foreground" data-folder-browser-target={target?.path ?? ""}>
            {target?.problem ? (
              <span className="min-w-0 truncate" data-folder-browser-problem="true">
                {target.problem}
              </span>
            ) : target ? (
              <>
                <span className="min-w-0 truncate font-mono text-foreground">{target.label}</span>
                <DeviceChip label={deviceLabel} local={local} variant="inline" className="shrink-0" />
              </>
            ) : null}
          </span>
          <Button type="button" disabled={!ready} onClick={submit} data-folder-browser-submit={mode}>
            {mode === "pick" ? t("addProject.browse.useLocation") : t("common.add")}
          </Button>
        </div>
        {status}
        <p className="flex flex-wrap items-center gap-x-sm gap-y-xxs text-caption text-muted-foreground" aria-hidden="true">
          <span>
            <Kbd>↑↓</Kbd> {t("addProject.browse.keyMove")}
          </span>
          <span>
            <Kbd>→</Kbd> {t("addProject.browse.keyIn")}
          </span>
          <span>
            <Kbd>←</Kbd> {t("addProject.browse.keyUp")}
          </span>
          <span>
            <Kbd>Tab</Kbd> {t("addProject.browse.keyComplete")}
          </span>
          <span>
            <Kbd>Enter</Kbd> {mode === "pick" ? t("addProject.browse.keyUse") : t("common.add")}
          </span>
        </p>
      </DialogBody>
    </>
  );
}

/** The list's own state, in the place its rows would be. */
function ListState({ children, ...data }: { children: ReactNode } & Record<`data-${string}`, string>) {
  return (
    <div className="flex h-full flex-col items-center justify-center gap-sm p-md text-center" {...data}>
      {children}
    </div>
  );
}

/** One child: a click selects it, a double click enters a folder; a repository shows its branch, a folder its repositories, a project already added is dimmed. */
function FolderRow({ id, entry, selected, added, onSelect, onEnter }: { id: string; entry: HomeEntry; selected: boolean; added: boolean; onSelect: () => void; onEnter: () => void }) {
  const { t } = useInterfaceTranslation();
  const Icon = entry.is_directory ? FolderIcon : FileIcon;
  return (
    <div
      id={id}
      role="option"
      aria-selected={selected}
      aria-disabled={added || undefined}
      onMouseDown={(event) => event.preventDefault()}
      onClick={(event) => (event.detail >= 2 ? onEnter() : onSelect())}
      className={`flex min-w-0 cursor-default select-none items-center gap-sm rounded-sm px-sm py-xxs text-body ${selected ? "bg-secondary text-foreground" : "text-subtle-foreground hover:bg-accent"} ${added ? "opacity-(--opacity-dimmed)" : ""}`}
      data-folder-row={entry.name}
      data-folder-row-added={added || undefined}
    >
      <Icon aria-hidden="true" className="size-(--size-icon) shrink-0 text-muted-foreground" />
      <span className="min-w-0 flex-1 truncate">{entry.name}</span>
      {added ? (
        <Badge variant="outline">{t("addProject.browse.addedBadge")}</Badge>
      ) : entry.repository ? (
        <Badge variant="outline" aria-label={entry.branch ? t("addProject.browse.branchLabel", { branch: entry.branch }) : "Git"} data-folder-row-branch={entry.branch ?? ""}>
          <GitBranchIcon aria-hidden="true" />
          <span className="max-w-(--size-pr-branch-max) truncate">{entry.branch ?? "Git"}</span>
        </Badge>
      ) : entry.repositories ? (
        <Badge variant="secondary" data-folder-row-repositories={entry.repositories}>
          {t("addProject.browse.repositories", { count: entry.repositories })}
        </Badge>
      ) : null}
      {entry.is_directory ? <ChevronRightIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-muted-foreground" /> : null}
    </div>
  );
}

/** The folder a typed line asks the machine for, or null when it only filters the folder shown. */
function typedPlaceRequest(place: TypedPlace): string | null {
  return place.kind === "folder" ? place.folder : place.kind === "path" ? place.path : null;
}
