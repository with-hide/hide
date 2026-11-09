// A chapter's card picture on the practice list: a small drawing of the shell
// (the sidebar, a terminal, the lane) that plays the one move the chapter
// teaches, over and over. It draws shapes only, never a name, a number or an
// agent's words, so nothing on it can be taken for the operator's own work.
// A chapter names its picture (`preview` in `chapter.ts`); a new kind of move
// is added here, with its drawing and its loop.

import type { ReactElement } from "react";
import type { PracticePreviewKind } from "./chapter";

const Line = ({ width }: { width: number }) => <i className="pv-ln" style={{ width: `${width}%` }} />;
const Row = ({ width, mark }: { width: number; mark?: "work" | "ask" | "done" }) => <i className="pv-r" style={{ width: `${width}%` }}>{mark ? <b className={`pv-dot ${mark}`} /> : null}</i>;

function Side({ rows = [80, 60, 72] }: { rows?: number[] }) {
  return <div className="pv-side">{rows.map((width, at) => <Row key={at} width={width} />)}</div>;
}

const DRAWINGS: Record<PracticePreviewKind, () => ReactElement> = {
  // A situation typed to the Conductor, its plan of six parts, the parts at work, then one question back.
  mission: () => (
    <>
      <Side rows={[80, 60]} />
      <div className="pv-main">
        <div className="pv-pane pv-full"><i className="pv-type" /></div>
        {[[14, 50], [36, 30], [36, 66], [58, 30], [58, 66], [80, 48]].map(([x, y], at) => (
          <i key={at} className="pv-node" style={{ left: `${x}%`, top: `${y}%`, animationDelay: `${at * 0.12}s` }} />
        ))}
        <i className="pv-ask" />
      </div>
    </>
  ),
  // The three parts of the screen, lit one after another in their zone colors.
  zones: () => (
    <>
      <Side />
      <div className="pv-main pv-with-lane">
        <div className="pv-pane pv-full"><Line width={60} /><Line width={40} /></div>
      </div>
      <div className="pv-lane"><Row width={70} /><Row width={50} /></div>
      <i className="pv-zone side" />
      <i className="pv-zone work" />
      <i className="pv-zone lane" />
    </>
  ),
  // One terminal, then a second beside it.
  split: () => (
    <>
      <Side />
      <div className="pv-main">
        <div className="pv-pane pv-full"><Line width={45} /><Line width={30} /></div>
        <div className="pv-pane pv-half-r pv-slide"><Line width={50} /></div>
      </div>
    </>
  ),
  // The keyboard going back and forth between two terminals.
  switch: () => (
    <>
      <Side />
      <div className="pv-main">
        <div className="pv-pane pv-half-l"><Line width={60} /><Line width={35} /></div>
        <div className="pv-pane pv-half-r"><Line width={50} /><Line width={70} /></div>
        <i className="pv-ring" />
      </div>
    </>
  ),
  // A working row turning to an ask and moving up to the top of the list.
  rows: () => (
    <>
      <div className="pv-side pv-wide">
        <i className="pv-head ask" />
        <span className="pv-gap" />
        <i className="pv-head work" />
        <Row width={82} mark="work" />
        <Row width={74} mark="work" />
        <i className="pv-head done" />
        <Row width={66} mark="done" />
        <i className="pv-r pv-rise" style={{ width: "86%" }}><b className="pv-dot pv-turn" /></i>
      </div>
      <div className="pv-main pv-narrow">
        <div className="pv-pane pv-full"><Line width={60} /><Line width={40} /></div>
      </div>
    </>
  ),
  // A lead whose children open one after another beside it.
  children: () => (
    <>
      <Side rows={[80]} />
      <div className="pv-main">
        <div className="pv-pane pv-half-l"><Line width={70} /><Line width={45} /></div>
        {[0, 1, 2].map((at) => <div key={at} className={`pv-pane pv-kid k${at}`}><Line width={55} /></div>)}
      </div>
    </>
  ),
  // A few letters typed into search, the results, and the one found.
  search: () => (
    <>
      <Side />
      <div className="pv-main"><div className="pv-pane pv-full pv-under" /></div>
      <div className="pv-palette"><i className="pv-query" /></div>
      <i className="pv-hit h0" />
      <i className="pv-hit h1" />
      <i className="pv-hit h2" />
    </>
  ),
  // The lane coming in beside a terminal with the project's week in bars.
  lane: () => (
    <>
      <Side />
      <div className="pv-main pv-with-lane">
        <div className="pv-pane pv-full"><Line width={60} /><Line width={40} /></div>
      </div>
      <div className="pv-lane pv-lane-in">
        <Row width={60} />
        <div className="pv-bars">{[40, 70, 30, 90, 55, 75, 60].map((height, at) => <i key={at} style={{ height: `${height}%`, animationDelay: `${at * 0.06}s` }} />)}</div>
      </div>
    </>
  ),
};

