// PROPOSAL ONLY (decision-views mockups, 2026-11-09): what a mission decision
// shows the operator to look at, in three structurally distinct layouts, from
// production parts over invented values. `variant`: now (today's card), a
// (볼 자료 rows in the card), b (pictures first), c (one button beside the
// choices); `case`: one, many, none, asked (볼 자료 요청 pressed), surfaces
// (the sidebar row and the phone card). Replaced by the real MissionScene
// state once a layout is chosen.
import { CheckIcon, ChevronDownIcon, EyeIcon, EyeOffIcon, FileDiffIcon, GlobeIcon } from "lucide-react";
import { useLayoutEffect, type ReactNode } from "react";
import { AgentMark } from "../AgentMark";
import { FileMark } from "../components/file-mark";
import { StatusMark } from "../components/status-mark";
import { Button } from "../components/ui/button";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuLabel, DropdownMenuTrigger } from "../components/ui/dropdown-menu";
import { RowButton } from "../components/ui/row-button";
import { TooltipProvider } from "../components/ui/tooltip";
import { fileIcon } from "../fileIcons";
import { cn } from "../lib/utils";

type Look = { id: string; kind: "browser" | "file" | "diff"; label: string; by: string; ago: string; unseen: boolean; picture: "pair" | "phone" | null };

const VIEWS: Look[] = [
  { id: "d970", kind: "browser", label: "전후 비교 · search-screens.html", by: "일꾼 · 검색 화면", ago: "3분 전", unseen: false, picture: "pair" },
  { id: "d971", kind: "file", label: "리뷰 요약.md", by: "Conductor", ago: "1분 전", unseen: true, picture: null },
  { id: "d972", kind: "browser", label: "좁은 폭 비교 · narrow.html", by: "일꾼 · 검색 화면", ago: "5분 전", unseen: true, picture: "phone" },
  { id: "d973", kind: "diff", label: "PR #1504 변경", by: "일꾼 · 검색 화면", ago: "12분 전", unseen: false, picture: null },
  { id: "d974", kind: "file", label: "측정 결과.csv", by: "일꾼 · 검색 화면", ago: "14분 전", unseen: false, picture: null },
];

const QUESTION = "검색 화면: 고치기 전·후 화면 12장을 보고 머지해도 될까요?";
const WHY = "화면이 바뀌는 PR이라 머지 전에 직접 봐야 해요";
const OPTIONS = ["머지해도 됨", "고칠 점 있음"];
const ASKED = "요청함 · Conductor가 비교 페이지를 열면 여기에 떠요";

type Case = "one" | "many" | "none" | "asked" | "surfaces";

export function DecisionViewsProposalScene({ params }: { params: URLSearchParams }) {
  const variant = params.get("variant") ?? "a";
  const shown = (params.get("case") ?? "one") as Case;
  const theme = params.get("theme") ?? "light";
  const width = Number(params.get("width") ?? 560);
  useLayoutEffect(() => {
    document.documentElement.classList.toggle("dark", theme === "dark");
    document.documentElement.classList.toggle("light", theme === "light");
    document.body.classList.add("bg-background");
  }, [theme]);
  const views = shown === "one" ? VIEWS.slice(0, 1) : shown === "many" ? VIEWS : [];
  return (
    <TooltipProvider>
      <div style={{ width }} className="flex min-w-0 flex-col gap-xs bg-background px-md py-md text-foreground" data-proposal={`${variant}-${shown}`}>
        {shown === "surfaces" ? (
          <Surfaces variant={variant} />
        ) : (
          <>
            <h3 className="text-caption font-semibold text-mission-hold">내 차례 1</h3>
            <Card variant={variant} views={views} asked={shown === "asked"} />
          </>
        )}
      </div>
    </TooltipProvider>
  );
}

