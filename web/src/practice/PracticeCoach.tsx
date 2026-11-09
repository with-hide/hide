// The practice layer over the real shell: the screen dimmed but for what a card
// points at, a ring round each target, and a card floating beside them in free
// space (`cardPlace.ts`), never over a target. ⏎ is the card's next, and the last
// card leaves for Hide. It reads the screen it points at every frame; the shell
// beneath answers the learner exactly as it answers the operator.

import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { useInterfaceTranslation } from "../i18n/client";
import type { MessageKey } from "../i18n/catalogs";
import { composingKey } from "../keys";
import { hostKind, keySystem } from "../host";
import { commandLabel } from "../shortcutLabels";
import { hostChord, hostRegistry, type CommandId } from "../shortcuts";
import { useShellStore } from "../store";
import type { PracticeChapter, PracticeScene } from "./chapter";
import { placeCard, type Box } from "./cardPlace";
import { placeTags } from "./calloutPlace";
import { cardLines } from "./cardText";
import { COACH_CSS } from "./coachStyle";

type Rect = { x: number; y: number; w: number; h: number };

/** One box around every element `selector` matches. */
function rectOf(selector: string | null | undefined): Rect | null {
  const elements = selector ? [...document.querySelectorAll(selector)] : [];
  const boxes = elements.map((element) => element.getBoundingClientRect()).filter((box) => box.width && box.height);
  if (!boxes.length) return null;
  const x = Math.min(...boxes.map((box) => box.left));
  const y = Math.min(...boxes.map((box) => box.top));
  return { x, y, w: Math.max(...boxes.map((box) => box.right)) - x, h: Math.max(...boxes.map((box) => box.bottom)) - y };
}

const same = (a: Rect | null, b: Rect | null) => a === b || (!!a && !!b && Math.abs(a.x - b.x) < 0.5 && Math.abs(a.y - b.y) < 0.5 && Math.abs(a.w - b.w) < 0.5 && Math.abs(a.h - b.h) < 0.5);

/** Whether this host has a chord for `id` with the operator's bindings (a browser tab has none for some). */
function hasChord(id: CommandId): boolean {
  const host = hostKind();
  const command = hostRegistry(useShellStore.getState().rest?.ui_state, host, keySystem()).registry.find((row) => row.id === id);
  return !!command && !!hostChord(command, host);
}

const MODIFIERS = "⇧⌘⌥⌃";

/** A chord label as keycaps: "⇧⌘B" as ⇧ ⌘ B, "Ctrl+Shift+B" as Ctrl Shift B. */
export function keyParts(label: string): string[] {
  if (label.includes("+")) return label.split("+");
  return [...label].reduce<string[]>((parts, glyph) => {
    const last = parts[parts.length - 1];
    if (!MODIFIERS.includes(glyph) && last !== undefined && !MODIFIERS.includes(last)) parts[parts.length - 1] = last + glyph;
    else parts.push(glyph);
    return parts;
  }, []);
}

const ZONES = [
  { selector: "[data-sidebar-box]", key: "view" },
  { selector: "[data-agent-area]", key: "work" },
  // The lane and the rail beside it that picks its tool, as one part.
  { selector: "[data-lane-slot], [data-tool-rail]", key: "project" },
] as const;

/** A callout as drawn: the box around what it names, and where its tag sits against that box. */
type Call = { text: string; box: Box; x: number; y: number; side: string };

type Props = {
  chapter: PracticeChapter;
  onStep: (step: number, finished: boolean) => void;
  onHub: () => void;
  /** Leaves the practice for the real Hide: the last card's button. */
  onLeave: () => void;
  /** Sets what the window shows for the card that is up. */
  onScene: (scene: PracticeScene) => void;
};

const boxOf = (element: Element | null): Box | null => {
  const box = element?.getBoundingClientRect();
  return box && box.width && box.height ? { x: box.left, y: box.top, w: box.width, h: box.height } : null;
};

/** How much the ring stands off its target, and how round it is. */
const RING = 3;
/** A callout's tag before it is drawn: its size, and how far its box stands off what it names. */
const CALL_HEIGHT = 26;
const CALL_WIDTH = 160;
const CALL_PAD = 2;
const ROUND = 12;

