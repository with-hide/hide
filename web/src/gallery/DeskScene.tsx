// Desk 1차 (PRD session-docs-and-assistant) for the operator's review before
// any of it is wired: every scene of the final design (proposal-v2: 1, 2,
// 2-1, 3-7) drawn from the real Desk parts in `components/desk` over
// invented values (`deskSceneData.ts`), plus the states it leaves out (the
// agent and @ menus, Hide's parts in a thread, the pane's way back, a
// mission, every chat notice, a thread that cannot be read, Settings ›
// Desk). The sidebar column is a stand-in built from the shared row parts;
// the real sidebar is not changed. Nothing leaves the page.
import { DiamondIcon, FileTextIcon, ImageIcon, MapIcon, PlayIcon } from "lucide-react";
import { useLayoutEffect, useMemo, useState, type ReactNode } from "react";
import { createActions } from "../actions";
import { AgentMark } from "../AgentMark";
import { DeskAgentBar, DeskComposer, DeskPermissionMenu } from "../components/desk/desk-composer";
import { sampleShot } from "../components/sample-shot";
import { DeskApprovalCard, DeskChat, DeskEmpty, DeskHandedLine, type DeskChatActions } from "../components/desk/desk-chat";
import { DeskPaneReturn } from "../components/desk/desk-entry";
import { DeskJobCard, JobMark, OutputMark } from "../components/desk/desk-job-card";
import { DeskChatColumn, DeskChatHead, DeskRoom, DeskTabPanel, type PanelTab } from "../components/desk/desk-room";
import { SessionAskCard, SessionHead, SessionRecordFoot } from "../components/desk/desk-session";
import { LineButton } from "../LinkSessions";
import { OutputShelf } from "../components/desk/output-shelf";
import { FileDropZone } from "../components/desk/file-drop";
import { FilePathChip } from "../components/desk/file-path-chip";
import { PreviewTileView, type TileFile } from "../components/desk/preview-tile";
import type { Preview } from "../filePreview";
import { MarkdownText } from "../MarkdownText";
import { SidebarHeader } from "../components/sidebar-header";
import { DeskMentionMenu } from "../components/desk/desk-mention-menu";
import { DeskOutputs } from "../components/desk/desk-outputs";
import { DeskSettingsTab } from "../components/desk/desk-settings";
import { SessionCard, SessionChip, SessionChipProvider } from "../components/desk/session-chip";
import { ThreadView } from "../components/desk/thread-view";
import { SessionRow } from "../components/session-row";
import { StatusMark } from "../components/status-mark";
import { Separator } from "../components/ui/separator";
import { TooltipProvider } from "../components/ui/tooltip";
import { askingCount, deskFavorites, outputKindOf, recordCard, threadLines, type DeskAgent, type DeskItem, type DeskJob, type DeskMention, type DeskQueuedMessage, type DeskOutput, type DeskPermission, type DeskSelection, type DeskSessionRef, type ThreadReadFailure, type WorkingPhase } from "../desk";
import { keepSent, mentionTree, type ComposerTray, type TrayFile } from "../deskComposer";
import { useInterfaceTranslation } from "../i18n/client";
import { requireInterfaceLanguage } from "../i18n/locale";
import { useFollowInterfaceLanguage } from "../i18n/translator";
import { MissionMap } from "../MissionMap";
import { cn } from "../lib/utils";
import { useShellStore } from "../store";
import type { DeskRecord, SessionPull, SnapshotRest } from "../snapshot";
import { missionFixture } from "./MissionScene";
import {
  AGENTS, AGENTS_PREPARING, FAVORITE_MODELS, cardSceneItems, LONG_CARD, PLAN_EXIT_OPTIONS, WAITING_CARD, CHIP_PROJECT, CHIP_SESSION, CHIP_TEXT, chipItems, chipsScene, endedScene, conductorThread, deskJob, enteredSessions, everyStateJobs, GALLERY_DONE_THREAD, galleryThread, handoffItems, MENTIONS, MENU_PROJECTS, menuSessions,
  mentionItems, missionItems, OUTPUTS, PICTURES, RECORD_THREAD, SESSION_VIEWS, SESSIONS, stateItems, statusItems, storyJobs, switchedItems, WORKER_THREAD,
} from "./deskSceneData";

