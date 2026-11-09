// The practice layer over the real shell: the screen dimmed but for what a card
// points at, one ring round it, and a card floating beside it in free space
// (`cardPlace.ts`), never over it. ⏎ is the card's next, and the last card leaves
// for Hide. The ring and the card glide from one target to the next; a card that
// changes what the screen shows lets the screen change first and brings the ring
// and the card back once it has settled. A card that asks for a press waits for
// the learner to press its target (⏎ presses it for them). It reads the screen it
// points at every frame; the shell beneath answers the learner exactly as it
// answers the operator.

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

/** How much the ring stands off its target. */
const RING = 3;
/** How long the screen has to settle after a card changes it, before the ring and the card come back. */
const SETTLE_MS = 600;
/** How long after a press the card goes on, so the learner sees the press land. */
const PRESS_BEAT_MS = 350;

/** The scene a card sets, with every card before it: a card that says nothing keeps the scene before it. */
const sceneAt = (chapter: PracticeChapter, index: number) => chapter.steps.slice(0, index + 1).reduce<PracticeScene>((scene, row) => ({ ...scene, ...row.scene }), {});

/** What of a scene changes what is on screen: the list, the lane and the panes shown (the keyboard's pane draws almost nothing). */
const looks = (scene: PracticeScene) => `${scene.lens}|${scene.lane}|${scene.solo}`;

/** Whether the learner asks the system for less motion: nothing glides or fades then, and nothing waits. */
const calm = () => window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;

