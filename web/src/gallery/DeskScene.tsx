// Desk 1차 (PRD session-docs-and-assistant) for the operator's review before
// any of it is wired: every scene of the final design (proposal-v2: 1, 2,
// 2-1, 3-7) drawn from the real Desk parts in `components/desk` over
// invented values (`deskSceneData.ts`), plus the states it leaves out (the
// agent and @ menus, Hide's parts in a thread, the pane's way back, a
// mission, every chat notice, a thread that cannot be read, Settings ›
// Desk). The sidebar column is a stand-in built from the shared row parts;
// the real sidebar is not changed. Nothing leaves the page.
import { DiamondIcon, FileTextIcon, ImageIcon, MapIcon } from "lucide-react";
import { useLayoutEffect, useMemo, useState, type ReactNode } from "react";
import { createActions } from "../actions";
import { AgentMark } from "../AgentMark";
import { DeskAgentBar, DeskComposer, type ComposerAttachment } from "../components/desk/desk-composer";
import { DeskChat, DeskEmpty, DeskHandedLine, type DeskChatActions } from "../components/desk/desk-chat";
import { DeskPaneReturn, DeskSidebarRow } from "../components/desk/desk-entry";
import { DeskJobCard, JobMark, OutputMark } from "../components/desk/desk-job-card";
import { DeskChatColumn, DeskChatHead, DeskRail, DeskRoom, DeskViewColumn, LaneColumnHead, LaneHeadControls, type ViewTab } from "../components/desk/desk-lane";
import { DeskMentionMenu } from "../components/desk/desk-mention-menu";
import { DeskOutputs, EveryOutput } from "../components/desk/desk-outputs";
import { DeskSettingsTab } from "../components/desk/desk-settings";
import { SessionCard, SessionChip, SessionChipProvider } from "../components/desk/session-chip";
import { ThreadList } from "../components/desk/thread-list";
import { ThreadHead, ThreadView, type ThreadConductor } from "../components/desk/thread-view";
import { SessionRow } from "../components/session-row";
import { StatusMark } from "../components/status-mark";
import { TooltipProvider } from "../components/ui/tooltip";
import { askingCount, outputKindOf, threadLines, unseenOutputs, type DeskAgent, type DeskItem, type DeskJob, type DeskOutput, type DeskSelection, type DeskSessionRef, type ThreadReadFailure } from "../desk";
import { useInterfaceTranslation } from "../i18n/client";
import { requireInterfaceLanguage } from "../i18n/locale";
import { useFollowInterfaceLanguage } from "../i18n/translator";
import { MissionMap } from "../MissionMap";
import { cn } from "../lib/utils";
import { useShellStore } from "../store";
import type { SnapshotRest } from "../snapshot";
import { missionFixture } from "./MissionScene";
import {
  AGENTS, AGENTS_PREPARING, cardSceneItems, chipsScene, conductorThread, deskJob, ENTERED_SESSIONS, everyStateJobs, GALLERY_DONE_THREAD, galleryThread, handoffItems, MENTIONS, MENU_PROJECTS, MENU_SESSIONS,
  mentionItems, missionItems, OUTPUTS, PICTURES, RECORD_THREAD, SESSION_VIEWS, SESSIONS, stateItems, statusItems, storyJobs, switchedItems, WORKER_THREAD,
} from "./deskSceneData";