/** Each scene, with the proposal-v2 scene it pairs with or the PRD behaviors it adds. */
export const DESK_SCENES = [
  { state: "s1", label: "1 빈 Desk (B1-B4)" },
  { state: "s2", label: "2 대화: 맡기기 (B7, B16, B24)" },
  { state: "s2b", label: "2-1 현황 묻기 (B7, D-24)" },
  { state: "s3", label: "3 카드 상태 모음 (B16-B24, B36-B39)" },
  { state: "s4", label: "4 Desk 옆에 연 세션 (B2, B6, B8, B9)" },
  { state: "s5", label: "5 세션이 물을 때: 에이전트 자신의 선택 카드 (B14, B34)" },
  { state: "s7", label: "7 결과물이 세션 옆 탭 패널에 (B16-B21)" },
  { state: "agent", label: "에이전트 고르기 · 바꿈 (B4, B8, B9, B44)" },
  { state: "agent-preparing", label: "Desk를 켠 직후 · 어댑터 받는 중 (B44, D-80)" },
  { state: "permissions", label: "권한: 다 허용 ⌄ 메뉴를 연 모습 (피드백 1, A2)" },
  { state: "permissions-agent", label: "권한: 에이전트대로를 고른 뒤 (피드백 1, A2)" },
  { state: "mention", label: "@ 메뉴: 부모만, 물어보는 자식은 펼침 (B18, B48)" },
  { state: "mention-open", label: "@ 메뉴: ▾ 하위 펼침 (B48)" },
  { state: "mention-search", label: "@리뷰: 부모 먼저, 그다음 하위 세션 (B48)" },
  { state: "mention-project", label: "@ 프로젝트에서 ⇥ (B18)" },
  { state: "mention-empty", label: "@ 맞는 것이 없을 때" },
  { state: "composer-chips", label: "입력창 칩 · 보낸 말 칩 (B49)" },
  { state: "attachments", label: "첨부: 입력칸 타일, 20MB 넘는 파일, 올리는 중, 보낸 말 (desk-2-ui B41, B42)" },
  { state: "parts", label: "스레드 속 Hide 부품 (B26)" },
  { state: "pane", label: "터미널로 ⌘J, 나도 참여 중 (B35, B36)" },
  { state: "mission", label: "미션: 지도 + 워커 스레드 (B34)" },
  { state: "outputs", label: "결과물 기준 (B30)" },
  { state: "chat-states", label: "채팅 상태: 승인 답 (항상 허용, 한 줄과 거절), 알림 줄, 생각 중 · 중단 (B5-B8, B11, B13-B15, B47)" },
  { state: "thread-states", label: "스레드 읽기 실패 · 읽는 중 · 빈 · 연결 안 됨 (B23, B27)" },
  { state: "chips", label: "답 속 칩 전부 · 흐린 칩 · 칩 상세 카드 (B50, D-86)" },
  { state: "ended", label: "끝난 세션: 기록 칩 · 상세 카드 · 기록 읽기 · 이어서 하기 (frame J, K)" },
  { state: "drop", label: "파일을 Desk 대화 칸 한가운데로 끌어온 순간: 입력칸 테두리만 (피드백 2, L 가)" },
  { state: "drop-session", label: "파일을 옆 세션 패널로 끌어온 순간: 그 입력칸 테두리만 (피드백 2, L 나)" },
  { state: "tiles", label: "종류별 미리보기 타일 · 못 만들 때의 타일 (피드백 2, M [1], 라)" },
  { state: "file-chips", label: "답 속 파일 칩: 프로젝트 안 · 밖 ↗ · 없는 경로 · 코드 (피드백 2, M [2], 가)" },
  { state: "settings", label: "Settings › Labs › Desk (B4, B19, B21)" },
  { state: "settings-off", label: "Settings › Labs › Desk 꺼짐, 기본값 (D-76, B41)" },
] as const;

type SceneState = (typeof DESK_SCENES)[number]["state"];

const noop = () => undefined;
/** A copy the gallery takes as done, so `복사함 ✓` shows. */
const copied = () => Promise.resolve(true);

const CHAT_ACTIONS: DeskChatActions = {
  onOpen: noop, onAnswer: noop, onCancel: noop, onRequeue: noop, onRetry: noop, onOutput: noop, onMission: noop,
  onApproval: noop, onQueuedRemove: noop, onNoticeAction: noop, onCopy: copied, onMention: noop, onHanded: noop,
};

const PART_ACTIONS = { onMission: noop, outputOf: (output: DeskOutput) => output, onOutput: noop, onSession: noop, onCopy: copied };

/** A session the scene names, as its chip draws it. */
const sceneSession = (name: string): DeskSessionRef => Object.values(SESSIONS).find((session) => session.name === name) ?? { name, mark: null, agent_kind: null };

