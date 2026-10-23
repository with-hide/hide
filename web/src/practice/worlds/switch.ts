// Chapter 2's world: two invented projects, an app with two agents in two
// tabs and its docs with one, answering the moves between them: a pane
// chosen from the recent order, a project brought forward, and a file found
// with ⌘P and opened in the lane's View.

import type { EditorDocumentSnapshot, EditorTabSnapshot, ViewDisplaySnapshot } from "../../snapshot";
import type { FileIndexResult } from "../../store";
import type { CoreEvent, PracticeWorld, WorldSnapshot } from "../world";
import { agent, applyDesk, deskSnapshot, panesOf, shellScreen, type DeskModel, type PracticeDesk, type PracticePane, type PracticePlace } from "./kit";

const APP: PracticePlace = { project: "practice-app", label: "practice-app", branch: "main" };
const DOCS: PracticePlace = { project: "practice-docs", label: "practice-docs", branch: "main" };
const PLACES = [APP, DOCS] as const;

/** Every practice project's folder, as `kit.ts` places it. */
const pathOf = (place: PracticePlace) => `/work/${place.project}`;

/** The files each project's index answers with, by project. */
const FILES: Record<string, string[]> = {
  [APP.project]: ["src/pay.ts", "src/pay.test.ts", "src/login.ts", "src/session.ts", "package.json"],
  [DOCS.project]: ["README.md", "docs/payments.md", "docs/login.md", "docs/install.md"],
};

/** What each opened file holds. */
const CONTENTS: Record<string, string> = {
  "README.md": "# Practice docs\n\nHow the practice app takes payments and signs people in.\n\n- [Payments](docs/payments.md)\n- [Login](docs/login.md)\n- [Install](docs/install.md)\n",
  "docs/payments.md": "# Payments\n\nCheckout sends the order to `pay()` in `src/pay.ts`.\nA payment that takes longer than three seconds is retried once.\n",
  "docs/login.md": "# Login\n\nA session lasts a day. Signing in again renews it.\n",
  "docs/install.md": "# Install\n\n```sh\nnpm install\nnpm test\n```\n",
  "src/pay.ts": "export async function pay(order: Order) {\n  const timeout = 3000;\n  return charge(order, { timeout });\n}\n",
  "src/pay.test.ts": "test(\"pay retries once\", async () => {\n  expect(await pay(order)).toBe(\"paid\");\n});\n",
  "src/login.ts": "export function signIn(user: User) {\n  return startSession(user, { days: 1 });\n}\n",
  "src/session.ts": "export const SESSION_DAYS = 1;\n",
  "package.json": "{\n  \"name\": \"practice-app\",\n  \"scripts\": { \"test\": \"vitest\" }\n}\n",
};

/** A file opened from the palette, in the project it was opened in. */
type Opened = { project: string; path: string };

export type SwitchModel = {
  desks: PracticeDesk[];
  /** The project in front. */
  front: string;
  /** Agent panes, the one the keyboard was in last first, as the core keeps `recent_pane_ids`. */
  recent: string[];
  /** The project in front when the keyboard first left the pane it started in, however far back it went. */
  left: string | null;
  /** The palette's last question, once the index has answered one. */
  index: { root: string; query: string } | null;
  opened: Opened | null;
  /** How many times the lane was called, as the core numbers `lane_calls`. */
  calls: number;
};

function desk(place: PracticePlace, tabs: { id: string; pane: PracticePane }[], nextId: number): PracticeDesk {
  return {
    place,
    model: {
      panes: tabs.map((tab) => tab.pane),
      tabs: tabs.map(({ id, pane }) => ({ id, name: null, root: { type: "pane", pane_id: pane.id }, focused: pane.id, zoomed: false })),
      groups: [{ id: null, name: null, tabs: tabs.map((tab) => tab.id) }],
      selectedGroup: null,
      activeTab: tabs[0]!.id,
      // Each desk numbers its new panes and tabs in its own range, so ids stay unique across the two.
      nextId,
      operations: [],
    },
  };
}