/** Each scene, with the proposal-v2 scene it pairs with or the PRD behaviors it adds. */
export const DESK_SCENES = [
  { state: "s1", label: "1 빈 Desk (B1-B4)" },
  { state: "s2", label: "2 대화: 맡기기 (B7, B16, B24)" },
  { state: "s2b", label: "2-1 현황 묻기 (B7, D-24)" },
  { state: "s3", label: "3 카드 상태 모음 (B16-B24, B36-B39)" },
  { state: "s4", label: "4 스레드 하나, 핀 고정 (B25, B26, B29)" },
  { state: "s5", label: "5 스레드가 물을 때 (B20, B25, B29)" },
  { state: "s6", label: "6 Threads 목록, 핀 안 됨 (B28)" },
  { state: "s7", label: "7 결과물이 스레드 옆에 (B30, B31)" },
  { state: "every-output", label: "▣ 모든 결과물, 칸 머리 📌 (B31, B32)" },
  { state: "agent", label: "에이전트 고르기 · 바꿈 (B4, B8, B9)" },
  { state: "mention", label: "@ 메뉴 (B16, B18)" },
  { state: "mention-project", label: "@ 프로젝트에서 ⇥ (B18)" },
  { state: "parts", label: "스레드 속 Hide 부품 (B26)" },
  { state: "pane", label: "터미널로 ⌘J, 나도 참여 중 (B35, B36)" },
  { state: "mission", label: "미션: 지도 + 워커 스레드 (B34)" },
  { state: "outputs", label: "결과물 기준 (B30)" },
  { state: "chat-states", label: "채팅 상태: 승인 답, 알림 줄, 생각 중 · 중단 (B5-B8, B11, B13-B15)" },
  { state: "thread-states", label: "스레드 읽기 실패 · 읽는 중 · 빈 · 연결 안 됨 (B23, B27)" },
  { state: "chips", label: "답 속 칩 전부 · 흐린 칩 · 칩 상세 카드 (B50, D-86)" },
  { state: "settings", label: "Settings › Desk (B4, B19, B21)" },
  { state: "settings-off", label: "Settings › Desk 꺼짐, 기본값 (D-76, B41)" },
] as const;

type SceneState = (typeof DESK_SCENES)[number]["state"];

const noop = () => undefined;

const CHAT_ACTIONS: DeskChatActions = {
  onOpen: noop, onAnswer: noop, onCancel: noop, onRequeue: noop, onRetry: noop, onOutput: noop, onMission: noop,
  onApproval: noop, onNoticeAction: noop, onCopy: noop, onMention: noop, onHanded: noop,
};

const PART_ACTIONS = { onMission: noop, onOutput: noop, onSession: noop };

/** A session the scene names, as its chip draws it. */
const sceneSession = (name: string): DeskSessionRef => Object.values(SESSIONS).find((session) => session.name === name) ?? { name, mark: null, agent_kind: null };

/** The stand-in sidebar: the ◆ Desk row, then each project's ◆ Desk group as the shared task rows (B3, B20). */
function SceneSidebar({ jobs, selected }: { jobs: readonly DeskJob[]; selected: string | null }) {
  const { t } = useInterfaceTranslation();
  const projects = [...new Set(jobs.map((job) => job.project.label))];
  return (
    <aside className="flex w-(--size-sidebar-ideal) shrink-0 flex-col gap-xs border-r border-sidebar-border bg-sidebar px-xs py-sm text-sidebar-foreground" data-desk-scene-sidebar="true">
      <DeskSidebarRow selected={selected === null} asking={askingCount(jobs)} onOpen={noop} />
      {projects.map((project) => {
        const rows = jobs.filter((job) => job.project.label === project && job.state !== "moved_out");
        return (
          <section key={project} className="flex min-w-0 flex-col">
            <h3 className="px-xs pb-xxs pt-sm text-caption text-muted-foreground">{project}</h3>
            <p className="flex items-center gap-xs px-xs py-xxs text-caption text-subtle-foreground"><DiamondIcon aria-hidden="true" className="size-(--size-icon-sm) fill-current" /><span className="flex-1">{t("desk.name")}</span>{rows.length}</p>
            <ol className="flex min-w-0 flex-col pl-(--size-tree-indent)">
              {rows.map((job) => <SessionRow key={job.id} mark={<JobMark job={job} />} title={job.title} className={cn(selected === job.id && "bg-sidebar-accent")} />)}
            </ol>
          </section>
        );
      })}
    </aside>
  );
}

/** The chat column's head, as a scene sets it. */
function ChatHead() {
  return <DeskChatHead onNewConversation={noop} />;
}

/** Desk's composer with its bar in the box's foot, as a scene sets them. */
function ChatComposer({ agents = AGENTS, selection, menuOpen = null, value = "", answering = false, mention = null }: { agents?: DeskAgent[]; selection: DeskSelection; menuOpen?: "agent" | null; value?: string; answering?: boolean; mention?: ReactNode }) {
  const { t } = useInterfaceTranslation();
  const [text, setText] = useState(value);
  return (
    <DeskComposer value={text} label={t("desk.composer.label")} placeholder={t("desk.composer.placeholder")} answering={answering} focused={mention !== null} menu={mention} menuOpen={mention !== null} onChange={setText} onSend={noop} onStop={noop} onAttach={noop} onRemoveAttachment={noop}>
      <DeskAgentBar agents={agents} selection={selection} open={menuOpen} onAgent={noop} onModel={noop} onEffort={noop} />
    </DeskComposer>
  );
}