function Card({ variant, views, asked, phone = false }: { variant: string; views: Look[]; asked: boolean; phone?: boolean }) {
  return (
    <div className="flex min-w-0 flex-col gap-xs rounded-md border border-mission-hold bg-card px-sm py-xs" data-proposal-card={variant}>
      <div className="flex min-w-0 items-center gap-sm">
        <StatusMark mark="turn" />
        <span className={cn("min-w-0 flex-1 text-body font-semibold", phone ? "break-keep" : "truncate")}>{QUESTION}</span>
        {phone ? null : <span className="max-w-(--size-flow-title) shrink-0 truncate rounded-xs bg-muted px-xs text-caption text-mission-hold">검색 화면 · 판단 대기</span>}
        {phone ? null : <span className="shrink-0 text-caption text-muted-foreground tabular-nums">8분 기다림</span>}
      </div>
      <p className="min-w-0 break-words text-caption text-muted-foreground">{WHY}</p>
      {variant === "a" ? <LookRows views={views} asked={asked} /> : null}
      {variant === "b" ? <LookPictures views={views} asked={asked} /> : null}
      <div className="flex min-w-0 flex-wrap items-center gap-xs">
        {variant === "c" ? <LookButton views={views} asked={asked} open={!phone} /> : null}
        {OPTIONS.map((option) => (
          <Button key={option} size="sm" variant="secondary" className="max-w-full"><span className="min-w-0 truncate">{option}</span></Button>
        ))}
        <Button size="sm" variant="ghost">직접 답하기…</Button>
      </div>
    </div>
  );
}

function Mark({ view }: { view: Look }) {
  if (view.kind === "browser") return <GlobeIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-muted-foreground" />;
  if (view.kind === "diff") return <FileDiffIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-muted-foreground" />;
  return <FileMark icon={fileIcon(view.label)} className="size-(--size-icon-sm)" />;
}

/** A: the Views as rows inside the card, between the why and the choices. */
function LookRows({ views, asked }: { views: Look[]; asked: boolean }) {
  const shown = views.slice(0, 3);
  const more = views.length - shown.length;
  return (
    <div className="flex min-w-0 flex-col" data-proposal-look="rows">
      <span className="text-caption text-muted-foreground">{views.length > 0 ? `볼 자료 ${views.length}` : "볼 자료"}</span>
      {views.length === 0 ? (
        <Missing asked={asked} />
      ) : (
        <ul className="flex min-w-0 flex-col">
          {shown.map((view) => (
            <li key={view.id} className="min-w-0">
              <RowButton className="gap-xs px-xxs py-xxs text-body">
                <Mark view={view} />
                <span className="min-w-0 flex-1 truncate">{view.label}</span>
                {view.unseen ? <StatusMark mark="look" label="볼 것" /> : null}
                <span className="shrink-0 text-caption text-muted-foreground">{view.by} · {view.ago}</span>
                <span className="ml-xs shrink-0 text-caption font-semibold text-subtle-foreground">열기 ›</span>
              </RowButton>
            </li>
          ))}
          {more > 0 ? <li><Button variant="text" size="inline" className="px-xxs text-caption text-muted-foreground">+{more} 더 보기</Button></li> : null}
        </ul>
      )}
    </div>
  );
}

function Missing({ asked }: { asked: boolean }) {
  return asked ? (
    <p className="flex min-w-0 items-center gap-xs py-xxs text-caption text-muted-foreground" data-proposal-asked="true">
      <CheckIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-mission-done" />
      <span className="min-w-0 truncate">{ASKED}</span>
    </p>
  ) : (
    <p className="flex min-w-0 items-center gap-xs py-xxs text-caption text-muted-foreground" data-proposal-missing="true">
      <EyeOffIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0" />
      <span className="min-w-0 truncate">연결된 볼 자료가 없어요</span>
      <Button variant="text" size="inline" className="shrink-0 font-semibold text-foreground">볼 자료 요청</Button>
    </p>
  );
}