function initial(): SwitchModel {
  const pay = agent({ pane_id: "pay", identity_label: "Fix the checkout 500", agent_kind: "claude", group: "working", mark: "going", status_code: "working", activity: "working", elapsed: "2m" });
  const login = agent({ pane_id: "login", identity_label: "Fix the login bug", agent_kind: "codex", group: "working", mark: "going", status_code: "working", activity: "working", elapsed: "6m" });
  const docs = agent({ pane_id: "docs", identity_label: "Tidy the README", agent_kind: "claude", group: "working", mark: "going", status_code: "working", activity: "working", elapsed: "9m" });
  return {
    desks: [
      desk(APP, [{ id: "app1", pane: { id: "pay", agent: pay, title: "Fix the checkout 500" } }, { id: "app2", pane: { id: "login", agent: login, title: "Fix the login bug" } }], 10),
      desk(DOCS, [{ id: "docs1", pane: { id: "docs", agent: docs, title: "Tidy the README" } }], 50),
    ],
    front: APP.project,
    recent: ["pay", "login", "docs"],
    left: null,
    index: null,
    opened: null,
    calls: 0,
  };
}

const holding = (model: SwitchModel, has: (desk: DeskModel) => boolean) => model.desks.find((one) => has(one.model));

/** `model` with the desk of `project` replaced by `next`, and that project in front. */
function withDesk(model: SwitchModel, project: string, next: DeskModel): SwitchModel {
  return { ...model, front: project, desks: model.desks.map((one) => (one.place.project === project ? { ...one, model: next } : one)) };
}

/** The pane the keyboard is in on the desk in front. */
function frontPane(model: SwitchModel): string {
  const shown = model.desks.find((one) => one.place.project === model.front)!.model;
  return shown.tabs.find((tab) => tab.id === shown.activeTab)!.focused;
}

/** The model with `pane` first in the recent order, when it runs an agent. */
function visited(model: SwitchModel, pane: string): SwitchModel {
  if (!model.recent.includes(pane) || model.recent[0] === pane) return model;
  return { ...model, recent: [pane, ...model.recent.filter((id) => id !== pane)] };
}

/** The pane the keyboard starts the chapter in. */
const START = "pay";

function apply(model: SwitchModel, event: CoreEvent): SwitchModel | null {
  const after = move(model, event);
  if (!after || after.left !== null || frontPane(after) === START) return after;
  return { ...after, left: after.front };
}

function move(model: SwitchModel, event: CoreEvent): SwitchModel | null {
  const p = event.payload;
  switch (event.kind) {
    case "focus_pane": {
      const id = String(p.pane_id);
      const owner = holding(model, (one) => one.tabs.some((tab) => panesOf(tab.root).includes(id)));
      const next = owner && applyDesk(owner.model, event);
      return owner && next ? visited(withDesk(model, owner.place.project, next), id) : null;
    }
    case "focus_tab": {
      const owner = holding(model, (one) => one.tabs.some((tab) => tab.id === p.tab_id));
      const next = owner && applyDesk(owner.model, event);
      if (!owner || !next) return null;
      const after = withDesk(model, owner.place.project, next);
      return visited(after, frontPane(after));
    }
    case "focus_checkout": {
      const place = PLACES.find((one) => one.project === p.workspace_id);
      if (!place) return null;
      const after = { ...model, front: place.project };
      return visited(after, frontPane(after));
    }
    case "file_index": {
      const root = String(p.root);
      const query = String(p.query ?? "");
      if (!PLACES.some((place) => pathOf(place) === root)) return null;
      if (model.index?.root === root && model.index.query === query) return null;
      return { ...model, index: { root, query } };
    }
    case "file_open": {
      const place = PLACES.find((one) => one.project === p.workspace_id);
      const path = String(p.path);
      if (!place || !path.startsWith(`${pathOf(place)}/`)) return null;
      return { ...model, front: place.project, opened: { project: place.project, path: path.slice(pathOf(place).length + 1) }, calls: model.calls + 1 };
    }
    default: {
      // Splits, zoom, tabs and groups act on the desk in front, as chapter 1 taught them.
      const shown = model.desks.find((one) => one.place.project === model.front)!;
      const next = applyDesk(shown.model, event);
      return next ? withDesk(model, shown.place.project, next) : null;
    }
  }
}

/** The project's files whose path holds the query's letters in order, as the index matches them. */
function matching(project: string, query: string): string[] {
  const letters = query.toLowerCase().replace(/\s+/g, "");
  return FILES[project]!.filter((file) => {
    let at = 0;
    for (const letter of file.toLowerCase()) if (letter === letters[at]) at += 1;
    return at === letters.length;
  });
}