const CLAUDE: DeskSelection = { agent: "claude", model: "opus", effort: "high" };
const CODEX: DeskSelection = { agent: "codex", model: "gpt", effort: "medium" };

/** A thread in the lane's tool column: its head, read as a pane's, and the thread under it. */
function ThreadColumn({ job, model = null, updates, parts, pinned = false, conductor = null, reading = false, failure = null, outputs = job.outputs, selectedOutput = null, focused = false, draft = "", attachments = [] }: {
  job: DeskJob;
  model?: string | null;
  updates: Parameters<typeof threadLines>[0];
  parts?: Parameters<typeof threadLines>[1];
  pinned?: boolean;
  conductor?: ThreadConductor | null;
  reading?: boolean;
  failure?: ThreadReadFailure | null;
  outputs?: readonly DeskOutput[];
  selectedOutput?: string | null;
  focused?: boolean;
  draft?: string;
  attachments?: ComposerAttachment[];
}) {
  const [text, setText] = useState(draft);
  const lines = useMemo(() => threadLines(updates, parts, sceneSession), [updates, parts]);
  return <>
    <ThreadHead job={job} model={model} conductor={conductor} pinned={pinned} onBack={noop} onTerminal={noop} onPin={noop} onClose={noop} />
    <ThreadView
      job={job} lines={lines} reading={reading} failure={failure} mentions={MENTIONS} startedByDesk outputs={outputs} selectedOutput={selectedOutput}
      composer={{ value: text, attachments, focused, onChange: setText, onSend: noop, onAttach: noop, onRemoveAttachment: noop }}
      onTerminal={noop} {...PART_ACTIONS}
    />
  </>;
}

/** An image output in the 보기 column: the picture, fitted as the image viewer fits it. */
function PictureView({ src, label }: { src: string; label: string }) {
  return <div className="flex min-h-0 flex-1 items-center justify-center p-md"><img src={src} alt={label} className="max-h-full max-w-full rounded-sm border border-border object-contain" /></div>;
}

const tab = (id: string, label: string, preview: boolean, icon: ReactNode): ViewTab => ({ id, label, preview, icon });