/** B: the pages as pictures first, other Views as rows under them. */
function LookPictures({ views, asked }: { views: Look[]; asked: boolean }) {
  if (views.length === 0) {
    return (
      <div className="flex min-w-0 items-center justify-center gap-sm rounded-sm border border-dashed py-sm border-border bg-muted text-caption text-muted-foreground" data-proposal-look="empty">
        {asked ? (
          <span className="flex items-center gap-xs"><CheckIcon aria-hidden="true" className="size-(--size-icon-sm) text-mission-done" />{ASKED}</span>
        ) : (
          <>
            <span className="flex items-center gap-xs"><EyeOffIcon aria-hidden="true" className="size-(--size-icon-sm)" />연결된 볼 자료가 없어요</span>
            <Button size="sm" variant="secondary">볼 자료 요청</Button>
          </>
        )}
      </div>
    );
  }
  const pictures = views.filter((view) => view.picture);
  const rows = views.filter((view) => !view.picture).slice(0, 2);
  const more = views.length - pictures.length - rows.length;
  return (
    <div className="flex min-w-0 flex-col gap-xs" data-proposal-look="pictures">
      <ul className={cn("grid min-w-0 gap-xs", pictures.length === 1 ? "grid-cols-1" : "grid-cols-2")}>
        {pictures.map((view) => (
          <li key={view.id} className="flex min-w-0 flex-col gap-xxs">
            <button type="button" className="relative flex aspect-[5/2] w-full min-w-0 overflow-hidden rounded-sm border border-border bg-muted outline-none hover:border-ring">
              <Still kind={view.picture!} />
              {view.unseen ? <StatusMark mark="look" className="absolute top-xs right-xs" /> : null}
            </button>
            <span className="flex min-w-0 items-center gap-xs text-caption">
              <span className="min-w-0 flex-1 truncate">{view.label}</span>
              <span className="shrink-0 text-muted-foreground">{view.ago}</span>
            </span>
          </li>
        ))}
      </ul>
      {rows.length > 0 ? (
        <ul className="flex min-w-0 flex-col">
          {rows.map((view) => (
            <li key={view.id} className="min-w-0">
              <RowButton className="gap-xs px-xxs py-xxs text-caption">
                <Mark view={view} />
                <span className="min-w-0 flex-1 truncate text-subtle-foreground">{view.label}</span>
                {view.unseen ? <StatusMark mark="look" /> : null}
                <span className="shrink-0 text-micro text-muted-foreground">{view.by} · {view.ago}</span>
              </RowButton>
            </li>
          ))}
          {more > 0 ? <li><Button variant="text" size="inline" className="px-xxs text-micro text-muted-foreground">+{more}</Button></li> : null}
        </ul>
      ) : null}
    </div>
  );
}

/** A page's still, drawn small: the before and after columns, or a narrow phone pair. */
function Still({ kind }: { kind: "pair" | "phone" }) {
  const column = (label: string, tone: string) => (
    <span className="flex min-w-0 flex-1 flex-col gap-xxs rounded-xs border border-border bg-background p-xs">
      <span className={cn("text-micro font-semibold", tone)}>{label}</span>
      <span className="h-xs w-4/5 rounded-xs bg-border" />
      <span className="h-xs w-3/5 rounded-xs bg-border" />
      <span className="h-xs w-full rounded-xs bg-border" />
      <span className="h-xs w-2/5 rounded-xs bg-border" />
    </span>
  );
  if (kind === "phone") {
    return (
      <span className="flex size-full items-center justify-center gap-md p-xs">
        <span className="flex h-full w-1/5 flex-col gap-xxs rounded-xs border border-border bg-background p-xxs"><span className="h-xs rounded-xs bg-border" /><span className="h-xs w-3/5 rounded-xs bg-border" /><span className="h-xs rounded-xs bg-border" /></span>
        <span className="flex h-full w-1/5 flex-col gap-xxs rounded-xs border border-border bg-background p-xxs"><span className="h-xs rounded-xs bg-mission-done" /><span className="h-xs w-3/5 rounded-xs bg-border" /><span className="h-xs rounded-xs bg-border" /></span>
      </span>
    );
  }
  return (
    <span className="flex size-full min-w-0 flex-col gap-xxs p-xs text-left">
      <span className="text-micro text-muted-foreground">바뀐 것 · 확인할 것 3 · 좋은 상태</span>
      <span className="flex min-h-0 min-w-0 flex-1 gap-xs">{column("전", "text-muted-foreground")}{column("후", "text-mission-done")}</span>
    </span>
  );
}