/** The stand-in sidebar: the ◆ Desk row, then each project's ◆ Desk group as the shared task rows (B3, B20). */
function SceneSidebar({ jobs, selected }: { jobs: readonly DeskJob[]; selected: string | null }) {
  const { t } = useInterfaceTranslation();
  const projects = [...new Set(jobs.map((job) => job.project.label))];
  return (
    <aside className="flex w-(--size-sidebar-ideal) shrink-0 flex-col gap-xs border-r border-sidebar-border bg-sidebar px-xs py-sm text-sidebar-foreground" data-desk-scene-sidebar="true">
      <SidebarHeader desk={{ selected: selected === null, asking: askingCount(jobs), chord: null, onOpen: noop }} searchChord={null} onSearch={noop} />
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
  return <DeskChatHead onNewConversation={noop} onLeave={noop} leaveTo="herdr-ide › Windows 파일 링크 버그" leaveChord="⌃⇥" />;
}

/** Desk's composer with its bar in the box's foot, as a scene sets them. */
function ChatComposer({ agents = AGENTS, selection, menuOpen = null, value = "", mentions = [], answering = false, mention = null, files = [], permission = "allow_all" }: { agents?: DeskAgent[]; selection: DeskSelection; menuOpen?: "agent" | "permissions" | null; value?: string; mentions?: DeskMention[]; answering?: boolean; mention?: ReactNode; files?: TrayFile[]; permission?: DeskPermission }) {
  const { t } = useInterfaceTranslation();
  const [text, setText] = useState(value);
  return (
    <DeskComposer value={text} draft={{ text, mentions }} label={t("desk.composer.label")} placeholder={t("desk.composer.placeholder")} tray={sceneTray(files)} answering={answering} focused={mention !== null} menu={mention} menuOpen={mention !== null} onChange={setText} onSend={noop} onStop={noop}>
      <DeskPermissionMenu permission={permission} agent={agents.find((agent) => agent.id === selection.agent)?.label ?? null} open={menuOpen === "permissions" ? true : undefined} onPermission={noop} />
      <Separator orientation="vertical" className="mx-xxs data-[orientation=vertical]:h-(--size-icon)" />
      <DeskAgentBar agents={agents} selection={selection} favorites={deskFavorites(FAVORITE_MODELS, agents, selection)} open={menuOpen === "agent" ? "agent" : null} onAgent={noop} onFavorite={noop} onModel={noop} onEffort={noop} onAgentSettings={noop} />
    </DeskComposer>
  );
}

const CLAUDE: DeskSelection = { agent: "claude", model: "opus", effort: "high" };
const CODEX: DeskSelection = { agent: "codex", model: "gpt", effort: "medium" };

/** A tray that holds `files` as a scene sets them. */
function sceneTray(files: readonly TrayFile[]): ComposerTray {
  return { files, add: noop, remove: noop, outgoing: () => null, clear: noop };
}

/** A sent picture, so its bubble draws it as this window's own send would. */
function samplePicture(): Blob {
  const data = sampleShot();
  return new Blob([decodeURIComponent(data.slice(data.indexOf(",") + 1))], { type: "image/svg+xml" });
}

/** A card's pull requests as the session's PR list draws them on the shelf. */
function scenePulls(job: DeskJob): SessionPull[] {
  return job.pull_requests.map((pull) => ({ url: `https://github.com/with-hide/hide/pull/${pull.number}`, number: pull.number, title: `${job.title} #${pull.number}`, state: pull.state, own: true, holders: [], stale: false, last_read_at_unix_ms: null }));
}

/** A session open beside Desk: its head and outputs shelf, its conversation, and the question Hide found above the box when it asks. */
function ThreadColumn({ job, updates, parts, reading = false, failure = null, selectedOutput = null, focused = false, draft = "", files = [] }: {
  job: DeskJob;
  updates: Parameters<typeof threadLines>[0];
  parts?: Parameters<typeof threadLines>[1];
  reading?: boolean;
  failure?: ThreadReadFailure | null;
  selectedOutput?: string | null;
  focused?: boolean;
  draft?: string;
  files?: TrayFile[];
}) {
  const [text, setText] = useState(draft);
  const actions = useMemo(() => createActions(() => true), []);
  const lines = useMemo(() => threadLines(updates, parts, sceneSession), [updates, parts]);
  // A question is one Hide found in the conversation (the line above the box); an approval is the agent's own screen asking (its choice card).
  const asks = useMemo(() => (job.state === "asking" && job.ask === "question" && job.question ? [
    { id: `${job.id}:earlier`, text: "라이트 캡처는 6장 모두 찍을까요?", at_unix_ms: Date.now() - 12 * 60_000 },
    { id: `${job.id}:ask`, text: job.question, at_unix_ms: Date.now() - 3 * 60_000 },
  ] : []), [job]);
  const choice = job.state === "asking" && job.ask === "approval" && job.question
    ? <SessionAskCard ask={{ kind: "permission", turn: `${job.id}:turn`, since_ms: 2 * 60_000, command: job.question, choices: [{ id: "1", label: "Yes" }, { id: "2", label: "Yes, and don't ask again for this command" }, { id: "3", label: "No, and tell Codex what to do differently" }], answerable: true }} disabled={false} waited="2분" picked={null} answer={null} error={null} onPick={noop} onAnswer={noop} onTerminal={noop} onWrite={noop} />
    : null;
  return <>
    <SessionHead paneId={job.pane_id ?? job.id} title={job.title} job={job} row={null} pane={null} machine={job.machine} disconnected={job.disconnected} sleeping={job.state === "sleeping"} explorerOpen={false} filesChord="⇧⌘E" terminalChord="⌘J" deskActions={actions} onFiles={noop} onTerminal={noop} onOwnTab={noop} onClose={noop} />
    <OutputShelf outputs={job.outputs} pulls={scenePulls(job)} selected={selectedOutput} onOpen={noop} onPull={noop} />
    <ThreadView
      job={job} lines={lines} reading={reading} failure={failure} mentions={MENTIONS} startedByDesk
      composer={{ value: text, tray: sceneTray(files), focused, onChange: setText, onSend: noop }}
      ask={choice}
      asks={asks}
      onAsk={noop}
      onTerminal={noop} {...PART_ACTIONS}
    />
  </>;
}

/** A session whose pane is gone, read from the record Hide keeps (frame K): no composer, no terminal, 이어서 하기 at its foot. */
function EndedColumn({ record }: { record: DeskRecord }) {
  const actions = useMemo(() => createActions(() => true), []);
  const job = useMemo(() => recordCard(record), [record]);
  const lines = useMemo(() => threadLines(RECORD_THREAD, {}, sceneSession), []);
  const resume = <LineButton icon={<PlayIcon aria-hidden="true" />} label="이어서 하기" blocked={null} busy={false} onClick={noop} data="resume" id={record.session.session_id} />;
  return <>
    <SessionHead paneId={record.pane_id} title={record.title} job={job} row={null} pane={null} machine={null} disconnected={false} sleeping={false} ended={{ pulls: record.pull_requests }} explorerOpen={false} filesChord={null} terminalChord={null} deskActions={actions} onFiles={noop} onTerminal={noop} onOwnTab={noop} onClose={noop} />
    <ThreadView job={job} lines={lines} mentions={MENTIONS} startedByDesk composer={{ value: "", tray: sceneTray([]), onChange: noop, onSend: noop }} foot={<SessionRecordFoot endedAt={record.ended_at_unix_ms} resume={resume} blocked={null} />} {...PART_ACTIONS} />
  </>;
}

/** An image output in the 보기 column: the picture, fitted as the image viewer fits it. */
function PictureView({ src, label }: { src: string; label: string }) {
  return <div className="flex min-h-0 flex-1 items-center justify-center p-md"><img src={src} alt={label} className="max-h-full max-w-full rounded-sm border border-border object-contain" /></div>;
}

const tab = (id: string, label: string, preview: boolean, icon: ReactNode): PanelTab => ({ id, label, project: null, preview, icon });

const WIDTHS = { view_width: null, session_width: null };

/** Desk's room as a scene sets it. */
function Room({ chat, session = null, panel = null }: { chat: ReactNode; session?: ReactNode; panel?: ReactNode }) {
  return <DeskRoom chat={chat} session={session} panel={panel} widths={WIDTHS} onDim={noop} onWidth={noop} />;
}

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
  useLayoutEffect(() => {
    // The mission scene's map is Hide's own mission map, drawn from the store as the mission scene draws it.
    const missions = (mission.rest as { missions: { items: { title: string }[] } }).missions.items.map((one) => ({ ...one, title: "Desk 1차" }));
    useShellStore.setState({ rest: { ...mission.rest, missions: { items: missions } } as unknown as SnapshotRest, agents: [], connection: "live" });
    document.documentElement.classList.toggle("dark", theme === "dark");
    document.documentElement.classList.toggle("light", theme === "light");
    document.documentElement.style.setProperty("--interface-scale", params.get("scale") ?? "1");
  }, [mission, theme, params]);

  const pick = (...ids: string[]) => ids.map((id) => jobs[id]!);
  const chat = (items: DeskItem[], composer: ReactNode, options: { jobs?: Record<string, DeskJob>; selectedJob?: string | null; selectedOutput?: string | null; live?: string | null; working?: WorkingPhase; queued?: DeskQueuedMessage[] } = {}) => (
    <DeskChatColumn head={<ChatHead />} composer={composer}>
      <DeskChat items={items} jobs={options.jobs ?? jobs} agents={AGENTS} selectedJob={options.selectedJob ?? null} selectedOutput={options.selectedOutput ?? null} live={options.live ?? null} working={options.working ? { phase: options.working, since: now - 3_000 } : null} queued={options.queued ?? []} {...CHAT_ACTIONS} />
    </DeskChatColumn>
  );
  let sidebar: ReactNode = <SceneSidebar jobs={pick("mention", "gallery", "release")} selected={null} />;
  let body: ReactNode;
  switch (state) {
    case "s1": {
      // A new conversation while threads from before still run: the sidebar, the rail and the empty Desk count the same ones.
      const running = pick("mention", "gallery", "release");
      sidebar = <SceneSidebar jobs={running} selected={null} />;
      body = <Room chat={<DeskChatColumn head={<ChatHead />} composer={<ChatComposer selection={CLAUDE} />}><DeskEmpty working={running.filter((job) => job.state === "working").length} asking={askingCount(running)} onExample={noop} /></DeskChatColumn>} />;
      break;
    }
    case "s2":
      body = <Room chat={chat(handoffItems(now), <ChatComposer selection={CLAUDE} />, { working: "sessions", selectedJob: "gallery" })} />;
      break;
    case "s2b":
      body = <Room chat={chat(statusItems(now), <ChatComposer selection={CLAUDE} />)} />;
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
        { kind: "reply", mentions: [], missions: [], agent: "claude", id: 2, text: "갤러리 비교를 새 스레드로 띄웠어요.", at_ms: now, state: "done", thought_ms: null, steps: { sessions: 0, files: 0, letters: 0, starts: 0, other: 0 } },
        { kind: "job", id: 3, job_id: "gallery" },
      ];
      body = <Room chat={chat(items, <ChatComposer selection={CLAUDE} />, { jobs: { gallery }, selectedJob: "gallery" })}
        session={<ThreadColumn job={gallery} updates={thread.updates} parts={thread.parts} />} />;
      break;
    }
    case "s5": {
      const items: DeskItem[] = [{ kind: "job", id: 1, job_id: "record" }, { kind: "job", id: 2, job_id: "gallery" }];
      body = <Room chat={chat(items, <ChatComposer selection={CLAUDE} />, { selectedJob: "record" })}
        session={<ThreadColumn job={jobs.record!} updates={RECORD_THREAD} focused draft="응 지우고 다시 빌드해" />} />;
      break;
    }
    case "s7": {
      const done = deskJob(jobs.gallery!, { state: "done", ask: null, question: null, summary: "차이 3곳", plan: { done: 5, total: 5 }, outputs: [OUTPUTS.compareLight, OUTPUTS.compareDark, OUTPUTS.notes] });
      const items: DeskItem[] = [{ kind: "job", id: 1, job_id: "gallery" }];
      body = <Room chat={chat(items, <ChatComposer selection={CLAUDE} />, { jobs: { gallery: done }, selectedJob: "gallery", selectedOutput: OUTPUTS.compareDark.id })}
        panel={<DeskTabPanel tabs={[tab("notes", "notes.md", false, <FileTextIcon aria-hidden="true" className="size-(--size-icon-sm)" />), tab("compare", "compare-dark.png", true, <ImageIcon aria-hidden="true" className="size-(--size-icon-sm)" />)]} active="compare" onTab={noop} onKeep={noop} onCloseTab={noop} onClose={noop}><PictureView src={PICTURES.dark} label="compare-dark.png" /></DeskTabPanel>}
        session={<ThreadColumn job={done} updates={GALLERY_DONE_THREAD.updates} parts={GALLERY_DONE_THREAD.parts} selectedOutput={OUTPUTS.compareDark.id} />} />;
      break;
    }
    case "agent":
      body = <Room chat={chat(switchedItems(now), <ChatComposer selection={CODEX} menuOpen="agent" />)} />;
      break;
    case "agent-preparing":
      body = <Room chat={chat([], <ChatComposer agents={AGENTS_PREPARING} selection={CLAUDE} menuOpen="agent" />)} />;
      break;
    case "permissions":
      body = <Room chat={chat(switchedItems(now), <ChatComposer selection={CLAUDE} menuOpen="permissions" />)} />;
      break;
    case "permissions-agent":
      body = <Room chat={chat(switchedItems(now), <ChatComposer selection={CLAUDE} permission="agent" />)} />;
      break;
    case "mention":
    case "mention-open":
    case "mention-search":
    case "mention-empty":
    case "mention-project": {
      const query = state === "mention-search" ? "리뷰" : state === "mention-empty" ? "zzzz" : "";
      const section = state === "mention-empty" ? { sessions: [], projects: [] } : { sessions: menuSessions(now), projects: MENU_PROJECTS };
      const tree = mentionTree(section, query, state === "mention-open" ? { "agent-holder": true } : {});
      const entered = state === "mention-project" ? { project: MENU_PROJECTS[0]!, sessions: enteredSessions(now) } : null;
      const active = state === "mention-project" ? "session:agent-record-now" : state === "mention-empty" ? "" : `session:${(state === "mention-open" ? tree.rows[2] : tree.rows[0] ?? tree.found[0])!.session.id}`;
      const menu = <DeskMentionMenu tree={tree} searching={query !== ""} entered={entered} active={active} onActive={noop} onSession={noop} onProject={noop} onFold={noop} onBack={noop} />;
      body = <Room chat={chat(mentionItems(now), <ChatComposer selection={CLAUDE} value={state === "mention-project" ? "@herdr-ide " : `@${query}`} mention={menu} />)} />;
      break;
    }
    case "composer-chips":
      body = <Room chat={chat(chipItems(now), <ChatComposer selection={CLAUDE} value={CHIP_TEXT} mentions={[CHIP_SESSION, CHIP_PROJECT]} />)} />;
      break;
    case "attachments": {
      // B41, B42 (Doop ⑦-b, ⑦-c): a sent message with its picture and file over the bubble; Desk's box with a picture and a file past
      // the cap, which holds send; the session's box with a file still being read in.
      const stage = "6f1d2c3b-4a5e-4f60-8a7b-9c0d1e2f3a4b-0";
      keepSent(stage, "끊김.png", samplePicture());
      const sent: DeskItem[] = [{ kind: "user", id: 61, text: "이 화면이랑 로그 보고 왜 끊기는지 봐 줘", at_ms: now - 60_000, mentions: [], attachments: [
        { name: "끊김.png", bytes: 182_000, path: `/state/attachments/${stage}-끊김.png` },
        { name: "설치-로그.txt", bytes: 84_000, path: "/state/attachments/6f1d2c3b-4a5e-4f60-8a7b-9c0d1e2f3a4b-1-설치-로그.txt" },
      ] }];
      const shot: TrayFile = { id: "shot", name: "화면.png", size: 182_000, preview: sampleShot(), state: "ready", read: 182_000 };
      const zip: TrayFile = { id: "zip", name: "전체-로그.zip", size: 31 * 1024 * 1024, preview: null, state: "too_large", read: 0 };
      const log: TrayFile = { id: "log", name: "설치-로그.txt", size: 84_000, preview: null, state: "reading", read: 34_000 };
      const thread = conductorThread(jobs);
      body = <Room chat={chat(sent, <ChatComposer selection={CLAUDE} value="이 화면이랑 로그 보고 왜 끊기는지 봐 줘" files={[shot, zip]} />)}
        session={<ThreadColumn job={jobs.conductor!} updates={thread.updates} parts={thread.parts} draft="이 로그도 같이 봐 줘" files={[log]} />} />;
      break;
    }
    case "parts": {
      const thread = conductorThread(jobs);
      body = <Room chat={chat(missionItems(now), <ChatComposer selection={CLAUDE} />, { selectedJob: "conductor" })}
        session={<ThreadColumn job={jobs.conductor!} updates={thread.updates} parts={thread.parts} />} />;
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
      body = <Room chat={chat(missionItems(now), <ChatComposer selection={CLAUDE} />, { selectedJob: "conductor" })}
        panel={<DeskTabPanel tabs={[tab("map", t("desk.lane.missionMap", { title: "Desk 1차" }), true, <MapIcon aria-hidden="true" className="size-(--size-icon-sm)" />)]} active="map" onTab={noop} onKeep={noop} onCloseTab={noop} onClose={noop}><MissionMap missionId="mission-1" actions={actions} /></DeskTabPanel>}
        session={<ThreadColumn job={jobs["1b"]!} updates={WORKER_THREAD} />} />;
      break;
    case "outputs":
      body = <OutputsCriteria />;
      break;
    case "chat-states":
      body = <Room chat={chat(stateItems(now), <ChatComposer agents={AGENTS_PREPARING} selection={CODEX} answering />, { live: "지금 돌고 있는 일은 세 개예요. mention 버튼 정리는 테스트를 돌리는 중이고", queued: [{ id: 900, text: "끝나면 release 노트도 정리해줘", at_ms: now, mentions: [] }] })} />;
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
          {column("disconnected", <ThreadColumn job={off} updates={RECORD_THREAD} />)}
        </div>
      );
      break;
    }
    case "drop": {
      const items: DeskItem[] = [{ kind: "user", id: 1, text: "Windows 설치가 중간에 멈춰. 로그 붙일게", at_ms: now, mentions: [] }, { kind: "reply", mentions: [], missions: [], agent: "claude", id: 2, text: "로그를 주시면 어느 단계에서 멈췄는지 볼게요.", at_ms: now, state: "done", thought_ms: 8_000, steps: { sessions: 6, files: 0, letters: 0, starts: 0, other: 0 } }];
      body = <Room chat={<DeskChatColumn head={<ChatHead />} composer={<ChatComposer selection={CLAUDE} />} tray={sceneTray([])} dragging>{<DeskChat items={items} jobs={jobs} agents={AGENTS} selectedJob={null} selectedOutput={null} live={null} working={null} queued={[]} {...CHAT_ACTIONS} />}</DeskChatColumn>} />;
      break;
    }
    case "drop-session": {
      const thread = galleryThread();
      body = <Room chat={chat([{ kind: "job", id: 1, job_id: "gallery" }], <ChatComposer selection={CLAUDE} />, { selectedJob: "gallery" })}
        session={<FileDropZone tray={sceneTray([])} dragging className="flex min-h-0 min-w-0 flex-1 flex-col"><ThreadColumn job={jobs.gallery!} updates={thread.updates} parts={thread.parts} /></FileDropZone>} />;
      break;
    }
    case "tiles":
      body = <TileStates />;
      break;
    case "file-chips":
      body = <FileChipStates />;
      break;
    case "chips": {
      // Every session the answer names is a chip, the 17th on too; m349 is no longer listed, so it is dimmed with no status.
      const scene = chipsScene(now);
      const open = scene.facts.get(scene.open)!;
      const source = { facts: (session: DeskSessionRef) => (session.pane_id ? scene.facts.get(session.pane_id) ?? null : null), onOpen: noop };
      body = (
        <SessionChipProvider source={source}>
          <Room chat={chat(scene.items, <ChatComposer selection={CLAUDE} />)} />
          <aside className="flex shrink-0 flex-col gap-xs border-l border-border p-lg" data-desk-scene-chip-card="open">
            <h3 className="text-micro text-muted-foreground">칩에 마우스를 올리면</h3>
            <span><SessionChip session={{ name: open.card!.title, mark: open.mark, agent_kind: open.agent_kind, pane_id: scene.open }} onPress={noop} className="ring-1 ring-mention-session" /></span>
            <div className="w-(--size-pr-popover) max-w-full rounded-sm bg-popover text-popover-foreground shadow-overlay">
              <SessionCard job={open.card!} since={open.since} parent={open.parent} onOpen={noop} onParent={noop} />
            </div>
          </aside>
        </SessionChipProvider>
      );
      break;
    }
    case "ended": {
      // The ended chip opens its record beside Desk with the ring; the dimmed one has no record and no press.
      const scene = endedScene(now);
      const ended = scene.facts.get(scene.record.pane_id)!;
      const source = { facts: (session: DeskSessionRef) => (session.pane_id ? scene.facts.get(session.pane_id) ?? null : null), onOpen: noop, open: scene.record.pane_id };
      body = (
        <SessionChipProvider source={source}>
          <Room chat={chat(scene.items, <ChatComposer selection={CLAUDE} />)} session={<EndedColumn record={scene.record} />} />
          <aside className="flex shrink-0 flex-col gap-xs border-l border-border p-lg" data-desk-scene-chip-card="ended">
            <h3 className="text-micro text-muted-foreground">칩에 마우스를 올리면</h3>
            <div className="w-(--size-pr-popover) max-w-full rounded-sm bg-popover text-popover-foreground shadow-overlay">
              <SessionCard job={ended.card!} since={ended.since} parent={null} onOpen={noop} onParent={noop} />
            </div>
          </aside>
        </SessionChipProvider>
      );
      break;
    }
    case "settings":
    case "settings-off":
      sidebar = null;
      // On, one row set to 묻기 so both answers show; off is every default, 바로 on each row.
      body = <div className="w-(--size-settings-sheet-w) max-w-full p-xl"><DeskSettingsTab settings={{ enabled: state === "settings", default_agent: "claude", location: "desk", cap: 8, permission: "allow_all" }} agents={AGENTS} onChange={noop} /></div>;
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
        <h3 className="pt-sm text-micro text-muted-foreground">에이전트 바꿈 · Desk 자신의 승인 (로고 없는 유일한 ! 카드) · 답한 승인</h3>
        <DeskChat items={cardSceneItems()} jobs={{}} agents={AGENTS} {...CHAT_ACTIONS} />
        <h3 className="pt-sm text-micro text-muted-foreground">거절… 을 누르면 (한 줄은 Desk에게, 비우면 그냥 거절)</h3>
        <DeskApprovalCard item={{ ...WAITING_CARD, id: 56 }} onAnswer={noop} defaultRefusal="빌드 말고 타입 검사만 돌려줘" />
        <h3 className="pt-sm text-micro text-muted-foreground">긴 명령은 두 줄로 접힘</h3>
        <DeskApprovalCard item={LONG_CARD} onAnswer={noop} />
        <h3 className="pt-sm text-micro text-muted-foreground">같은 종류의 답이 여럿이면 에이전트가 붙인 이름 그대로 (Claude의 플랜 모드 나가기)</h3>
        <DeskApprovalCard item={{ ...WAITING_CARD, id: 58, card: "other", command: "ExitPlanMode", folder: null, options: PLAN_EXIT_OPTIONS }} onAnswer={noop} />
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

