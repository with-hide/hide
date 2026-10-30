// Desk's room on screen (PRD session-docs-and-assistant, Desk 1차 B1-B24):
// the `desk`, `desk_live`, `desk_jobs` and `desk_mentions` sections drawn
// with the Desk parts, and each press sent as one `desk_*` event. The core
// decides every state; this screen keeps only what the operator is writing
// and where the `@` menu is. The lane (Threads, 결과물) is the lane's own.

import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent, type RefObject } from "react";
import type { Actions } from "./actions";
import { askingCount, type DeskItem, type DeskJob, type DeskMention } from "./desk";
import { mentionAt, mentionRows, outgoing, pickMention } from "./deskComposer";
import { DeskChat, DeskEmpty, type DeskChatActions, type DeskNoticeAction } from "./components/desk/desk-chat";
import { DeskAgentBar, DeskComposer } from "./components/desk/desk-composer";
import { DeskSidebarRow } from "./components/desk/desk-entry";
import { DeskChatColumn, DeskChatHead, DeskRoom } from "./components/desk/desk-lane";
import { DeskMentionMenu, type MentionProject, type MentionSession } from "./components/desk/desk-mention-menu";
import { DeskSettingsTab } from "./components/desk/desk-settings";
import { useInterfaceTranslation } from "./i18n/client";
import { catalogWorkspaces } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { windowIsActive } from "./viewFocus";

const NO_JOBS: DeskJob[] = [];

/** The sidebar's `◆ Desk` row (B1), once hided runs Desk. */
export function DeskSidebarEntry({ actions }: { actions: Actions }) {
  const shown = useShellStore((s) => s.desk !== null);
  const selected = useUiStore((s) => s.screen?.kind === "desk");
  const asking = useShellStore((s) => askingCount(s.deskJobs?.jobs ?? NO_JOBS));
  if (!shown) return null;
  return (
    <div className="shrink-0 px-xs pt-xs">
      <DeskSidebarRow selected={selected} asking={asking} onOpen={actions.openDesk} />
    </div>
  );
}

/** Settings › Desk (B4, B19, B21). */
export function DeskSettingsPage({ actions }: { actions: Actions }) {
  const desk = useShellStore((s) => s.desk);
  if (!desk) return null;
  return <DeskSettingsTab settings={desk.settings} agents={desk.agents} onChange={actions.deskSettings} />;
}

/** A conversation this close to its end follows what arrives. */
const STICK_PX = 48;

/**
 * Reads the job cards the operator can see (a card read is its row read, as
 * the sidebar reads it): cards in the conversation's view while this window
 * is in front, each change of a card once.
 */
function useReadShownCards(scroller: RefObject<HTMLDivElement | null>, jobs: readonly DeskJob[], onRead: (id: string) => void) {
  const sent = useRef(new Set<string>());
  useEffect(() => {
    const box = scroller.current;
    if (!box) return;
    let frame = 0;
    const read = () => {
      frame = 0;
      if (document.visibilityState !== "visible" || !windowIsActive()) return;
      const view = box.getBoundingClientRect();
      const unread = new Map(jobs.filter((job) => !job.read).map((job) => [job.id, job]));
      for (const card of Array.from(box.querySelectorAll<HTMLElement>("[data-desk-job-id]"))) {
        const job = unread.get(card.dataset.deskJobId!);
        if (!job) continue;
        const rect = card.getBoundingClientRect();
        if (rect.bottom <= view.top || rect.top >= view.bottom) continue;
        const key = `${job.id}\u0000${job.changed_at_unix_ms}`;
        if (sent.current.has(key)) continue;
        sent.current.add(key);
        onRead(job.id);
      }
    };
    const soon = () => {
      if (!frame) frame = requestAnimationFrame(read);
    };
    soon();
    box.addEventListener("scroll", soon, { passive: true });
    window.addEventListener("focus", soon);
    document.addEventListener("visibilitychange", soon);
    return () => {
      if (frame) cancelAnimationFrame(frame);
      box.removeEventListener("scroll", soon);
      window.removeEventListener("focus", soon);
      document.removeEventListener("visibilitychange", soon);
    };
  }, [scroller, jobs, onRead]);
}

/** Where the `@` menu stands: the `@` word Esc closed it on, and the project ⇥ entered. */
type MenuPlace = { dismissed: number | null; entered: string | null; active: string };