/** The chapter's picture; `playing` runs its move, and a still one holds the move's middle. */
export function PracticePreview({ kind, playing, large = false }: { kind: PracticePreviewKind; playing: boolean; large?: boolean }) {
  const Drawing = DRAWINGS[kind];
  return (
    <div className={`pv${large ? " pv-large" : ""}`} data-practice-preview={kind} data-playing={playing} aria-hidden>
      <Drawing />
    </div>
  );
}

// Every move loops in 6 s; a still picture holds each move 3.9 s into its loop
// (the zones' at the start, where all three are lit), where what it teaches is
// on screen. Colors are the shell's own
// surfaces and the practice's amber, and the zone colors the guide uses.
export const PREVIEW_CSS = `
.pv{position:relative;aspect-ratio:16/9;overflow:hidden;background:var(--sidebar)}
.pv *{animation-duration:6s;animation-iteration-count:infinite;animation-timing-function:ease-in-out}
.pv[data-playing="false"] *{animation-delay:-3.9s!important;animation-play-state:paused}
.pv[data-playing="false"][data-practice-preview="zones"] *{animation-delay:-.6s!important}
@media (prefers-reduced-motion:reduce){.pv *{animation-delay:-3.9s!important;animation-play-state:paused}.pv[data-practice-preview="zones"] *{animation-delay:-.6s!important}}
.pv-large{aspect-ratio:2/1}
.pv i,.pv b{display:block}
.pv-side{position:absolute;inset:0 auto 0 0;width:26%;padding:var(--spacing-sm) var(--spacing-xs);border-right:var(--size-hairline) solid var(--border)}
.pv-side.pv-wide{width:50%}
.pv-main{position:absolute;inset:0 0 0 26%;background:var(--background)}
.pv-main.pv-narrow{left:50%}
.pv-main.pv-with-lane{right:30%}
.pv-r{position:relative;height:var(--spacing-xs);border-radius:var(--radius-xs);background:color-mix(in srgb,var(--foreground) 14%,transparent);margin-bottom:var(--spacing-xs)}
.pv-dot{position:absolute;right:var(--spacing-xxs);top:0;height:100%;aspect-ratio:1;border-radius:50%}
.pv-dot.work{background:var(--practice-zone-view)}
.pv-dot.done{background:var(--practice-zone-work)}
.pv-dot.ask{background:var(--practice-accent)}
.pv-pane{position:absolute;padding:var(--spacing-xs);border-radius:var(--radius-xs);background:var(--card);box-shadow:inset 0 0 0 var(--size-hairline) var(--border)}
.pv-full{inset:var(--spacing-xs)}
.pv-half-l{inset:var(--spacing-xs) calc(50% + var(--spacing-xxs)) var(--spacing-xs) var(--spacing-xs)}
.pv-half-r{inset:var(--spacing-xs) var(--spacing-xs) var(--spacing-xs) calc(50% + var(--spacing-xxs))}
.pv-ln{height:var(--spacing-xxs);border-radius:var(--radius-xs);background:color-mix(in srgb,var(--foreground) 14%,transparent);margin-bottom:var(--spacing-xs)}

.pv-type{height:var(--spacing-xxs);width:55%;border-radius:var(--radius-xs);background:var(--practice-accent);transform-origin:left;animation-name:pv-type}
@keyframes pv-type{0%{transform:scaleX(0)}20%,100%{transform:scaleX(1)}}
.pv-node{position:absolute;width:12%;height:12%;border-radius:var(--radius-xs);background:color-mix(in srgb,var(--foreground) 14%,transparent);opacity:0;animation-name:pv-node}
@keyframes pv-node{0%,24%{opacity:0;transform:scale(.6)}32%{opacity:1;transform:none;background:color-mix(in srgb,var(--foreground) 14%,transparent)}52%{background:var(--practice-zone-view)}80%,100%{opacity:1;background:var(--practice-zone-work)}}
.pv-ask{position:absolute;right:var(--spacing-sm);bottom:var(--spacing-sm);width:26%;height:12%;border-radius:var(--radius-xl);background:var(--practice-accent);opacity:0;animation-name:pv-ask}
@keyframes pv-ask{0%,58%{opacity:0;transform:translateY(40%)}64%,86%{opacity:1;transform:none}94%,100%{opacity:0}}

.pv-lane{position:absolute;inset:0 0 0 auto;width:30%;padding:var(--spacing-sm) var(--spacing-xs);background:var(--sidebar);border-left:var(--size-hairline) solid var(--border)}
.pv-zone{position:absolute;top:var(--spacing-xxs);bottom:var(--spacing-xxs);border-radius:var(--radius-sm);opacity:0;animation-name:pv-zone}
.pv-zone.side{left:var(--spacing-xxs);width:calc(26% - var(--spacing-xs));background:color-mix(in srgb,var(--practice-zone-project) 30%,transparent);box-shadow:inset 0 0 0 var(--size-hairline) var(--practice-zone-project)}
.pv-zone.work{left:calc(26% + var(--spacing-xxs));right:calc(30% + var(--spacing-xxs));background:color-mix(in srgb,var(--practice-zone-work) 30%,transparent);box-shadow:inset 0 0 0 var(--size-hairline) var(--practice-zone-work);animation-delay:2s}
.pv-zone.lane{right:var(--spacing-xxs);width:calc(30% - var(--spacing-xs));background:color-mix(in srgb,var(--practice-zone-view) 30%,transparent);box-shadow:inset 0 0 0 var(--size-hairline) var(--practice-zone-view);animation-delay:4s}
@keyframes pv-zone{0%{opacity:0}6%,28%{opacity:1}34%,100%{opacity:0}}

.pv-slide{animation-name:pv-slide}
@keyframes pv-slide{0%,22%{transform:translateX(110%);opacity:0}36%,78%{transform:none;opacity:1}92%,100%{transform:translateX(110%);opacity:0}}

.pv-ring{position:absolute;inset:var(--spacing-xs) calc(50% + var(--spacing-xxs)) var(--spacing-xs) var(--spacing-xs);border-radius:var(--radius-xs);box-shadow:0 0 0 var(--size-hairline) var(--practice-accent),0 0 0 var(--spacing-xxs) color-mix(in srgb,var(--practice-accent) 35%,transparent);animation-name:pv-ring}
@keyframes pv-ring{0%,40%{transform:none}50%,90%{transform:translateX(calc(100% + var(--spacing-xxs) * 2))}100%{transform:none}}

.pv-head{height:var(--spacing-xxs);width:30%;border-radius:var(--radius-xs);margin:var(--spacing-xxs) 0 var(--spacing-xs)}
.pv-head.ask{background:var(--practice-accent)}
.pv-head.work{background:var(--practice-zone-view)}
.pv-head.done{background:var(--practice-zone-work)}
.pv-gap{display:block;height:calc(var(--spacing-xs) * 2)}
.pv-rise{position:absolute;left:var(--spacing-xs);top:calc(var(--spacing-sm) + var(--spacing-xxs) + var(--spacing-xs) * 2);animation-name:pv-rise}
@keyframes pv-rise{0%,28%{transform:translateY(calc(var(--spacing-xs) * 6.5))}44%,100%{transform:none}}
.pv-turn{animation-name:pv-turn}
@keyframes pv-turn{0%,28%{background:var(--practice-zone-view)}44%,100%{background:var(--practice-accent)}}

.pv-kid{right:var(--spacing-xs);left:calc(50% + var(--spacing-xxs));height:calc((100% - var(--spacing-xs) * 4) / 3);opacity:0;animation-name:pv-kid}
.pv-kid.k0{top:var(--spacing-xs)}
.pv-kid.k1{top:calc(var(--spacing-xs) * 2 + (100% - var(--spacing-xs) * 4) / 3);animation-delay:.5s}
.pv-kid.k2{bottom:var(--spacing-xs);animation-delay:1s}
@keyframes pv-kid{0%,18%{opacity:0;transform:translateX(-30%)}30%,84%{opacity:1;transform:none}94%,100%{opacity:0}}

.pv-under{opacity:var(--opacity-dimmed)}
.pv-palette{position:absolute;left:20%;right:20%;top:14%;height:14%;border-radius:var(--radius-sm);background:var(--popover);box-shadow:0 0 0 var(--size-hairline) var(--border),0 var(--spacing-xs) var(--spacing-md) color-mix(in srgb,var(--practice-scrim) 35%,transparent);padding:0 var(--spacing-xs);display:flex;align-items:center}
.pv-query{height:var(--spacing-xxs);width:40%;border-radius:var(--radius-xs);background:var(--foreground);opacity:var(--opacity-secondary);transform-origin:left;animation-name:pv-query}
@keyframes pv-query{0%,10%{transform:scaleX(0)}34%,100%{transform:scaleX(1)}}
.pv-hit{position:absolute;left:20%;right:20%;height:10%;border-radius:var(--radius-xs);background:var(--popover);box-shadow:0 0 0 var(--size-hairline) var(--border);opacity:0;animation-name:pv-hit}
.pv-hit.h0{top:31%;animation-name:pv-hit-on}
.pv-hit.h1{top:43%;animation-delay:.12s}
.pv-hit.h2{top:55%;animation-delay:.24s}
@keyframes pv-hit{0%,36%{opacity:0}44%,90%{opacity:1}100%{opacity:0}}
@keyframes pv-hit-on{0%,36%{opacity:0}44%{opacity:1;background:var(--popover)}54%,90%{opacity:1;background:var(--practice-accent)}100%{opacity:0}}

.pv-lane-in{animation-name:pv-lane}
@keyframes pv-lane{0%,14%{transform:translateX(100%)}28%,86%{transform:none}96%,100%{transform:translateX(100%)}}
.pv-bars{display:flex;align-items:flex-end;gap:var(--spacing-xxs);height:45%;margin-top:var(--spacing-sm)}
.pv-bars i{flex:1;border-radius:var(--radius-xs) var(--radius-xs) 0 0;background:var(--practice-accent-strong);transform-origin:bottom;animation-name:pv-bar}
@keyframes pv-bar{0%,28%{transform:scaleY(0)}40%,86%{transform:scaleY(1)}96%,100%{transform:scaleY(0)}}
`;