/** C: one button at the head of the choices: the View itself, a menu of several, or the request. */
function LookButton({ views, asked, open }: { views: Look[]; asked: boolean; open: boolean }) {
  const divider = <span aria-hidden="true" className="mx-xxs h-(--size-icon-sm) w-px shrink-0 bg-border" />;
  if (views.length === 0) {
    return (
      <>
        {asked ? (
          <span className="flex shrink-0 items-center gap-xxs text-caption text-muted-foreground"><CheckIcon aria-hidden="true" className="size-(--size-icon-sm) text-mission-done" />볼 자료 요청함</span>
        ) : (
          <Button size="sm" variant="outline"><EyeOffIcon aria-hidden="true" />볼 자료 요청</Button>
        )}
        {divider}
      </>
    );
  }
  if (views.length === 1) {
    const view = views[0]!;
    return (
      <>
        <Button size="sm" variant="outline" className="max-w-(--size-flow-title)"><EyeIcon aria-hidden="true" /><span className="min-w-0 truncate">전후 비교 열기</span></Button>
        {view.unseen ? <StatusMark mark="look" /> : null}
        {divider}
      </>
    );
  }
  const unseen = views.filter((view) => view.unseen).length;
  return (
    <>
      <DropdownMenu open={open} modal={false}>
        <DropdownMenuTrigger asChild>
          <Button size="sm" variant="outline"><EyeIcon aria-hidden="true" />볼 자료 {views.length}{unseen > 0 ? <StatusMark mark="look" /> : null}<ChevronDownIcon aria-hidden="true" /></Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start" className="min-w-(--size-flow-title)">
          <DropdownMenuLabel>이 결정에 연결된 볼 자료</DropdownMenuLabel>
          {views.map((view) => (
            <DropdownMenuItem key={view.id}>
              <Mark view={view} />
              <span className="min-w-0 flex-1 truncate">{view.label}</span>
              {view.unseen ? <StatusMark mark="look" /> : null}
              <span className="shrink-0 text-caption text-muted-foreground">{view.ago}</span>
            </DropdownMenuItem>
          ))}
        </DropdownMenuContent>
      </DropdownMenu>
      {divider}
    </>
  );
}

/** The decision where else it stands: the Conductor's sidebar row and the phone's mission page. */
function Surfaces({ variant }: { variant: string }) {
  const views = VIEWS.slice(0, 2);
  return (
    <div className="flex min-w-0 items-start gap-xl">
      <Frame title="사이드바 · 내 차례">
        <div className="flex min-w-0 flex-col gap-xxs rounded-sm bg-sidebar px-xs py-xs text-body" style={{ width: 280 }}>
          <span className="flex min-w-0 items-center gap-xs">
            <StatusMark mark="turn" />
            <AgentMark kind="claude" />
            <span className="min-w-0 flex-1 truncate">검색 개선 Conductor</span>
            <span className="shrink-0 font-mono text-caption text-muted-foreground">8분</span>
          </span>
          <span className="flex min-w-0 items-center gap-xs text-caption text-subtle-foreground" style={{ paddingLeft: 38 }}>
            <span className="min-w-0 truncate">{QUESTION}</span>
            <span className="flex shrink-0 items-center gap-xxs text-muted-foreground"><EyeIcon aria-hidden="true" className="size-(--size-icon-sm)" />2</span>
          </span>
        </div>
      </Frame>
      <Frame title="폰 · 미션">
        <div className="flex min-w-0 flex-col gap-xs rounded-lg border border-border bg-background p-sm" style={{ width: 320 }}>
          <span className="text-headline font-semibold">검색 개선</span>
          <Card variant={variant} views={views} asked={false} phone />
        </div>
      </Frame>
    </div>
  );
}

function Frame({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="flex min-w-0 flex-col gap-xs">
      <h3 className="text-caption text-muted-foreground">{title}</h3>
      {children}
    </section>
  );
}