export function DeskScene({ params }: { params: URLSearchParams }) {
  useFollowInterfaceLanguage(requireInterfaceLanguage(params.get("lang") ?? "ko"));
  const { t } = useInterfaceTranslation();
  const theme = params.get("theme") === "light" ? "light" : "dark";
  const state = (params.get("state") ?? "s1") as SceneState;
  if (!DESK_SCENES.some((scene) => scene.state === state)) throw new Error(`Unknown Desk scene ${state}`);
  const now = useMemo(() => Date.now(), []);
  const jobs = useMemo(() => storyJobs(now), [now]);
  const actions = useMemo(() => createActions(() => true), []);
  const mission = useMemo(() => missionFixture("running", now), [now]);
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(() => new Set(["conductor"]));
  useLayoutEffect(() => {
    // The mission scene's map is Hide's own mission map, drawn from the store as the mission scene draws it.
    const missions = (mission.rest as { missions: { items: { title: string }[] } }).missions.items.map((one) => ({ ...one, title: "Desk 1차" }));
    useShellStore.setState({ rest: { ...mission.rest, missions: { items: missions } } as unknown as SnapshotRest, agents: [], connection: "live" });
    document.documentElement.classList.toggle("dark", theme === "dark");
    document.documentElement.classList.toggle("light", theme === "light");
    document.documentElement.style.setProperty("--interface-scale", params.get("scale") ?? "1");
  }, [mission, theme, params]);

  const story = Object.values(jobs);
  const pick = (...ids: string[]) => ids.map((id) => jobs[id]!);
  const rail = (threads: boolean, outputs: boolean, shown: readonly DeskJob[] = story) => <DeskRail asking={askingCount(shown)} unseen={unseenOutputs(shown)} threads={threads} outputs={outputs} onThreads={noop} onOutputs={noop} />;
  const chat = (items: DeskItem[], composer: ReactNode, options: { jobs?: Record<string, DeskJob>; selectedJob?: string | null; selectedOutput?: string | null; live?: string | null; thinking?: boolean } = {}) => (
    <DeskChatColumn head={<ChatHead />} composer={composer}>
      <DeskChat items={items} jobs={options.jobs ?? jobs} agents={AGENTS} startedAt={now - 3_000} selectedJob={options.selectedJob ?? null} selectedOutput={options.selectedOutput ?? null} live={options.live ?? null} thinking={options.thinking ?? false} {...CHAT_ACTIONS} />
    </DeskChatColumn>
  );
  const threads = (pinned: boolean, shown: readonly DeskJob[], selected: string | null = null) => (
    <>
      <LaneColumnHead pinned={pinned} onExpand={noop} onPin={noop} onClose={noop}><span className="font-semibold text-foreground">{t("desk.threads.title")}</span></LaneColumnHead>
      <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-y-auto">
        <ThreadList jobs={shown} selected={selected} expanded={expanded} onOpen={noop} onMission={noop} onOutput={noop} onToggle={(id) => setExpanded((open) => { const next = new Set(open); if (next.has(id)) next.delete(id); else next.add(id); return next; })} />
      </div>
    </>
  );

  let sidebar: ReactNode = <SceneSidebar jobs={pick("mention", "gallery", "release")} selected={null} />;
  let body: ReactNode;
  switch (state) {
    case "s1": {
      // A new conversation while threads from before still run: the sidebar, the rail and the empty Desk count the same ones.
      const running = pick("mention", "gallery", "release");
      sidebar = <SceneSidebar jobs={running} selected={null} />;
      body = <DeskRoom pinned={false} rail={rail(false, false, running)}
        chat={<DeskChatColumn head={<ChatHead />} composer={<ChatComposer selection={CLAUDE} />}><DeskEmpty working={running.filter((job) => job.state === "working").length} asking={askingCount(running)} onExample={noop} onShowThreads={noop} /></DeskChatColumn>} />;
      break;
    }
    case "s2":
      body = <DeskRoom pinned={false} rail={rail(false, false, pick("mention", "gallery", "release"))} chat={chat(handoffItems(now), <ChatComposer selection={CLAUDE} />, { thinking: true, selectedJob: "gallery" })} />;
      break;
    case "s2b":
      body = <DeskRoom pinned={false} rail={rail(false, false)} chat={chat(statusItems(now), <ChatComposer selection={CLAUDE} />)} />;
      break;
    case "s3":
      sidebar = null;
      body = <CardStates />;
      break;
    case "s4": {
      const gallery = deskJob(jobs.gallery!, { state: "working", ask: null, question: null, now: "다크 모드 찍는 중", plan: { done: 3, total: 5 } });
      const thread = galleryThread();
      const items: DeskItem[] = [
        { kind: "user", id: 1, text: "갤러리 캡처 라이트·다크로 비교해줘", at_ms: now, mentions: [] },
        { kind: "reply", mentions: [], agent: "claude", id: 2, text: "갤러리 비교를 새 스레드로 띄웠어요.", at_ms: now, state: "done", thought_ms: null, steps: { sessions: 0, files: 0, letters: 0, starts: 0, other: 0 } },
        { kind: "job", id: 3, job_id: "gallery" },
      ];
      body = <DeskRoom pinned rail={rail(true, false)} chat={chat(items, <ChatComposer selection={CLAUDE} />, { jobs: { gallery }, selectedJob: "gallery" })}
        tool={<ThreadColumn job={gallery} model="Opus 5.5" updates={thread.updates} parts={thread.parts} pinned outputs={[]} />} />;
      break;
    }
    case "s5": {
      const items: DeskItem[] = [{ kind: "job", id: 1, job_id: "record" }, { kind: "job", id: 2, job_id: "gallery" }];
      body = <DeskRoom pinned rail={rail(true, false)} chat={chat(items, <ChatComposer selection={CLAUDE} />, { selectedJob: "record" })}
        tool={<ThreadColumn job={jobs.record!} model="GPT-6" updates={RECORD_THREAD} pinned outputs={[]} focused draft="응 지우고 다시 빌드해" />} />;
      break;
    }
    case "s6": {
      const items: DeskItem[] = [
        { kind: "user", id: 1, text: "오늘 herdr-ide에서 돌던 거 어디까지 왔어?", at_ms: now, mentions: [] },
        { kind: "reply", mentions: [], agent: "claude", id: 2, text: "지금 다섯 개가 돌고 있고 둘이 답을 기다려요.", at_ms: now, state: "done", thought_ms: null, steps: { sessions: 0, files: 0, letters: 0, starts: 0, other: 0 } },
      ];
      // A sleeping and an archived thread are in no group: the foot line counts them.
      const resting = [deskJob(jobs.mention!, { id: "asleep", title: "문서 정리", state: "sleeping", agent_kind: "codex" }), deskJob(jobs.release!, { id: "archived", title: "로그 정리", state: "archived" })];
      const listed = [deskJob(jobs.gallery!, { mark_read: true }), ...pick("record", "conductor", "1a", "1b", "mention", "reel", "release"), ...resting];
      body = <DeskRoom pinned={false} rail={rail(true, false)} chat={chat(items, <ChatComposer selection={CLAUDE} />)} tool={threads(false, listed, "record")} />;
      break;
    }
    case "s7": {
      const done = deskJob(jobs.gallery!, { state: "done", ask: null, question: null, summary: "차이 3곳", plan: { done: 5, total: 5 }, outputs: [OUTPUTS.compareLight, OUTPUTS.compareDark, OUTPUTS.notes] });
      const items: DeskItem[] = [{ kind: "job", id: 1, job_id: "gallery" }];
      body = <DeskRoom pinned rail={rail(false, true)}
        chat={chat(items, <ChatComposer selection={CLAUDE} />, { jobs: { gallery: done }, selectedJob: "gallery", selectedOutput: OUTPUTS.compareDark.id })}
        view={<DeskViewColumn tabs={[tab("notes", "notes.md", false, <FileTextIcon aria-hidden="true" className="size-(--size-icon-sm)" />), tab("compare", "compare-dark.png", true, <ImageIcon aria-hidden="true" className="size-(--size-icon-sm)" />)]} active="compare" onTab={noop} onKeep={noop} onCloseTab={noop}><PictureView src={PICTURES.dark} label="compare-dark.png" /></DeskViewColumn>}
        tool={<ThreadColumn job={done} model="Opus 5.5" updates={GALLERY_DONE_THREAD.updates} parts={GALLERY_DONE_THREAD.parts} pinned selectedOutput={OUTPUTS.compareDark.id} />} />;
      break;
    }
    case "every-output": {
      const done = deskJob(jobs.gallery!, { state: "done", ask: null, question: null, summary: "차이 3곳", plan: { done: 5, total: 5 }, outputs: [OUTPUTS.compareLight, OUTPUTS.compareDark, OUTPUTS.notes] });
      const shown = [done, ...pick("record", "release")];
      body = <DeskRoom pinned={false} rail={rail(false, true, shown)} chat={chat([{ kind: "job", id: 1, job_id: "gallery" }], <ChatComposer selection={CLAUDE} />, { jobs: { gallery: done } })}
        view={<DeskViewColumn tabs={[]} active={null} onTab={noop} onKeep={noop} onCloseTab={noop} controls={<LaneHeadControls edge expanded={false} pinned={false} onExpand={noop} onPin={noop} onClose={noop} />}><EveryOutput threads={shown} onOpen={noop} /></DeskViewColumn>} />;
      break;
    }
    case "agent":
      body = <DeskRoom pinned={false} rail={rail(false, false)} chat={chat(switchedItems(now), <ChatComposer selection={CODEX} menuOpen="agent" />)} />;
      break;
    case "mention":
    case "mention-project": {
      const entered = state === "mention-project" ? { project: MENU_PROJECTS[0]!, sessions: ENTERED_SESSIONS } : null;
      const menu = <DeskMentionMenu sessions={MENU_SESSIONS} projects={MENU_PROJECTS} entered={entered} active="session:agent-record-now" onActive={noop} onSession={noop} onProject={noop} onBack={noop} />;
      body = <DeskRoom pinned={false} rail={rail(false, false)} chat={chat(mentionItems(now), <ChatComposer selection={CLAUDE} value={state === "mention-project" ? "@herdr-ide " : "@re"} mention={menu} />)} />;
      break;
    }
    case "parts": {
      const thread = conductorThread(jobs);
      body = <DeskRoom pinned rail={rail(true, false)} chat={chat(missionItems(now), <ChatComposer selection={CLAUDE} />, { selectedJob: "conductor" })}
        tool={<ThreadColumn job={jobs.conductor!} updates={thread.updates} parts={thread.parts} pinned outputs={[]} />} />;
      break;
    }
    case "pane":
      sidebar = <SceneSidebar jobs={pick("mention", "gallery", "release")} selected="mention" />;
      body = (
        <div className="flex min-h-0 min-w-0 flex-1 flex-col" data-desk-scene-pane="true">
          <div className="flex h-(--size-pane-header) shrink-0 items-center gap-xs border-b border-border bg-muted px-sm text-caption">
            <StatusMark mark="going" />
            <AgentMark kind="claude" />
            <span className="min-w-0 flex-1 truncate text-foreground">mention 버튼 정리</span>
            <DeskPaneReturn joined onDesk={noop} />
          </div>
          <pre className="min-h-0 flex-1 overflow-hidden bg-background p-md font-mono text-caption text-subtle-foreground">{"⏺ PaneHeader의 @ 버튼을 hover에서만 보이게 바꿀게요.\n  Edit web/src/PaneHeader.tsx\n  Ran bash scripts/verify-web.sh --filter PaneHeader\n\n❯ 아이콘 크기도 14로 줄여 주세요\n\n⏺ 14로 줄일게요…"}</pre>
        </div>
      );
      break;
    case "mission":
      body = <DeskRoom pinned={false} rail={rail(false, true)} chat={chat(missionItems(now), <ChatComposer selection={CLAUDE} />, { selectedJob: "conductor" })}
        view={<DeskViewColumn tabs={[tab("map", t("desk.lane.missionMap", { title: "Desk 1차" }), true, <MapIcon aria-hidden="true" className="size-(--size-icon-sm)" />)]} active="map" onTab={noop} onKeep={noop} onCloseTab={noop}><MissionMap missionId="mission-1" actions={actions} /></DeskViewColumn>}
        tool={<ThreadColumn job={jobs["1b"]!} updates={WORKER_THREAD} conductor={{ title: "Desk 1차", onOpen: noop }} outputs={[]} />} />;
      break;
    case "outputs":
      body = <OutputsCriteria />;
      break;
    case "chat-states":
      body = <DeskRoom pinned={false} rail={rail(false, false)} chat={chat(stateItems(now), <ChatComposer agents={AGENTS_PREPARING} selection={CODEX} answering />, { live: "지금 돌고 있는 일은 세 개예요. mention 버튼 정리는 테스트를 돌리는 중이고" })} />;
      break;
    case "thread-states": {
      const fresh = deskJob(jobs.mention!, { id: "fresh", title: "로그 정리", state: "starting", now: null, plan: null, pull_requests: [], outputs: [] });
      const off: DeskJob = jobs.reel!;
      const column = (key: string, thread: ReactNode) => (
        <div key={key} className="flex min-h-0 min-w-0 flex-1 flex-col border-r border-border" data-desk-scene-thread-state={key}>{thread}</div>
      );
      body = (
        <div className="flex min-h-0 min-w-0 flex-1">
          {column("failed", <ThreadColumn job={jobs.release!} updates={[]} failure="no_record" />)}
          {column("reading", <ThreadColumn job={jobs.release!} updates={[]} reading />)}
          {column("empty", <ThreadColumn job={fresh} updates={[]} />)}
          {column("disconnected", <ThreadColumn job={off} updates={RECORD_THREAD} outputs={[]} />)}
        </div>
      );
      break;
    }
    case "chips": {
      // Every session the answer names is a chip, the 17th on too; m349 is no longer listed, so it is dimmed with no status.
      const scene = chipsScene(now);
      const open = scene.facts.get(scene.open)!;
      const source = { facts: (session: DeskSessionRef) => (session.pane_id ? scene.facts.get(session.pane_id) ?? null : null), onOpen: noop };
      body = (
        <SessionChipProvider source={source}>
          <DeskRoom pinned={false} rail={rail(false, false)} chat={chat(scene.items, <ChatComposer selection={CLAUDE} />)} />
          <aside className="flex shrink-0 flex-col gap-xs border-l border-border p-lg" data-desk-scene-chip-card="open">
            <h3 className="text-micro text-muted-foreground">칩에 마우스를 올리면</h3>
            <span><SessionChip session={{ name: open.card!.title, mark: open.mark, agent_kind: open.agent_kind, pane_id: scene.open }} onPress={noop} className="ring-1 ring-mention-session" /></span>
            <div className="w-(--size-pr-popover) max-w-full rounded-sm border border-border bg-popover text-popover-foreground shadow-lg">
              <SessionCard job={open.card!} since={open.since} parent={open.parent} onOpen={noop} onParent={noop} />
            </div>
          </aside>
        </SessionChipProvider>
      );
      break;
    }
    case "settings":
    case "settings-off":
      sidebar = null;
      body = <div className="w-(--size-settings-sheet-w) max-w-full p-xl"><DeskSettingsTab settings={{ enabled: state === "settings", default_agent: "claude", location: "desk", cap: 8 }} agents={AGENTS} onChange={noop} /></div>;
      break;
  }
  return (
    <TooltipProvider>
      <div className="flex h-screen min-w-0 bg-background text-body text-foreground" data-gallery-scene="desk" data-desk-scene={state}>
        {sidebar}
        {body}
      </div>
    </TooltipProvider>
  );
}