/** Every tile a file or page shows (feedback 2 frame M [1], 라), each from the preview it would make. */
function TileStates() {
  const shot = sampleShot();
  const file = (label: string): TileFile => ({ id: label, label, path: `/repo/${label}`, url: null, device: null, root: "/repo" });
  const picture = (pages: number | null, seconds: number | null): Preview => ({ state: "picture", picture: shot, pages, seconds });
  const made: [TileFile, Preview][] = [
    [file("settings-devices.png"), picture(null, null)],
    [file("rc9-검증-보고서.pdf"), picture(12, null)],
    [{ ...file("withhide.dev/download"), path: null, url: "https://withhide.dev/download" }, picture(null, null)],
    [file("release-notes.md"), { state: "text", title: "1.3.0 릴리스 노트", lines: ["기기 연결이 끊긴 뒤 다시 붙을 때 로그인이 풀리던 문제를 고쳤어요.", "Windows 설치 중 멈추던 단계를 다시 시도해요.", "Desk에 파일을 어디에나 놓을 수 있어요."] }],
    [file("pairing-demo.mp4"), picture(null, 42)],
  ];
  const missing: [TileFile, Preview][] = [
    [file("mini-login.log"), { state: "reading" }],
    [file("rc9-검증-보고서.pdf"), { state: "failed" }],
    [file("full-run.mov"), { state: "too_large", bytes: 1_288_490_189 }],
    [{ ...file("settings-devices.png"), device: "deepbox" }, { state: "offline" }],
    [file("build.ts"), { state: "none" }],
  ];
  const row = (tiles: [TileFile, Preview][], key: string) => (
    <ul className="flex flex-wrap gap-sm" data-desk-scene-tiles={key}>
      {tiles.map(([one, preview], index) => <li key={index}><PreviewTileView file={one} preview={preview} onRetry={noop} onOpen={noop} unseen={index === 1 && key === "made"} /></li>)}
    </ul>
  );
  return (
    <div className="flex flex-col gap-lg p-lg">
      <h3 className="text-caption text-muted-foreground">종류별</h3>
      {row(made, "made")}
      <h3 className="text-caption text-muted-foreground">미리보기를 못 만들 때</h3>
      {row(missing, "missing")}
      <h3 className="text-caption text-muted-foreground">선반 펼친 목록 (3칸)</h3>
      <ul className="grid w-(--size-pr-popover) grid-cols-3 gap-sm" data-desk-scene-tiles="shelf">
        {[...made.slice(0, 4), missing[0]!].map(([one, preview], index) => <li key={index} className="min-w-0"><PreviewTileView file={one} preview={preview} onRetry={noop} onOpen={noop} fill /></li>)}
      </ul>
    </div>
  );
}

