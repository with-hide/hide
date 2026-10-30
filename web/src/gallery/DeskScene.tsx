// Desk 1차 (PRD session-docs-and-assistant) for the operator's review before
// any of it is wired: every scene of the approved mockup drawn from the real
// Desk parts in `components/desk` over invented values (`deskSceneData.ts`),
// plus the states the mockup leaves out (every card, every chat notice, a
// thread that cannot be read, Settings › Desk) and the one question for the
// operator (`row-sizes`). The sidebar column is a stand-in built from the
// shared row parts; the real sidebar is not changed. Nothing leaves the page.
import { DiamondIcon, FileTextIcon, ImageIcon, MapIcon, PlusIcon } from "lucide-react";
import { useLayoutEffect, useMemo, useState, type ReactNode } from "react";
import { createActions } from "../actions";
import { AgentMark } from "../AgentMark";
import { DeskAgentBar, DeskComposer, type ComposerAttachment } from "../components/desk/desk-composer";
import { DeskChat, DeskEmpty, type DeskChatActions } from "../components/desk/desk-chat";
import { DeskPaneReturn, DeskSidebarRow } from "../components/desk/desk-entry";
import { DeskHandedCard, DeskJobCard, JobMark, OutputMark, jobLine } from "../components/desk/desk-job-card";
import { DeskChatColumn, DeskRail, DeskRoom, DeskViewColumn, LaneColumnHead, type ViewTab } from "../components/desk/desk-lane";
import { DeskMentionMenu } from "../components/desk/desk-mention-menu";
import { DeskOutputs } from "../components/desk/desk-outputs";
import { DeskSettingsTab } from "../components/desk/desk-settings";
import { ThreadList } from "../components/desk/thread-list";
import { ThreadHead, ThreadView, type ThreadPathStep } from "../components/desk/thread-view";
import { SessionRow, SessionRowBody } from "../components/session-row";
import { StatusMark } from "../components/status-mark";
import { Button } from "../components/ui/button";
import { RowButton } from "../components/ui/row-button";
import { TooltipProvider } from "../components/ui/tooltip";
import { askingCount, outputKindOf, threadLines, unseenOutputs, type DeskAgent, type DeskItem, type DeskJob, type DeskOutput, type DeskSelection, type ThreadReadFailure } from "../desk";
import { useInterfaceTranslation } from "../i18n/client";
import { requireInterfaceLanguage } from "../i18n/locale";
import { useFollowInterfaceLanguage } from "../i18n/translator";
import { MarkdownText } from "../MarkdownText";
import { MissionMap } from "../MissionMap";
import { cn } from "../lib/utils";
import { useShellStore } from "../store";
import type { SnapshotRest } from "../snapshot";
import { missionFixture } from "./MissionScene";
import {
  AGENTS, AGENTS_PREPARING, conductorThread, ENTERED_SESSIONS, everyStateJobs, GALLERY_THREAD, handoffItems, MENTION_THREAD, MENTIONS, MENU_PROJECTS, MENU_SESSIONS,
  mentionItems, missionItems, NOTES_MD, OUTPUTS, PICTURES, RELEASE_THREAD, SESSION_VIEWS, stateItems, storyJobs, switchedItems, WORKER_THREAD,
} from "./deskSceneData";

/** Each scene, with the mockup's section it draws (desk-thread.html) or the PRD behaviors it adds. */
export const DESK_SCENES = [
  { state: "s1", label: "1 처음 Desk (B1-B4)" },
  { state: "s1b", label: "1-1 에이전트 고르기 · 새 대화 (B4, B8, B9)" },
  { state: "s2", label: "2 맡기기 (B16, B17, B20, B24)" },
  { state: "s3", label: "3 스레드 목록, 핀 안 됨 (B28)" },
  { state: "s3b", label: "3-1 @ 메뉴 (B16, B18)" },
  { state: "s3c", label: "3-2 @ 프로젝트에서 ⇥ (B18)" },
  { state: "s4", label: "4 스레드 하나 (B25, B29)" },
  { state: "s4b", label: "4-1 Hide 부품으로 그리는 줄 (B26)" },
  { state: "s5", label: "5 결과물 미리보기, 핀 안 됨 (B30, B31)" },
  { state: "s6", label: "6 핀 고정, 남긴 탭 (B31, B32)" },
  { state: "s7", label: "7 질문에 답 (B29)" },
  { state: "s8", label: "8 터미널로 ⌘J, 나도 참여 중 (B35, B36)" },
  { state: "s9", label: "9 미션: 지도 + Conductor 스레드 (B34)" },
  { state: "outputs", label: "결과물 기준 (B30)" },
  { state: "cards", label: "일 카드 모든 상태 (B16, B21-B24, B36-B39)" },
  { state: "chat-states", label: "채팅 상태: 승인 카드, 알림 줄, 생각 중 · 중단 (B5-B8, B11, B13-B15)" },
  { state: "thread-failed", label: "스레드 기록 읽기 실패 (B27)" },
  { state: "settings", label: "Settings › Desk (B4, B19, B21)" },
  { state: "row-sizes", label: "운영자 질문: 태스크 줄 두 크기" },
] as const;