/**
 * Scene 3: every card state at the chat's width, each under the name of the
 * state it shows, then the lines that change no state and Desk's own
 * approval card, asked and answered.
 */
function CardStates() {
  const now = useMemo(() => Date.now(), []);
  return (
    <div className="flex min-h-0 flex-1 justify-center overflow-y-auto p-lg" data-desk-scene-cards="true">
      <div className="flex w-full max-w-(--size-editor-measure) min-w-0 flex-col gap-xs">
        {everyStateJobs(now).map(({ label, job }) => (
          <section key={job.id} className="flex min-w-0 flex-col gap-xxs">
            <h3 className="pt-xs text-micro text-muted-foreground">{label}</h3>
            <DeskJobCard job={job} {...CHAT_ACTIONS} />
          </section>
        ))}
        <h3 className="pt-sm text-micro text-muted-foreground">기존 세션에 맡김 (상태가 안 바뀜: 상자 없음)</h3>
        <DeskHandedLine session={SESSIONS.recordNow} handed line="갤러리 숫자 줄인 거 다크에서도 확인해줘" read onOpen={noop} />
        <h3 className="pt-sm text-micro text-muted-foreground">에이전트 바꿈 · 막힘 · Desk 자신의 승인 (로고 없는 유일한 ! 카드) · 답한 승인</h3>
        <DeskChat items={cardSceneItems()} jobs={{}} agents={AGENTS} {...CHAT_ACTIONS} />
      </div>
    </div>
  );
}