export function PracticeCoach({ chapter, onStep, onHub, onLeave, onScene }: Props) {
  const { t } = useInterfaceTranslation();
  const say = (key: string, values?: Record<string, unknown>) => t(key as MessageKey, values as never) as string;
  const [index, setIndex] = useState(0);
  const [rings, setRings] = useState<Rect[]>([]);
  const [zones, setZones] = useState<(Rect | null)[]>([]);
  const [place, setPlace] = useState<{ x: number; y: number } | null>(null);
  const [calls, setCalls] = useState<Call[]>([]);
  const step = chapter.steps[index]!;
  const last = index === chapter.steps.length - 1;
  const go = useRef<HTMLButtonElement>(null);
  const card = useRef<HTMLDivElement>(null);
  const bar = useRef<HTMLDivElement>(null);
  const targets = useMemo(() => (step.target === undefined ? [] : Array.isArray(step.target) ? step.target : [step.target]), [step]);

  useEffect(() => onStep(index, last), [index, last, onStep]);

  // The card sets its own scene when it opens, and a card that says nothing keeps the one before it: the way back is the card's, not the learner's.
  useEffect(() => {
    onScene(chapter.steps.slice(0, index + 1).reduce<PracticeScene>((scene, row) => ({ ...scene, ...row.scene }), {}));
  }, [chapter, index, onScene]);

  // A card that talks about a popover presses its button once the button is there, and presses it again when the card is left.
  useEffect(() => {
    if (!step.opens) return;
    const selector = step.opens;
    // The panes may still be settling into the card's scene when it opens and a redraw closes the list again, so for the first moments the button is pressed again whenever the list is shut.
    const until = performance.now() + 1500;
    let frame = 0;
    const open = () => {
      const button = document.querySelector<HTMLElement>(selector);
      if (button && button.getAttribute("aria-expanded") !== "true") button.click();
      if (performance.now() < until) frame = requestAnimationFrame(open);
    };
    frame = requestAnimationFrame(open);
    return () => {
      cancelAnimationFrame(frame);
      const button = document.querySelector<HTMLElement>(selector);
      if (button?.getAttribute("aria-expanded") === "true") button.click();
    };
  }, [step]);

  // The keyboard rests on the card's button, so ⏎ is the card's and nothing waits.
  useEffect(() => go.current?.focus({ preventScroll: true }), [index]);

  // The rings, the zones and the card follow what they mark every frame: panes resize, rows appear.
  useLayoutEffect(() => {
    let scrolled = false;
    let frame = requestAnimationFrame(function follow() {
      // What the card points at may sit below the fold of its own list: bring it into view once, then ring it.
      const named = (step.callouts ?? []).map((call) => call.target);
      if (!scrolled && targets.length && [...targets, ...named].every((selector) => document.querySelector(selector))) {
        for (const selector of [...targets, ...named]) document.querySelector(selector)?.scrollIntoView({ block: "nearest" });
        scrolled = true;
      }
      const marked = targets.flatMap((selector) => rectOf(selector) ?? []);
      setRings((before) => (before.length === marked.length && marked.every((rect, at) => same(rect, before[at] ?? null)) ? before : marked));
      setZones((before) => {
        const now = step.zones ? ZONES.map((zone) => rectOf(zone.selector)) : [];
        return now.length === before.length && now.every((rect, at) => same(rect, before[at] ?? null)) ? before : now;
      });
      // Each callout is a box drawn close around what it names, with its tag stuck to the side of that box where it covers the least.
      const found = (step.callouts ?? []).flatMap((call) => {
        const box = rectOf(call.target);
        return box ? [{ text: say(`practice.${chapter.id}.${step.id}.tag.${call.say}`), box: { x: box.x - CALL_PAD, y: box.y - CALL_PAD, w: box.w + 2 * CALL_PAD, h: box.h + 2 * CALL_PAD } }] : [];
      });
      const drawn = [...document.querySelectorAll(".pc-call")].map((tag) => tag.getBoundingClientRect());
      const sizes = found.map((_, at) => ({ w: drawn[at]?.width || CALL_WIDTH, h: drawn[at]?.height || CALL_HEIGHT }));
      const barBox = boxOf(bar.current);
      const spots = placeTags(found.map((call) => call.box), sizes, barBox ? [barBox] : [], { x: 0, y: 0, w: window.innerWidth, h: window.innerHeight });
      const next = found.map((call, at) => ({ ...call, x: spots[at]!.x, y: spots[at]!.y, side: spots[at]!.side }));
      setCalls((before) => (JSON.stringify(before) === JSON.stringify(next) ? before : next));
      // The card floats where nothing it talks about, nor the zone tags, nor the callouts, nor the mode bar is.
      const own = boxOf(card.current);
      if (own) {
        const kept = (step.keepClear ?? []).flatMap((selector) => [...document.querySelectorAll(selector)]);
        // The callouts' places of this very frame, not the last one drawn: a card never lands where a tag is about to stand.
        const tagBoxes = next.flatMap((call, at) => [call.box, { x: call.x, y: call.y, w: sizes[at]!.w, h: sizes[at]!.h }]);
        const avoid = [...document.querySelectorAll(".pc-tag"), bar.current, ...kept].flatMap((element) => boxOf(element) ?? []).concat(tagBoxes);
        const at = placeCard(own, marked, avoid, { w: window.innerWidth, h: window.innerHeight });
        setPlace((before) => (before && before.x === at.x && before.y === at.y ? before : at));
      }
      frame = requestAnimationFrame(follow);
    });
    return () => cancelAnimationFrame(frame);
  }, [step, targets]);

  const forward = useCallback(() => (last ? onLeave() : setIndex((value) => value + 1)), [last, onLeave]);
  const back = index > 0 ? () => setIndex((value) => value - 1) : null;

  // ⏎ is the card's button and ← goes back; every other key reaches the shell untouched.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (!event.isTrusted || event.metaKey || event.ctrlKey || event.altKey || event.shiftKey || composingKey(event)) return;
      if (event.key === "Enter" || (event.key === "ArrowRight" && !last)) {
        event.preventDefault();
        event.stopPropagation();
        forward();
      } else if (event.key === "ArrowLeft" && index > 0) {
        event.preventDefault();
        event.stopPropagation();
        setIndex((value) => value - 1);
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [forward, index, last]);

  const words = (part: string) => say(`practice.${chapter.id}.${step.id}.${part}`);
  const title = say(`practice.${chapter.id}.title`);

  return (
    <div className="pc-layer" data-practice-layer data-practice-step={step.id} data-practice-index={index} data-practice-steps={chapter.steps.length} onPointerDown={(event) => event.stopPropagation()}>
      <style>{COACH_CSS}</style>
      {step.hides?.length ? <style>{step.hides.map((selector) => `${selector}{display:none !important}`).join("\n")}</style> : null}
      {zones.length ? <div className="pc-zdim" /> : null}
      {zones.map((rect, at) => rect ? (
        <div key={ZONES[at]!.key} className={`pc-zone ${ZONES[at]!.key}`} style={{ left: rect.x + 6, top: rect.y + 6, width: rect.w - 12, height: rect.h - 12, animationDelay: `${150 + at * 260}ms` }}>
          <div className="pc-tag">{say(`practice.zone.${ZONES[at]!.key}`)}<small>{say(`practice.zone.${ZONES[at]!.key}Sub`)}</small></div>
        </div>
      ) : null)}
      {rings.length ? (
        <svg className="pc-scrim" aria-hidden="true">
          <defs>
            <mask id="pc-hole">
              <rect width="100%" height="100%" fill="white" />
              {rings.map((rect, at) => <rect key={at} x={rect.x - RING} y={rect.y - RING} width={rect.w + 2 * RING} height={rect.h + 2 * RING} rx={ROUND} fill="black" />)}
            </mask>
          </defs>
          <rect width="100%" height="100%" mask="url(#pc-hole)" className="pc-scrim-fill" />
        </svg>
      ) : null}
      {rings.map((rect, at) => <div key={at} className="pc-spot" data-practice-ring style={{ left: rect.x - RING, top: rect.y - RING, width: rect.w + 2 * RING, height: rect.h + 2 * RING }} />)}
      {calls.map((call) => <div key={`${call.text}:mark`} className="pc-mark" data-practice-mark style={{ left: call.box.x, top: call.box.y, width: call.box.w, height: call.box.h }} />)}
      {calls.map((call) => <div key={call.text} className="pc-call" data-practice-callout data-side={call.side} style={{ left: call.x, top: call.y }}>{call.text}</div>)}
      <div className="pc-mode" ref={bar}>
        <button type="button" onClick={onHub} data-practice-hub-open>{say("practice.hub")}</button>
        <span>{say("practice.mode", { number: chapter.number, title })}</span>
        <button type="button" className="pc-leave" onClick={onLeave} data-practice-leave>{say("practice.leave")}</button>
      </div>
      {/* The card floats beside what it talks about; a press on it leaves the screen's focus alone. */}
      <div ref={card} className="pc-card" data-practice-card style={place ? { left: place.x, top: place.y } : { visibility: "hidden" }} onMouseDown={(event) => event.preventDefault()}>
        <div key={step.id} className="pc-card-text">
          <div className="pc-kick"><span>{words("kick")}</span><span>{say("practice.count", { index: index + 1, total: chapter.steps.length })}</span></div>
          <h3>{words("title")}</h3>
          <div className="pc-body">{cardLines(words("text")).map((line, at) => (
            <p key={at} className={line.kind === "line" ? undefined : `pc-${line.kind}`}>{line.parts.map((part, i) => part.bold ? <b key={i}>{part.text}</b> : part.text)}</p>
          ))}</div>
          {(step.commands ?? []).filter(hasChord).map((command) => <div key={command} className="pc-via">{say(`commands.${command}`)} <span className="pc-keys">{keyParts(commandLabel(command)).map((part, at) => <span key={at} className="pc-kc">{part}</span>)}</span></div>)}
        </div>
        <div className="pc-row">
          <div className="pc-dots">{chapter.steps.map((row, at) => <i key={row.id} className={at === index ? "on" : at < index ? "done" : ""} />)}</div>
          {back ? <button type="button" className="pc-back" onClick={back} data-practice-back>{say("practice.back")}</button> : null}
          <button ref={go} type="button" className="pc-go" onClick={forward} data-practice-go>{last ? t("practice.leave") : t("practice.next")} <span className="pc-kc">⏎</span></button>
        </div>
      </div>
    </div>
  );
}