export function PracticeCoach({ chapter, onStep, onHub, onLeave, onScene }: Props) {
  const { t } = useInterfaceTranslation();
  const say = (key: string, values?: Record<string, unknown>) => t(key as MessageKey, values as never) as string;
  const [index, setIndex] = useState(0);
  const [rings, setRings] = useState<Rect[]>([]);
  const [place, setPlace] = useState<{ x: number; y: number } | null>(null);
  /** False while a card's scene change is on screen and the ring and the card wait for it to settle. */
  const [settled, setSettled] = useState(true);
  const step = chapter.steps[index]!;
  const last = index === chapter.steps.length - 1;
  const go = useRef<HTMLButtonElement>(null);
  const card = useRef<HTMLDivElement>(null);
  const bar = useRef<HTMLDivElement>(null);
  const before = useRef<number | null>(null);
  const pressed = useRef(false);
  const targets = useMemo(() => (step.target === undefined ? [] : Array.isArray(step.target) ? step.target : [step.target]), [step]);

  useEffect(() => onStep(index, last), [index, last, onStep]);

  // The card sets its own scene when it opens, and a card that says nothing keeps the one before it: the way back is the card's, not the learner's.
  useEffect(() => {
    onScene(sceneAt(chapter, index));
  }, [chapter, index, onScene]);

  // A card that changes the list, the lane or the panes lets the screen change first: the ring and the card step aside, then come back where the screen has settled.
  useLayoutEffect(() => {
    const was = before.current;
    before.current = index;
    pressed.current = false;
    if (was === null || was === index || calm() || looks(sceneAt(chapter, was)) === looks(sceneAt(chapter, index))) return setSettled(true);
    setSettled(false);
    const timer = window.setTimeout(() => setSettled(true), SETTLE_MS);
    return () => window.clearTimeout(timer);
  }, [chapter, index]);

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
      if (button?.getAttribute("aria-expanded") !== "true") return;
      button.click();
      // A popover gives the keyboard back to its button as it closes, and the button's hint would open over the next card: the keyboard goes back to the card.
      for (const wait of [80, 300]) window.setTimeout(() => go.current?.focus({ preventScroll: true }), wait);
    };
  }, [step]);

  // A card that asks for a press goes on a beat after its target is pressed, by the learner or by ⏎.
  useEffect(() => {
    if (!step.press) return;
    let timer = 0;
    const onClick = (event: MouseEvent) => {
      const part = document.querySelector(targets[0]!);
      if (!part || !(event.target instanceof Node) || !part.contains(event.target)) return;
      pressed.current = true;
      window.clearTimeout(timer);
      timer = window.setTimeout(() => setIndex((value) => value + 1), calm() ? 0 : PRESS_BEAT_MS);
    };
    document.addEventListener("click", onClick, true);
    return () => {
      document.removeEventListener("click", onClick, true);
      window.clearTimeout(timer);
    };
  }, [step, targets]);

  // The keyboard rests on the card's button, so ⏎ is the card's and nothing waits; a card whose popover is open leaves the keyboard where the popover put it, since taking it away would close the popover (⏎ is the card's wherever the keyboard is).
  useEffect(() => {
    if (!step.opens) go.current?.focus({ preventScroll: true });
  }, [index, step]);

  // The ring and the card follow what they mark every frame: panes resize, rows appear.
  useLayoutEffect(() => {
    let scrolled = false;
    let aim: Rect[] = [];
    let frame = requestAnimationFrame(function follow() {
      // What the card points at may sit below the fold of its own list: bring it into view once, then ring it.
      if (!scrolled && targets.length && targets.every((selector) => document.querySelector(selector))) {
        for (const selector of targets) document.querySelector(selector)?.scrollIntoView({ block: "nearest" });
        scrolled = true;
      }
      const marked = targets.flatMap((selector) => rectOf(selector) ?? []);
      // A target that is not there yet (its list is still opening) leaves the ring and the card where they were.
      if (marked.length || !targets.length) {
        aim = marked;
        setRings((now) => (now.length === marked.length && marked.every((rect, at) => same(rect, now[at] ?? null)) ? now : marked));
      }
      // The card floats where nothing it talks about, nor the mode bar is.
      const own = boxOf(card.current);
      if (own) {
        const kept = (step.keepClear ?? []).flatMap((selector) => [...document.querySelectorAll(selector)]);
        const avoid = [bar.current, ...kept].flatMap((element) => boxOf(element) ?? []);
        const at = placeCard(own, aim, avoid, { w: window.innerWidth, h: window.innerHeight });
        setPlace((now) => (now && now.x === at.x && now.y === at.y ? now : at));
      }
      frame = requestAnimationFrame(follow);
    });
    return () => cancelAnimationFrame(frame);
  }, [step, targets]);

  const forward = useCallback(() => {
    if (last) return onLeave();
    // The press is the learner's, or ⏎'s: it presses the target, and the card goes on a beat after.
    if (step.press) {
      if (pressed.current) return;
      const part = document.querySelector<HTMLElement>(targets[0]!);
      if (part) return part.click();
    }
    setIndex((value) => value + 1);
  }, [last, onLeave, step, targets]);
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
  const away = settled ? "" : " pc-away";

  return (
    <div className="pc-layer" data-practice-layer data-practice-step={step.id} data-practice-index={index} data-practice-steps={chapter.steps.length} onPointerDown={(event) => event.stopPropagation()}>
      <style>{COACH_CSS}</style>
      {step.hides?.length ? <style>{step.hides.map((selector) => `${selector}{display:none !important}`).join("\n")}</style> : null}
      {/* The one ring: the screen is dimmed everywhere but inside it, and it glides from one target to the next. */}
      {rings.slice(0, 1).map((rect) => <div key="ring" className={`pc-spot${away}`} data-practice-ring style={{ left: rect.x - RING, top: rect.y - RING, width: rect.w + 2 * RING, height: rect.h + 2 * RING }} />)}
      <div className="pc-mode" ref={bar}>
        <button type="button" onClick={onHub} data-practice-hub-open>{say("practice.hub")}</button>
        <span>{say("practice.mode", { number: chapter.number, title })}</span>
        <button type="button" className="pc-leave" onClick={onLeave} data-practice-leave>{say("practice.leave")}</button>
      </div>
      {/* The card floats beside what it talks about; a press on it leaves the screen's focus alone. */}
      <div ref={card} className={`pc-card${away}`} data-practice-card data-practice-press={step.press ? "true" : undefined} style={place ? { left: place.x, top: place.y } : { visibility: "hidden" }} onMouseDown={(event) => event.preventDefault()}>
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
          <button ref={go} type="button" className="pc-go" onClick={forward} data-practice-go>{last ? t("practice.leave") : step.press ? say("practice.press") : t("practice.next")} <span className="pc-kc">⏎</span></button>
        </div>
      </div>
    </div>
  );
}