export function DeskScreen({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const desk = useShellStore((s) => s.desk);
  const live = useShellStore((s) => s.deskLive);
  const jobs = useShellStore((s) => s.deskJobs?.jobs ?? NO_JOBS);
  const mentions = useShellStore((s) => s.deskMentions);
  const draft = useUiStore((s) => s.deskDraft);
  const setDraft = useUiStore((s) => s.setDeskDraft);
  const root = useRef<HTMLDivElement>(null);
  const scroller = useRef<HTMLDivElement>(null);
  const stick = useRef(true);
  const [caret, setCaret] = useState(draft.text.length);
  const [menu, setMenu] = useState<MenuPlace>({ dismissed: null, entered: null, active: "" });
  // The words a send carried, held until the conversation shows them, so a refused send keeps them in the box.
  const pending = useRef<{ text: string; after: number } | null>(null);

  const byId = useMemo(() => Object.fromEntries(jobs.map((job) => [job.id, job])), [jobs]);
  const items = desk?.conversation.items;
  // A card past the newest 64 the section carries is not drawn; its line in the conversation goes with it.
  const shown = useMemo(() => (items ?? []).filter((item) => item.kind !== "job" || byId[item.job_id] !== undefined), [items, byId]);
  const turn = live?.started_at_ms ?? null;
  const answering = turn !== null;
  const liveText = live?.text ? live.text : null;
  const agent = desk?.selection.agent ? desk.agents.find((one) => one.id === desk.selection.agent) : undefined;
  const canSend = !answering && (agent?.state === "ready" || agent?.state === "login_needed");

  const lastUserId = (list: readonly DeskItem[]) => list.reduce((last, item) => (item.kind === "user" ? item.id : last), -1);
  useEffect(() => {
    const sent = pending.current;
    if (!sent || !items) return;
    if (items.some((item) => item.kind === "user" && item.id > sent.after && item.text === sent.text)) {
      pending.current = null;
      if (outgoing(useUiStore.getState().deskDraft).text === sent.text) setDraft({ text: "", mentions: [] });
    }
  }, [items, setDraft]);

  useLayoutEffect(() => {
    const box = scroller.current;
    if (box && stick.current) box.scrollTop = box.scrollHeight;
  }, [shown, liveText, answering]);
  useEffect(() => {
    const box = scroller.current;
    if (!box) return;
    const follow = () => {
      stick.current = box.scrollHeight - box.scrollTop - box.clientHeight < STICK_PX;
    };
    box.addEventListener("scroll", follow, { passive: true });
    return () => box.removeEventListener("scroll", follow);
  }, []);

  useReadShownCards(scroller, jobs, actions.deskJobRead);

  const send = (words = draft) => {
    if (!canSend || !items) return;
    const out = outgoing(words);
    if (!out.text) return;
    pending.current = { text: out.text, after: lastUserId(items) };
    stick.current = true;
    actions.deskSend(out.text, out.mentions);
  };

  // The `@` menu: open while the caret ends an `@` word Esc did not close, and something matches it.
  const at = mentionAt(draft.text, caret);
  const entered = menu.entered !== null ? mentions?.projects.find((project) => project.id === menu.entered) ?? null : null;
  const rows = at && mentions ? mentionRows(mentions, at.query) : null;
  const enteredSessions = entered && mentions ? mentions.sessions.filter((session) => entered.sessions.includes(session.id)) : [];
  const values = entered ? [`new:${entered.id}`, ...enteredSessions.map((session) => `session:${session.id}`)] : rows ? [...rows.sessions.map((session) => `session:${session.id}`), ...rows.projects.map((project) => `project:${project.id}`)] : [];
  const menuOpen = at !== null && menu.dismissed !== at.start && values.length > 0;
  const active = values.includes(menu.active) ? menu.active : (values[0] ?? "");

  const input = () => root.current?.querySelector<HTMLTextAreaElement>("[data-desk-composer-input]") ?? null;
  const place = (next: number) => {
    setCaret(next);
    requestAnimationFrame(() => input()?.setSelectionRange(next, next));
  };
  const pick = (mention: DeskMention) => {
    if (!at) return;
    const next = pickMention(draft, at, mention);
    setDraft(next.draft);
    setMenu({ dismissed: null, entered: null, active: "" });
    place(next.caret);
    input()?.focus();
  };
  const pickSession = (session: MentionSession) => pick({ kind: "session", id: session.id, name: session.name, mark: session.mark, agent_kind: session.agent_kind });
  const pickProject = (project: MentionProject) => pick({ kind: "project", id: project.id, name: project.name });
  const choose = (value: string) => {
    const [kind, ...rest] = value.split(":");
    const id = rest.join(":");
    if (kind === "session") {
      const session = mentions?.sessions.find((one) => one.id === id);
      if (session) pickSession(session);
    } else {
      const project = mentions?.projects.find((one) => one.id === id);
      if (project) pickProject(project);
    }
  };

  // The `@` menu's keys; the composer sends on ↵ when the menu is closed.
  const onKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.nativeEvent.isComposing || !menuOpen) return;
    const key = event.key;
    if (key === "ArrowDown" || key === "ArrowUp") {
      event.preventDefault();
      const index = values.indexOf(active);
      const next = (index + (key === "ArrowDown" ? 1 : values.length - 1)) % values.length;
      setMenu({ ...menu, active: values[next]! });
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
  const track = () => {
    const box = input();
    if (box) setCaret(box.selectionStart);
  };

  const onNoticeAction = useCallback((action: DeskNoticeAction, notice: Extract<DeskItem, { kind: "notice" }>) => {
    const now = useShellStore.getState().desk;
    switch (action) {
      case "restart":
        return actions.deskRestart();
      case "reinstall": {
        const which = notice.agent ?? now?.selection.agent ?? null;
        if (which) actions.deskInstall(which);
        return;
      }
      case "retry": {
        // 다시 시도 sends the operator's last message again, as they wrote it.
        const last = [...(now?.conversation.items ?? [])].reverse().find((item) => item.kind === "user");
        if (last?.kind === "user") actions.deskSend(last.text, last.mentions);
        return;
      }
    }
  }, [actions]);
  const chatActions = useMemo<DeskChatActions>(() => {
    // The lane shows a thread once it is drawn here; until then a card opens its session's own tab.
    const openJob = (job: DeskJob) => {
      if (job.pane_id) actions.openAgent(job.pane_id);
    };
    return {
      onOpen: openJob,
      onAnswer: openJob,
      onCancel: (job) => actions.deskJobCancel(job.id),
      onRequeue: (job) => actions.deskJobRequeue(job.id),
      onRetry: (job) => actions.deskJobRequeue(job.id),
      onOutput: (output) => {
        const job = useShellStore.getState().deskJobs?.jobs.find((one) => one.outputs.some((made) => made.id === output.id));
        if (job) openJob(job);
      },
      onMission: (missionId) => actions.openMission(missionId),
      onApproval: (id, allow) => actions.deskApproval(id, allow),
      onNoticeAction,
      onCopy: (text) => void actions.copyText(text, "desk sign-in command"),
      onMention: (mention) => {
        if (mention.kind === "session") return actions.openAgent(mention.id);
        const rest = useShellStore.getState().rest;
        const project = catalogWorkspaces(rest).find((row) => row.checkouts.some((checkout) => checkout.path === mention.id));
        const checkout = project?.checkouts.find((row) => row.path === mention.id);
        if (project && checkout) actions.openWorkspace(project.device_id, project.id, checkout.id);
      },
      onHanded: (paneId) => actions.openAgent(paneId),
    };
  }, [actions, onNoticeAction]);

  if (!desk) return null;
  const working = jobs.filter((job) => job.state === "working").length;
  const composer = (
    <DeskComposer
      value={draft.text}
      label={t("desk.composer.label")}
      placeholder={t("desk.composer.placeholder")}
      answering={answering}
      held={!canSend}
      focused={menuOpen}
      menuOpen={menuOpen}
      menu={menuOpen ? (
        <DeskMentionMenu
          sessions={rows?.sessions ?? []}
          projects={rows?.projects ?? []}
          entered={entered ? { project: entered, sessions: enteredSessions } : null}
          active={active}
          onActive={(value) => setMenu({ ...menu, active: value })}
          onSession={pickSession}
          onProject={pickProject}
          onBack={() => setMenu({ ...menu, entered: null, active: "" })}
        />
      ) : null}
      onMenuOpenChange={(open) => {
        if (!open && at) setMenu({ ...menu, dismissed: at.start });
      }}
      onChange={(text) => {
        setDraft({ ...draft, text });
        requestAnimationFrame(track);
      }}
      onKeyDown={onKeyDown}
      onSend={() => send()}
      onStop={actions.deskStop}
    >
      <DeskAgentBar agents={desk.agents} selection={desk.selection} onAgent={(id) => actions.deskSelect(id, null, null)} onModel={(model) => actions.deskSelect(null, model, null)} onEffort={(effort) => actions.deskSelect(null, null, effort)} />
    </DeskComposer>
  );
  return (
    <div ref={root} className="flex min-h-0 min-w-0 flex-1" data-desk-screen="true" onKeyUp={track} onMouseUp={track}>
      <DeskRoom
        pinned={false}
        rail={null}
        chat={(
          <DeskChatColumn head={<DeskChatHead onNewConversation={actions.deskNewConversation} />} composer={composer} scrollRef={scroller}>
            {shown.length === 0 && !answering ? (
              <DeskEmpty working={working} asking={askingCount(jobs)} onExample={(text) => (canSend ? send({ text, mentions: [] }) : setDraft({ text, mentions: [] }))} />
            ) : (
              <DeskChat items={shown} jobs={byId} agents={desk.agents} live={liveText} thinking={answering && liveText === null} startedAt={turn} {...chatActions} />
            )}
          </DeskChatColumn>
        )}
      />
    </div>
  );
}