/** 결과물 기준 (D-65, B30): a session's View list as it is, and the outputs Desk keeps of it. */
function OutputsCriteria() {
  const { t } = useInterfaceTranslation();
  const kept: DeskOutput[] = [OUTPUTS.compareLight, OUTPUTS.galleryPage, OUTPUTS.notes, { ...OUTPUTS.pr, label: "PR 1902", pr_state: "merged" }];
  return (
    <div className="grid min-h-0 flex-1 grid-cols-2 content-start gap-xl p-xl" data-desk-scene-outputs="true">
      <section className="flex min-w-0 flex-col gap-xs">
        <h3 className="text-caption font-medium text-muted-foreground">View</h3>
        <ul className="flex min-w-0 flex-col gap-xxs">
          {SESSION_VIEWS.map((view) => {
            const kind = outputKindOf(view.kind);
            return (
              <li key={view.label} className={cn("flex min-w-0 items-center gap-xs text-caption", kind ? "text-subtle-foreground" : "text-muted-foreground line-through opacity-(--opacity-dimmed)")} data-desk-view-kept={kind ? "true" : "false"}>
                <OutputMark output={{ id: view.label, kind: kind ?? "doc", label: view.label, thumb: null, seen: true, pr_state: kind === "pr" ? "merged" : null, device_id: null, checkout: null, path: null, url: null }} />
                <span className="truncate">{view.label}</span>
                <span className="ml-auto font-mono text-micro text-muted-foreground">{view.kind}</span>
              </li>
            );
          })}
        </ul>
      </section>
      <section className="flex min-w-0 flex-col gap-xs">
        <h3 className="text-caption font-medium text-muted-foreground">{t("desk.outputs.title")}</h3>
        <DeskOutputs outputs={kept} onOpen={noop} />
      </section>
    </div>
  );
}