type SceneState = (typeof DESK_SCENES)[number]["state"];

const noop = () => undefined;

const CHAT_ACTIONS: DeskChatActions = {
  onOpen: noop, onAnswer: noop, onCancel: noop, onRequeue: noop, onRetry: noop, onOutput: noop, onMission: noop,
  onApproval: noop, onNoticeAction: noop, onCopy: noop, onMention: noop, onHanded: noop,
};

const PART_ACTIONS = { onJob: noop, onMission: noop, onOutput: noop, onSession: noop };

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
            <h3 className="px-xs pb-xxs pt-sm text-caption font-semibold text-muted-foreground">{project}</h3>
            <p className="flex items-center gap-xs px-xs py-xxs text-caption text-subtle-foreground"><DiamondIcon aria-hidden="true" className="size-(--size-icon-sm) fill-current" />{t("desk.name")} · {rows.length}</p>
            <ol className="flex min-w-0 flex-col pl-(--size-tree-indent)">
              {rows.map((job) => <SessionRow key={job.id} mark={<><JobMark job={job} /><AgentMark kind={job.agent_kind ?? null} /></>} title={job.title} line={{ text: jobLine(t, job) }} className={cn(selected === job.id && "bg-sidebar-accent")} />)}
            </ol>
          </section>
        );
      })}
    </aside>
  );
}

function ChatHead() {
  const { t } = useInterfaceTranslation();
  return <>
    <DiamondIcon aria-hidden="true" className="size-(--size-icon-sm) fill-current text-subtle-foreground" />
    <span className="flex-1 font-semibold text-foreground">{t("desk.name")}</span>
    <Button variant="ghost" size="sm" className="h-(--size-control-compact) gap-xxs px-xs text-caption font-normal text-subtle-foreground"><PlusIcon />{t("desk.newConversation")}</Button>
  </>;
}

/** Desk's composer and the bar under it, as a scene sets them. */
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

function Thread({ job, updates, parts, failure = null, outputs = job.outputs, selectedOutput = null, focused = false, draft = "", attachments = [] }: {
  job: DeskJob;
  updates: Parameters<typeof threadLines>[0];
  parts?: Parameters<typeof threadLines>[1];
  failure?: ThreadReadFailure | null;
  outputs?: readonly DeskOutput[];
  selectedOutput?: string | null;
  focused?: boolean;
  draft?: string;
  attachments?: ComposerAttachment[];
}) {
  const [text, setText] = useState(draft);
  const lines = useMemo(() => threadLines(updates, parts), [updates, parts]);
  return (
    <ThreadView
      job={job} lines={lines} failure={failure} mentions={MENTIONS} outputs={outputs} selectedOutput={selectedOutput}
      composer={{ value: text, attachments, focused, onChange: setText, onSend: noop, onAttach: noop, onRemoveAttachment: noop }}
      onTerminal={noop} {...PART_ACTIONS}
    />
  );
}

function ToolColumn({ head, pinned, children }: { head: ReactNode; pinned: boolean; children: ReactNode }) {
  return <>
    <LaneColumnHead pinned={pinned} onExpand={noop} onPin={noop} onClose={noop}>{head}</LaneColumnHead>
    <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-y-auto">{children}</div>
  </>;
}