function fileIndex(model: SwitchModel): FileIndexResult | null {
  if (!model.index) return null;
  const place = PLACES.find((one) => pathOf(one) === model.index!.root)!;
  const files = matching(place.project, model.index.query).map((relative) => ({ path: `${pathOf(place)}/${relative}`, relative_path: relative }));
  return { device_id: "local", root_path: model.index.root, query: model.index.query, files, truncated: false, indexing: false, unavailable: null };
}

const EDITOR_TAB = "file1";

function snapshot(model: SwitchModel, nowMs: number): WorldSnapshot {
  const shot = deskSnapshot(model.desks, model.front, nowMs);
  const rest = { ...shot.rest, ui_state: { ...shot.rest.ui_state, recent_pane_ids: model.recent } };
  const frames: NonNullable<WorldSnapshot["frames"]> = { fileIndex: fileIndex(model) };
  const view = rest.workspace_view!;
  const opened = model.opened;
  const place = PLACES.find((one) => one.project === model.front)!;
  // The lane's View shows the opened file while its project is in front.
  if (opened && opened.project === model.front) {
    const path = `${pathOf(place)}/${opened.path}`;
    const label = opened.path.slice(opened.path.lastIndexOf("/") + 1);
    const display: ViewDisplaySnapshot = { id: "view1", tab_id: EDITOR_TAB, path, label, kind: "file", committed: null, preview: true, state: "open", reason: null };
    rest.workspace_view = {
      ...view, open: true, documents: true, pinned: true, lane_called: model.calls, lane_calls: model.calls, lane_called_side: "view",
      layout: { root: { area: { id: "v1", active: display.id, displays: [display] } }, active_area: "v1", display_count: 1, limits: { areas: 4, depth: 3, displays: 32 } },
    };
    const tab: EditorTabSnapshot = { id: EDITOR_TAB, workspace_id: place.project, checkout_id: `${place.project}:${place.branch}`, path, label, kind: "file", diff_committed: null, markdown_live: true, wrap: false, dirty: false, preview: true };
    const markdown = path.endsWith(".md");
    const document: EditorDocumentSnapshot = { path, language: markdown ? "markdown" : "typescript", document_kind: markdown ? "markdown" : "text", contents_utf8: CONTENTS[opened.path] ?? "", revision: "sha256:0", dirty: false, readonly_reason: null, conflict: null, save: null };
    frames.editor = { tabs: [tab], active_tab_id: EDITOR_TAB };
    frames.documents = { [EDITOR_TAB]: document };
  } else {
    rest.workspace_view = { ...view, lane_called: model.calls, lane_calls: model.calls };
  }
  return { ...shot, rest, frames };
}

/** What each agent's terminal shows: the request it was given and what it is doing about it. */
const SCREENS: Record<string, string[]> = {
  pay: ["\x1b[38;5;209m✻\x1b[0m Claude Code", "", "\x1b[2m>\x1b[0m Checkout returns a 500 when I pay. Fix it", "", "\x1b[38;5;209m●\x1b[0m The payment call times out after one second.", "  \x1b[2m⎿  Editing src/pay.ts…\x1b[0m"],
  login: ["\x1b[1m>_\x1b[0m Codex", "", "\x1b[2m›\x1b[0m People get signed out after an hour. Fix it", "", "\x1b[32m•\x1b[0m The session ends an hour early.", "  \x1b[2m└ Running npm test\x1b[0m"],
  docs: ["\x1b[38;5;209m✻\x1b[0m Claude Code", "", "\x1b[2m>\x1b[0m Tidy the README and link every guide", "", "\x1b[38;5;209m●\x1b[0m Linked payments, login and install.", "  \x1b[2m⎿  Writing README.md…\x1b[0m"],
};

function screen(model: SwitchModel, paneId: string): string {
  const owner = model.desks.find((one) => one.model.panes.some((pane) => pane.id === paneId))!;
  return SCREENS[paneId]?.join("\r\n") ?? shellScreen(owner.place);
}

export const SWITCH_WORLD: PracticeWorld<SwitchModel> = {
  initial,
  apply,
  snapshot,
  panes: (model) => model.desks.flatMap((one) => one.model.panes.map((pane) => pane.id)),
  screen,
  lane: false,
};