/** A Desk answer naming files (feedback 2 frame M [2], 가): a project's file as a chip, an outside one with ↗, a missing path and code as words. */
function FileChipStates() {
  const text = "검증은 다 통과했어요. 보고서는 agents/runs/rc9/rc9-검증-보고서.pdf, 바뀐 점은 release-notes.md 에 정리했고, 문제였던 화면은 `settings-devices.png` 예요. 내려받은 ~/Downloads/crash.log 는 이번 일과 관계없어요. 옛 메모 agents/runs/old/notes.md 는 지웠어요.\n```sh\nopen agents/runs/rc9/rc9-검증-보고서.pdf\n```";
  const project = (written: string, path: string) => ({ token: written, at: [text.indexOf(written)], node: <TooltipProvider><FilePathChip file={{ kind: "project", path, device: "local", root: "/repo" }} onOpen={noop} /></TooltipProvider> });
  const chips = [
    project("agents/runs/rc9/rc9-검증-보고서.pdf", "/repo/agents/runs/rc9/rc9-검증-보고서.pdf"),
    project("release-notes.md", "/repo/release-notes.md"),
    project("`settings-devices.png`", "/repo/settings-devices.png"),
    { token: "~/Downloads/crash.log", at: [text.indexOf("~/Downloads/crash.log")], node: <TooltipProvider><FilePathChip file={{ kind: "outside", path: "/Users/me/Downloads/crash.log" }} onOpen={noop} /></TooltipProvider> },
  ];
  return (
    <div className="flex max-w-(--size-editor-measure) flex-col gap-lg p-lg">
      <MarkdownText text={text} size="chat" chips={chips} />
    </div>
  );
}

