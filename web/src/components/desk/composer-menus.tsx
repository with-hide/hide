import { useEffect, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import type { DeskMention } from "../../desk";
import { commandAt, commandRows, mentionAt, mentionTree, type MentionFile, type MentionProject, type MentionSession } from "../../deskComposer";
import { composingKey } from "../../keys";
import type { DeskCommand, DeskCommandsUnread, DeskMentions } from "../../snapshot";
import { DeskCommandMenu, type DeskCommandPanel, type HideCommand } from "./desk-command-menu";
import { DeskMentionMenu } from "./desk-mention-menu";

/** Where the `@` menu stands: the `@` Esc closed, the project ⇥ entered, the highlighted row and the operator's folds. */
type MenuPlace = { dismissed: number | null; entered: string | null; active: string; folds: Record<string, boolean> };

const NO_PLACE: MenuPlace = { dismissed: null, entered: null, active: "", folds: {} };
const NO_COMMANDS: readonly DeskCommand[] = [];
const NO_OWN: readonly HideCommand[] = [];

/** A Hide row's value in the `/` menu, apart from every agent command's name, which starts with its `/` or `$`. */
const ownValue = (command: HideCommand) => `hide:${command.id}`;

/** What one composer's `/` and `@` menus read, and what a pick writes into its box. */
export type ComposerMenuSource = {
  text: string;
  caret: number;
  /**
   * The `/` menu: the agent's name and its list, null before the agent ever sent one, with why (`unread`, the
   * core's; none for an agent Desk does not speak with); `opening` while Desk's session with it opens and the list
   * is on its way. None for a box with no agent.
   */
  commands: { agent: string; list: readonly DeskCommand[] | null; unread?: DeskCommandsUnread | null; opening?: boolean } | null;
  /**
   * Hide's own rows at the head of the `/` menu, above the agent's commands, the same for every agent (Desk's
   * `새 대화`); a pick runs it rather than writing words. None in a session's box.
   */
  own?: readonly HideCommand[];
  /** Called once each time the `/` menu opens, so the core can check whether the agent's list moved. */
  onCommandsOpen?: () => void;
  mentions: DeskMentions | null;
  /** Desk's `@` lists the projects and enters one with ⇥; a session's lists its project's files instead. */
  projects: boolean;
  /** The files of the session's project that match what follows `@`, listed above the sessions. */
  files?: readonly MentionFile[];
  onCommand: (at: { query: string }, command: DeskCommand) => void;
  onMention: (at: { start: number; query: string }, mention: DeskMention) => void;
  onFile?: (at: { start: number; query: string }, file: MentionFile) => void;
};

/** One composer's menus: the panel to draw over the box, whether it is open, and the keys it takes before the box. */
export type ComposerMenus = {
  menu: ReactNode;
  menuOpen: boolean;
  /** The open menu holds nothing to choose, so ↵ sends. */
  menuEmpty: boolean;
  onKeyDown: (event: KeyboardEvent<HTMLElement>) => void;
  onMenuOpenChange: (open: boolean) => void;
};

/**
 * The `/` and `@` menus of a composer (PRD Desk 1차 B18, B48; feedback 1
 * frame P 가), the same for Desk's box and a session's (mission-106): `/`
 * at the start of the words lists the agent's own commands, `@` the
 * sessions as a tree, with Desk's projects or the session's project's files.
 * ↑ ↓ move, ↵ picks, → and ← fold a parent, ⇥ enters a project, Esc closes;
 * a pick goes to the caller, which writes it into its box its own way.
 */
export function useComposerMenus(source: ComposerMenuSource): ComposerMenus {
  const { text, caret, commands, mentions, files = [], own = NO_OWN } = source;
  const [menu, setMenu] = useState<MenuPlace>(NO_PLACE);
  // The `/` menu's highlighted row, and whether Esc closed it for the `/` word the box starts with now.
  const [commandMenu, setCommandMenu] = useState<{ active: string; dismissed: boolean }>({ active: "", dismissed: false });

  // The `@` menu: open while the caret ends an `@` word Esc did not close; when nothing matches it says so in one row.
  const at = mentionAt(text, caret);
  const entered = source.projects && menu.entered !== null ? mentions?.projects.find((project) => project.id === menu.entered) ?? null : null;
  const found = at && mentions ? mentionTree(mentions, at.query, menu.folds) : null;
  const tree = found && !source.projects ? { ...found, projects: [] } : found;
  const shownFiles = at && !source.projects ? files : [];
  const enteredSessions = entered && mentions ? mentions.sessions.filter((session) => entered.sessions.includes(session.id)) : [];
  const values = entered
    ? [`new:${entered.id}`, ...enteredSessions.map((session) => `session:${session.id}`)]
    : tree
      ? [...shownFiles.map((file) => `file:${file.relative_path}`), ...[...tree.rows, ...tree.found].map((row) => `session:${row.session.id}`), ...tree.projects.map((project) => `project:${project.id}`)]
      : [];

  // The `/` menu: open while the box starts with a `/` word the caret is in. Hide's own rows that match come first; then
  // the agent's commands that match; with none matching it says so, and before the agent ever sent a list it says
  // that Hide is reading it or why it has none. An agent whose session sent an empty list has no part of the menu;
  // ↵ sends when the menu lists nothing.
  const slash = commandAt(text, caret);
  const list = commands?.list ?? null;
  const ownHits = slash ? ownRows(own, slash.query) : NO_OWN;
  const commandHits = slash && list ? commandRows(list, slash.query) : NO_COMMANDS;
  const unread = commands?.unread ?? null;
  const commandPanel: DeskCommandPanel | null = !slash || !commands ? null
    : list === null ? (unread === "reading" || commands.opening ? { kind: "waiting" } : unread ? { kind: "unread", why: unread } : { kind: "unknown" })
    : list.length === 0 ? null
    : commandHits.length > 0 ? { kind: "rows", commands: commandHits }
    // A Hide row that matches is the menu's answer; the agent's part then says nothing.
    : ownHits.length > 0 ? null
    : { kind: "none" };
  const commandValues = [...ownHits.map(ownValue), ...commandHits.map((command) => command.name)];
  const commandOpen = (commandPanel !== null || ownHits.length > 0) && !commandMenu.dismissed;
  const commandActive = commandValues.includes(commandMenu.active) ? commandMenu.active : (commandValues[0] ?? "");
  const inSlash = slash !== null;
  useEffect(() => {
    if (!inSlash && commandMenu.dismissed) setCommandMenu({ active: "", dismissed: false });
  }, [inSlash, commandMenu.dismissed]);
  // Once per opening of the `/` word, never per key typed after it.
  const hasCommands = commands !== null;
  const onCommandsOpen = useRef(source.onCommandsOpen);
  onCommandsOpen.current = source.onCommandsOpen;
  useEffect(() => {
    if (inSlash && hasCommands) onCommandsOpen.current?.();
  }, [inSlash, hasCommands]);
  const menuOpen = commandOpen || (at !== null && menu.dismissed !== at.start && (values.length > 0 || tree !== null));
  const menuEmpty = commandOpen ? commandValues.length === 0 : menuOpen && values.length === 0;
  const active = values.includes(menu.active) ? menu.active : (values[0] ?? "");

  const pickCommandRow = (value: string) => {
    if (!slash) return;
    const mine = ownHits.find((one) => ownValue(one) === value);
    const command = commandHits.find((one) => one.name === value);
    if (!mine && !command) return;
    setCommandMenu({ active: "", dismissed: true });
    if (mine) mine.run();
    else if (command) source.onCommand(slash, command);
  };
  const pick = (mention: DeskMention) => {
    if (!at) return;
    setMenu(NO_PLACE);
    source.onMention(at, mention);
  };
  const pickSession = (session: MentionSession) => pick({ kind: "session", id: session.id, name: session.name, mark: session.mark, agent_kind: session.agent_kind });
  const pickProject = (project: MentionProject) => pick({ kind: "project", id: project.id, name: project.name });
  const pickFile = (file: MentionFile) => {
    if (!at) return;
    setMenu(NO_PLACE);
    source.onFile?.(at, file);
  };
  const choose = (value: string) => {
    const [kind, ...rest] = value.split(":");
    const id = rest.join(":");
    if (kind === "session") {
      const session = mentions?.sessions.find((one) => one.id === id);
      if (session) pickSession(session);
    } else if (kind === "file") {
      const file = shownFiles.find((one) => one.relative_path === id);
      if (file) pickFile(file);
    } else {
      const project = mentions?.projects.find((one) => one.id === id);
      if (project) pickProject(project);
    }
  };
  const fold = (session: MentionSession, open: boolean) => setMenu({ ...menu, active: `session:${session.id}`, folds: { ...menu.folds, [session.id]: open } });

  // The `/` menu's keys: ↑ ↓ move, ↵ writes the command into the box, Esc closes it.
  const onCommandKey = (event: KeyboardEvent<HTMLElement>) => {
    const key = event.key;
    // With no rows the panel's only key is Esc; ↵ sends the words.
    if (commandValues.length === 0 && key !== "Escape") return;
    if (key === "ArrowDown" || key === "ArrowUp") {
      event.preventDefault();
      const index = commandValues.indexOf(commandActive);
      setCommandMenu({ ...commandMenu, active: commandValues[(index + (key === "ArrowDown" ? 1 : commandValues.length - 1)) % commandValues.length]! });
    } else if ((key === "Enter" && !event.shiftKey) || (key === "Tab" && !event.shiftKey)) {
      event.preventDefault();
      pickCommandRow(commandActive);
    } else if (key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      setCommandMenu({ ...commandMenu, dismissed: true });
    }
  };

  // The `@` menu's keys; the composer sends on ↵ when the menu is closed.
  const onKeyDown = (event: KeyboardEvent<HTMLElement>) => {
    if (composingKey(event) || !menuOpen) return;
    if (commandOpen) return onCommandKey(event);
    const key = event.key;
    // With nothing to choose the menu has no keys of its own but Esc, which closes it; ↵ sends the words.
    if (menuEmpty) {
      if (key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        setMenu({ ...menu, dismissed: at!.start });
      }
      return;
    }
    const row = tree?.rows.find((one) => `session:${one.session.id}` === active);
    if (key === "ArrowDown" || key === "ArrowUp") {
      event.preventDefault();
      const index = values.indexOf(active);
      const next = (index + (key === "ArrowDown" ? 1 : values.length - 1)) % values.length;
      setMenu({ ...menu, active: values[next]! });
    } else if (key === "ArrowRight" && row && row.children > 0 && !row.open) {
      // → opens a folded parent and ← folds it again, as the sidebar's tree does (D-84).
      event.preventDefault();
      fold(row.session, true);
    } else if (key === "ArrowLeft" && row && (row.open || row.session.parent !== null)) {
      event.preventDefault();
      if (row.open) fold(row.session, false);
      else setMenu({ ...menu, active: `session:${row.session.parent}` });
    } else if (key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      choose(active);
    } else if (key === "Tab" && !event.shiftKey && active.startsWith("project:")) {
      event.preventDefault();
      const id = active.slice("project:".length);
      setMenu({ ...menu, entered: id, active: `new:${id}` });
    } else if ((key === "Tab" && event.shiftKey) || key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      setMenu(menu.entered !== null ? { ...menu, entered: null, active: "" } : { ...menu, dismissed: at!.start });
    }
  };

  const panel = commandOpen ? (
    <DeskCommandMenu
      agent={commands?.agent ?? ""}
      own={ownHits}
      panel={commandPanel}
      query={slash?.query ?? ""}
      active={commandActive}
      onActive={(value) => setCommandMenu({ ...commandMenu, active: value })}
      onPickOwn={(command) => pickCommandRow(ownValue(command))}
      onPick={(command) => pickCommandRow(command.name)}
    />
  ) : menuOpen && at && tree ? (
    <DeskMentionMenu
      tree={tree}
      files={shownFiles}
      searching={at.query !== ""}
      entered={entered ? { project: entered, sessions: enteredSessions } : null}
      active={active}
      onActive={(value) => setMenu({ ...menu, active: value })}
      onSession={pickSession}
      onProject={pickProject}
      onFile={pickFile}
      onFold={fold}
      onBack={() => setMenu({ ...menu, entered: null, active: "" })}
    />
  ) : null;

  return {
    menu: panel,
    menuOpen,
    menuEmpty,
    onKeyDown,
    onMenuOpenChange: (open) => {
      if (open) return;
      if (commandOpen) setCommandMenu({ ...commandMenu, dismissed: true });
      else if (at) setMenu({ ...menu, dismissed: at.start });
    },
  };
}

/**
 * Hide's rows whose name starts with what follows `/`, case aside, in their own order. Only a start: words inside a
 * name (`/con` in `New conversation`) would put Hide's row over the agent's `/context`, and ↵ would run it.
 */
export function ownRows(own: readonly HideCommand[], query: string): readonly HideCommand[] {
  const needle = query.toLocaleLowerCase();
  return own.filter((command) => command.name.toLocaleLowerCase().startsWith(needle));
}