function ThreadsTitle() {
  const { t } = useInterfaceTranslation();
  return <span className="font-semibold text-foreground">{t("desk.threads.title")}</span>;
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
    // Scene 9's map is Hide's own mission map, drawn from the store as the mission scene draws it.
    const missions = (mission.rest as { missions: { items: { title: string }[] } }).missions.items.map((one) => ({ ...one, title: "Desk 1차" }));
    useShellStore.setState({ rest: { ...mission.rest, missions: { items: missions } } as unknown as SnapshotRest, agents: [], connection: "live" });
    document.documentElement.classList.toggle("dark", theme === "dark");
    document.documentElement.classList.toggle("light", theme === "light");
    document.documentElement.style.setProperty("--interface-scale", params.get("scale") ?? "1");
  }, [mission, theme, params]);

  const story = Object.values(jobs);
  const rail = (threads: boolean, outputs: boolean) => <DeskRail asking={askingCount(story)} unseen={unseenOutputs(story)} threads={threads} outputs={outputs} onThreads={noop} onOutputs={noop} />;
  const sidebarJobs = (ids: string[]) => ids.map((id) => jobs[id]!);
  const threadPath = (...labels: string[]): ThreadPathStep[] => [{ label: t("desk.threads.title"), onPress: noop }, ...labels.map((label, index) => ({ label, onPress: index < labels.length - 1 ? noop : undefined }))];
  const chat = (items: DeskItem[], composer: ReactNode, selectedJob: string | null = null, selectedOutput: string | null = null, extra: Partial<Parameters<typeof DeskChat>[0]> = {}) => (
    <DeskChatColumn head={<ChatHead />} composer={composer}>
      <DeskChat items={items} jobs={jobs} selectedJob={selectedJob} selectedOutput={selectedOutput} {...CHAT_ACTIONS} {...extra} />
    </DeskChatColumn>
  );

  let sidebar: ReactNode = <SceneSidebar jobs={sidebarJobs(["mention", "gallery", "release"])} selected={null} />;
  let body: ReactNode;
  switch (state) {
    case "s1":
      sidebar = <SceneSidebar jobs={[]} selected={null} />;
      body = <DeskRoom pinned={false} rail={rail(false, false)} chat={<DeskChatColumn head={<ChatHead />} composer={<ChatComposer selection={CLAUDE} />}><DeskEmpty working={2} asking={1} onExample={noop} onShowThreads={noop} /></DeskChatColumn>} />;
      break;
    case "s1b":
      body = <DeskRoom pinned={false} rail={rail(false, false)} chat={chat(switchedItems(now), <ChatComposer selection={CODEX} menuOpen="agent" />)} />;
      break;
    case "s2":
      body = <DeskRoom pinned={false} rail={rail(false, false)} chat={chat(handoffItems(now), <ChatComposer selection={CLAUDE} />)} />;
      break;
    case "s3":
      sidebar = <SceneSidebar jobs={story.filter((job) => job.project.label === "hide").slice(0, 3)} selected={null} />;
      body = <DeskRoom pinned={false} rail={rail(true, false)} chat={chat(handoffItems(now).slice(0, 4), <ChatComposer selection={CLAUDE} />)}
        tool={<ToolColumn pinned={false} head={<ThreadsTitle />}><ThreadList jobs={story} expanded={expanded} onOpen={noop} onMission={noop} onToggle={(id) => setExpanded((open) => { const next = new Set(open); if (next.has(id)) next.delete(id); else next.add(id); return next; })} /></ToolColumn>} />;
      break;
    case "s3b":
    case "s3c": {
      const entered = state === "s3c" ? { project: MENU_PROJECTS[0]!, sessions: ENTERED_SESSIONS } : null;
      const menu = <DeskMentionMenu sessions={MENU_SESSIONS} projects={MENU_PROJECTS} entered={entered} active="session:agent-record-now" onActive={noop} onSession={noop} onProject={noop} onBack={noop} />;
      body = <DeskRoom pinned={false} rail={rail(false, false)} chat={chat(mentionItems(now), <ChatComposer selection={CLAUDE} value={state === "s3c" ? "@hide " : "@re"} mention={menu} />)} />;
      break;
    }
    case "s4":
      body = <DeskRoom pinned={false} rail={rail(true, false)} chat={chat(handoffItems(now), <ChatComposer selection={CLAUDE} />, "mention")}
        tool={<ToolColumn pinned={false} head={<ThreadHead path={threadPath("mention 버튼 정리")} onTerminal={noop} />}><Thread job={jobs.mention!} updates={MENTION_THREAD} /></ToolColumn>} />;
      break;
    case "s4b": {
      const thread = conductorThread(jobs);
      body = <DeskRoom pinned={false} rail={rail(true, false)} chat={chat(missionItems(now), <ChatComposer selection={CLAUDE} />, "conductor")}
        tool={<ToolColumn pinned={false} head={<ThreadHead path={threadPath("Desk 1차")} onTerminal={noop} />}><Thread job={jobs.conductor!} updates={thread.updates} parts={thread.parts} outputs={[]} /></ToolColumn>} />;
      break;
    }
    case "s5": {
      const done: DeskJob = { ...jobs.gallery!, state: "done", summary: "라이트와 다크 캡처를 비교했어요", outputs: [OUTPUTS.compareLight, OUTPUTS.compareDark, OUTPUTS.notes] };
      const items: DeskItem[] = [{ kind: "job", id: "c1", job_id: "gallery-done" }];
      body = <DeskRoom pinned={false} rail={rail(true, true)}
        chat={<DeskChatColumn head={<ChatHead />} composer={<ChatComposer selection={CLAUDE} />}><DeskChat items={items} jobs={{ "gallery-done": done }} selectedJob="gallery-done" selectedOutput={OUTPUTS.compareLight.id} {...CHAT_ACTIONS} /></DeskChatColumn>}
        view={<DeskViewColumn tabs={[tab("compare", "compare-light.png", true, <ImageIcon aria-hidden="true" className="size-(--size-icon-sm)" />)]} active="compare" onTab={noop} onKeep={noop} onCloseTab={noop}><PictureView src={PICTURES.light} label="compare-light.png" /></DeskViewColumn>}
        tool={<ToolColumn pinned={false} head={<ThreadHead path={threadPath("갤러리 캡처 비교")} onTerminal={noop} />}><Thread job={done} updates={GALLERY_THREAD.slice(0, 3)} selectedOutput={OUTPUTS.compareLight.id} /></ToolColumn>} />;
      break;
    }
    case "s6": {
      const release = jobs.release!;
      const items: DeskItem[] = [{ kind: "job", id: "c1", job_id: "gallery" }, { kind: "job", id: "c2", job_id: "release" }];
      body = <DeskRoom pinned rail={rail(true, true)}
        chat={chat(items, <ChatComposer selection={CLAUDE} />, "release", OUTPUTS.notes.id)}
        view={<DeskViewColumn tabs={[tab("compare", "compare-light.png", false, <ImageIcon aria-hidden="true" className="size-(--size-icon-sm)" />), tab("notes", "notes.md", true, <FileTextIcon aria-hidden="true" className="size-(--size-icon-sm)" />)]} active="notes" onTab={noop} onKeep={noop} onCloseTab={noop}><div className="p-lg"><MarkdownText text={NOTES_MD} /></div></DeskViewColumn>}
        tool={<ToolColumn pinned head={<ThreadHead path={threadPath("release 노트 초안")} onTerminal={noop} />}><Thread job={release} updates={RELEASE_THREAD} selectedOutput={OUTPUTS.notes.id} /></ToolColumn>} />;
      break;
    }
    case "s7": {
      const items: DeskItem[] = [{ kind: "job", id: "c1", job_id: "gallery" }];
      body = <DeskRoom pinned={false} rail={rail(true, false)} chat={chat(items, <ChatComposer selection={CLAUDE} />, "gallery")}
        tool={<ToolColumn pinned={false} head={<ThreadHead path={threadPath("갤러리 캡처 비교")} onTerminal={noop} />}><Thread job={jobs.gallery!} updates={GALLERY_THREAD} outputs={[]} focused draft="응 다크도, 이 그림처럼 잘라줘" attachments={[{ id: "ref", name: "ref.png", image: true }]} /></ToolColumn>} />;
      break;
    }
    case "s8":
      sidebar = <SceneSidebar jobs={sidebarJobs(["mention", "gallery", "release"])} selected="mention" />;
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
    case "s9":
      body = <DeskRoom pinned={false} rail={rail(false, true)} chat={chat(missionItems(now), <ChatComposer selection={CLAUDE} />, "conductor")}
        view={<DeskViewColumn tabs={[tab("map", t("desk.lane.missionMap", { title: "Desk 1차" }), true, <MapIcon aria-hidden="true" className="size-(--size-icon-sm)" />)]} active="map" onTab={noop} onKeep={noop} onCloseTab={noop}><MissionMap missionId="mission-1" actions={actions} /></DeskViewColumn>}
        tool={<ToolColumn pinned={false} head={<ThreadHead path={threadPath("Desk 1차", "1b 스레드 화면")} onTerminal={noop} />}><Thread job={jobs["1b"]!} updates={WORKER_THREAD} outputs={[]} /></ToolColumn>} />;
      break;
    case "outputs":
      body = <OutputsCriteria />;
      break;
    case "cards":
      body = (
        <div className="grid min-h-0 flex-1 grid-cols-2 content-start gap-md overflow-y-auto p-lg" data-desk-scene-cards="true">
          {everyStateJobs(now).map((job) => <DeskJobCard key={job.id} job={job} {...CHAT_ACTIONS} />)}
          <DeskHandedCard name="record-now" project="hide" onOpen={noop} />
        </div>
      );
      break;
    case "chat-states":
      body = <DeskRoom pinned={false} rail={rail(false, false)} chat={chat(stateItems(now), <ChatComposer agents={AGENTS_PREPARING} selection={CODEX} answering />, null, null, { live: "지금 돌고 있는 일은 세 개예요. mention 버튼 정리는 테스트를 돌리는 중이고" })} />;
      break;
    case "thread-failed":
      body = <DeskRoom pinned={false} rail={rail(true, false)} chat={chat(handoffItems(now), <ChatComposer selection={CLAUDE} />, "release")}
        tool={<ToolColumn pinned={false} head={<ThreadHead path={threadPath("release 노트 초안")} onTerminal={noop} />}><Thread job={jobs.release!} updates={[]} failure="no_record" /></ToolColumn>} />;
      break;
    case "settings":
      sidebar = null;
      body = <div className="w-(--size-settings-sheet-w) max-w-full p-xl"><DeskSettingsTab settings={{ default_agent: "claude", location: "desk", cap: 8 }} agents={AGENTS} onChange={noop} /></div>;
      break;
    case "row-sizes":
      sidebar = null;
      body = <RowSizes />;
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

/** 결과물 기준 (D-65, B30): a session's View list as it is, and the outputs Desk keeps of it. */
function OutputsCriteria() {
  const { t } = useInterfaceTranslation();
  const kept: DeskOutput[] = [OUTPUTS.compareLight, OUTPUTS.galleryPage, OUTPUTS.notes, { ...OUTPUTS.pr, label: "PR 1902" }];
  return (
    <div className="grid min-h-0 flex-1 grid-cols-2 content-start gap-xl p-xl" data-desk-scene-outputs="true">
      <section className="flex min-w-0 flex-col gap-xs">
        <h3 className="text-caption font-medium text-muted-foreground">View</h3>
        <ul className="flex min-w-0 flex-col gap-xxs">
          {SESSION_VIEWS.map((view) => {
            const kind = outputKindOf(view.kind);
            return (
              <li key={view.label} className={cn("flex min-w-0 items-center gap-xs text-caption", kind ? "text-subtle-foreground" : "text-muted-foreground line-through opacity-(--opacity-dimmed)")} data-desk-view-kept={kind ? "true" : "false"}>
                <OutputMark output={{ id: view.label, kind: kind ?? "doc", label: view.label, seen: true, pr_state: kind === "pr" ? "merged" : null }} />
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

/**
 * The operator's question: the session record's child rows (`size="record"`)
 * and the sidebar's rows (`size="row"`) draw the same session differently.
 * Both sizes, side by side, for the same three sessions.
 */
function RowSizes() {
  const now = useMemo(() => Date.now(), []);
  const jobs = Object.values(storyJobs(now)).slice(0, 3);
  const { t } = useInterfaceTranslation();
  return (
    <div className="grid min-h-0 flex-1 grid-cols-2 content-start gap-xl p-xl" data-desk-scene-row-sizes="true">
      <section className="flex w-(--size-sidebar-ideal) min-w-0 flex-col gap-xs">
        <h3 className="text-caption font-semibold text-foreground">A · size=row</h3>
        <p className="text-micro text-muted-foreground">사이드바 · Threads 목록</p>
        <ol className="flex min-w-0 flex-col rounded-md bg-sidebar p-xs">
          {jobs.map((job) => <SessionRow key={job.id} mark={<><JobMark job={job} /><AgentMark kind={job.agent_kind ?? null} /></>} title={job.title} line={{ text: `${job.project.label} · ${jobLine(t, job)}` }} />)}
        </ol>
      </section>
      <section className="flex w-(--size-lane-panel-ideal) min-w-0 flex-col gap-xs">
        <h3 className="text-caption font-semibold text-foreground">B · size=record</h3>
        <p className="text-micro text-muted-foreground">세션 기록의 자식 줄</p>
        <ol className="flex min-w-0 flex-col rounded-md bg-card px-sm">
          {jobs.map((job) => (
            <li key={job.id}>
              <RowButton className="items-start gap-sm px-0 py-sm">
                <SessionRowBody size="record" mark={<JobMark job={job} />} head={<span className="block truncate text-caption font-medium">{job.title}</span>}>
                  <span className="mt-xs flex min-w-0 flex-wrap items-center gap-x-xs gap-y-xxs text-micro text-muted-foreground"><AgentMark kind={job.agent_kind ?? null} />{job.agent_kind === "codex" ? "Codex" : "Claude Code"} · <span>{jobLine(t, job)}</span></span>
                </SessionRowBody>
              </RowButton>
            </li>
          ))}
        </ol>
      </section>
    </div>
  );
}
