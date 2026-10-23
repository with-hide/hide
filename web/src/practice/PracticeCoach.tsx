// The practice layer over the real shell: everything but the step's target
// dimmed, a ring around the target, a light bubble that says what happens, and
// the chapter's quest card. It reads the world and presses what the learner
// would press; the shell beneath answers the same way it answers the operator.

import { useCallback, useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { hostKind, keySystem } from "../host";
import { useInterfaceTranslation } from "../i18n/client";
import type { MessageKey } from "../i18n/catalogs";
import { commandLabel } from "../shortcutLabels";
import { hostChord, hostRegistry, type CommandId } from "../shortcuts";
import { useShellStore } from "../store";
import { useUiStore } from "../ui";
import type { PracticeChapter, PracticeDrag, PracticeStep } from "./chapter";
import { COACH_CSS } from "./coachStyle";

type Rect = { x: number; y: number; w: number; h: number };
type Point = { x: number; y: number };

/** `id`'s chord on this host with the operator's bindings, or null where the host has none (⌥1 in a browser tab). */
function chordOf(id: CommandId) {
  const host = hostKind();
  const command = hostRegistry(useShellStore.getState().rest?.ui_state, host, keySystem()).registry.find((row) => row.id === id);
  return (command && hostChord(command, host)) ?? null;
}

/** Presses `id`'s chord the way a keyboard would, so the shell's own listener runs the command. */
function pressCommand(id: CommandId) {
  const chord = chordOf(id);
  if (!chord) throw new Error(`Practice step names ${id}, which this host has no chord for`);
  const key = chord.code.replace(/^Key|^Digit/, "");
  const target = document.activeElement ?? document.body;
  target.dispatchEvent(new KeyboardEvent("keydown", { key, code: chord.code, metaKey: !!chord.meta, altKey: !!chord.alt, shiftKey: !!chord.shift, ctrlKey: !!chord.ctrl, bubbles: true, cancelable: true }));
}

/** Presses the middle of what `selector` marks as a hand would, on whatever is drawn there: a terminal hears presses, not clicks. */
function pressTarget(selector: string | undefined) {
  const marked = selector ? document.querySelector<HTMLElement>(selector) : null;
  if (!marked) throw new Error(`Practice step target ${selector ?? "(none)"} is not on screen`);
  const box = marked.getBoundingClientRect();
  const x = box.left + box.width / 2;
  const y = box.top + box.height / 2;
  // The bubble may sit over the target; the press lands on the target's own topmost element there.
  const element = document.elementsFromPoint(x, y).find((hit) => marked.contains(hit)) ?? marked;
  const at = { bubbles: true, cancelable: true, composed: true, isPrimary: true, button: 0, detail: 1, clientX: x, clientY: y, pointerType: "mouse" };
  element.dispatchEvent(new PointerEvent("pointerdown", { ...at, buttons: 1 }));
  element.dispatchEvent(new MouseEvent("mousedown", { ...at, buttons: 1 }));
  element.dispatchEvent(new PointerEvent("pointerup", at));
  element.dispatchEvent(new MouseEvent("mouseup", at));
  element.dispatchEvent(new MouseEvent("click", at));
}

/** Drags a pane header onto another pane's edge as a hand would, so the shell's own drag runs the move; `follow` sees the pointer. */
function dragPane(drag: PracticeDrag, follow: (point: Point | null) => void) {
  const header = document.querySelector<HTMLElement>(drag.from);
  const target = document.querySelector<HTMLElement>(drag.to);
  if (!header || !target) return;
  const from = header.getBoundingClientRect();
  const to = target.getBoundingClientRect();
  const start = { x: from.left + from.width * 0.35, y: from.top + from.height / 2 };
  const end = {
    x: drag.edge === "left" ? to.left + 18 : drag.edge === "right" ? to.right - 18 : to.left + to.width / 2,
    y: drag.edge === "up" ? to.top + 18 : drag.edge === "down" ? to.bottom - 18 : to.top + to.height / 2,
  };
  const pointer = (type: string, at: Point) => ({ bubbles: true, cancelable: true, pointerId: 41, isPrimary: true, button: 0, buttons: type === "pointerup" ? 0 : 1, clientX: at.x, clientY: at.y, pointerType: "mouse" });
  follow(start);
  header.dispatchEvent(new PointerEvent("pointerdown", pointer("pointerdown", start)));
  const began = performance.now();
  const glide = (now: number) => {
    const t = Math.min(1, (now - began) / 900);
    const eased = t < 0.5 ? 2 * t * t : 1 - (-2 * t + 2) ** 2 / 2;
    const at = { x: start.x + (end.x - start.x) * eased, y: start.y + (end.y - start.y) * eased };
    follow(at);
    window.dispatchEvent(new PointerEvent("pointermove", pointer("pointermove", at)));
    if (t < 1) return void requestAnimationFrame(glide);
    window.setTimeout(() => {
      window.dispatchEvent(new PointerEvent("pointerup", pointer("pointerup", end)));
      window.setTimeout(() => follow(null), 250);
    }, 350);
  };
  window.setTimeout(() => requestAnimationFrame(glide), 250);
}

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

const MODIFIERS = "⇧⌘⌥⌃";

/** A chord label as keycaps: "⇧⌘B" as ⇧ ⌘ B, "Ctrl+Shift+B" as Ctrl Shift B. */
function keyParts(label: string): string[] {
  if (label.includes("+")) return label.split("+");
  return [...label].reduce<string[]>((parts, glyph) => {
    const last = parts[parts.length - 1];
    if (!MODIFIERS.includes(glyph) && last !== undefined && !MODIFIERS.includes(last)) parts[parts.length - 1] = last + glyph;
    else parts.push(glyph);
    return parts;
  }, []);
}

function Keys({ id }: { id: CommandId }) {
  return <span className="pc-keys">{keyParts(commandLabel(id)).map((part, index) => <span key={index} className="pc-kc">{part}</span>)}</span>;
}

/** A sentence with the command's keycaps where the catalog puts `{{keys}}`. */
function WithKeys({ text, id }: { text: string; id: CommandId }) {
  const [before, after] = text.split(KEYS_MARK);
  return <>{before}<Keys id={id} />{after}</>;
}
const KEYS_MARK = "\u0001";

const ZONES = [
  { selector: "[data-sidebar-box]", key: "view" },
  { selector: "[data-agent-area]", key: "work" },
  { selector: "[data-lane-slot]", key: "project" },
] as const;

type Props = {
  chapter: PracticeChapter;
  model: unknown;
  next: PracticeChapter | null;
  /** Steps before this one are played again by the coach itself, the way ⏎ plays them. */
  resume: number;
  onStep: (step: number, finished: boolean) => void;
  onNext: () => void;
  onRestart: () => void;
  onHub: () => void;
};

/** How long the coach waits before playing the next step it replays, so the learner sees where they are going. */
const REPLAY_MS = 120;

export function PracticeCoach({ chapter, model, next, resume, onStep, onNext, onRestart, onHub }: Props) {
  const { t } = useInterfaceTranslation();
  const say = (key: string, values?: Record<string, unknown>) => t(key as MessageKey, values as never) as string;
  const [index, setIndex] = useState(0);
  const [flash, setFlash] = useState<string | null>(null);
  const [ring, setRing] = useState<Rect | null>(null);
  const [zones, setZones] = useState<(Rect | null)[]>([]);
  const [hand, setHand] = useState<Point | null>(null);
  const step = chapter.steps[index]!;
  const last = index === chapter.steps.length - 1;
  // A command the host has no chord for is taught by pressing what the step marks.
  const pressed = !!step.command && !chordOf(step.command);

  // The steps the coach replays were reached before; only a step past them is news.
  useEffect(() => {
    if (index >= resume) onStep(index, last);
  }, [index, last, onStep, resume]);

  // A step the world can finish is done when the world says so, however the learner did it.
  const seen = useRef(model);
  useEffect(() => {
    if (seen.current === model) return;
    seen.current = model;
    if (step.command) setFlash(commandLabel(step.command));
    if (step.done?.(model)) setIndex((value) => value + 1);
  }, [model, step]);
  useEffect(() => {
    if (!flash) return undefined;
    const timer = window.setTimeout(() => setFlash(null), 900);
    return () => window.clearTimeout(timer);
  }, [flash]);

  // The ring and the zones follow what they mark every frame: panes resize, buttons appear, tabs open.
  const bubble = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState({ w: 340, h: 180 });
  useLayoutEffect(() => {
    let frame = requestAnimationFrame(function follow() {
      const marked = rectOf(step.target);
      setRing((before) => (same(before, marked) ? before : marked));
      setZones((before) => {
        const now = step.zones ? ZONES.map((zone) => rectOf(zone.selector)) : [];
        return now.length === before.length && now.every((rect, at) => same(rect, before[at] ?? null)) ? before : now;
      });
      const box = bubble.current;
      if (box) setSize((before) => (before.w === box.offsetWidth && before.h === box.offsetHeight ? before : { w: box.offsetWidth, h: box.offsetHeight }));
      frame = requestAnimationFrame(follow);
    });
    return () => cancelAnimationFrame(frame);
  }, [step]);

  const advance = useCallback(() => {
    if (!last) return setIndex((value) => value + 1);
    if (next) onNext();
    else onHub();
  }, [last, next, onNext, onHub]);

  const perform = useCallback(() => {
    if (!step.done) return advance();
    if (step.gesture && "drag" in step.gesture) return dragPane(step.gesture.drag, setHand);
    if (step.gesture && "click" in step.gesture) return document.querySelector<HTMLElement>(step.target ?? "")?.click();
    if (pressed) return pressTarget(step.target);
    if (step.command) pressCommand(step.command);
  }, [advance, pressed, step]);

  useEffect(() => {
    if (index >= resume) return undefined;
    const timer = window.setTimeout(perform, REPLAY_MS);
    return () => window.clearTimeout(timer);
  }, [index, perform, resume]);

  const back = index > 0 && !step.done && !chapter.steps[index - 1]!.done;

  // ⏎ and → perform the step, ← goes back over a look; every other key reaches the shell untouched,
  // and while a palette the shell opened is up, every key is the palette's.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (!event.isTrusted || event.metaKey || event.ctrlKey || event.altKey || event.shiftKey || event.isComposing) return;
      if (useUiStore.getState().overlay !== "none") return;
      if (event.key === "Enter" || (event.key === "ArrowRight" && !step.done)) {
        event.preventDefault();
        event.stopPropagation();
        perform();
      } else if (event.key === "ArrowLeft" && back) {
        event.preventDefault();
        event.stopPropagation();
        setIndex((value) => value - 1);
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [back, perform, step]);

  const words = (part: string) => say(`practice.${chapter.id}.${step.id}.${part}`);
  const via: ReactNode = pressed ? say("practice.via.press")
    : step.command && step.gesture && "click" in step.gesture ? <WithKeys id={step.command} text={say("practice.via.buttonOrKey", { keys: KEYS_MARK })} />
    : step.command ? <WithKeys id={step.command} text={say("practice.via.key", { keys: KEYS_MARK })} />
    : step.gesture && "drag" in step.gesture ? say("practice.via.drag") : null;
  const go = last ? (next ? say("practice.toChapter", { number: next.number }) : say("practice.toHub")) : step.done ? say("practice.try") : say("practice.next");
  const place = bubblePlace(ring, step.side, size);
  const title = say(`practice.${chapter.id}.title`);

  return (
    <div className="pc-layer" data-practice-layer data-practice-step={step.id}>
      <style>{COACH_CSS}</style>
      {zones.length ? <div className="pc-zdim" /> : null}
      {zones.map((rect, at) => rect ? (
        <div key={ZONES[at]!.key} className={`pc-zone ${ZONES[at]!.key}`} style={{ left: rect.x + 6, top: rect.y + 6, width: rect.w - 12, height: rect.h - 12, animationDelay: `${150 + at * 260}ms` }}>
          <div className="pc-tag">{say(`practice.zone.${ZONES[at]!.key}`)}<small>{say(`practice.zone.${ZONES[at]!.key}Sub`)}</small></div>
        </div>
      ) : null)}
      <div className={`pc-spot${ring && !step.zones ? "" : " off"}`} style={ring ? { left: ring.x - 3, top: ring.y - 3, width: ring.w + 6, height: ring.h + 6 } : undefined} />
      <div className="pc-mode">
        <button type="button" onClick={onHub} data-practice-hub-open>{say("practice.hub")}</button>
        <span>{say("practice.mode", { number: chapter.number, title })}</span>
      </div>
      <div ref={bubble} key={step.id} className="pc-bubble" style={{ left: place.x, top: place.y }}>
        {place.arrow ? <div className="pc-arrow" style={place.arrow} /> : null}
        <div className="pc-kick"><span>{words("kick")}</span><span>{say("practice.count", { index: index + 1, total: chapter.steps.length })}</span></div>
        <h3>{words("title")}</h3>
        <p>{words("text")}</p>
        {via ? <div className="pc-via">{via}</div> : null}
        <div className="pc-row">
          <div className="pc-dots">{chapter.steps.map((row, at) => <i key={row.id} className={at === index ? "on" : at < index ? "done" : ""} />)}</div>
          {back ? <button type="button" className="pc-back" onClick={() => setIndex((value) => value - 1)}>{say("practice.back")}</button> : null}
          <button type="button" className="pc-go" onClick={perform}>{go} <span className="pc-kc">⏎</span></button>
        </div>
      </div>
      {flash ? <div className="pc-flash">{keyParts(flash).map((part, at) => <span key={at} className="pc-kc">{part}</span>)}</div> : null}
      {hand ? <svg className="pc-hand" style={{ left: hand.x - 3, top: hand.y - 2 }} width="24" height="24" viewBox="0 0 18 18" aria-hidden="true"><path d="M1 1l6 15 2.2-6.3L15.5 7.5z" /></svg> : null}
      {chapter.quest ? <QuestCard chapter={chapter} index={index} title={title} say={say} onRestart={onRestart} /> : null}
    </div>
  );
}

type Placement = { x: number; y: number; arrow: CSSProperties | null };

function bubblePlace(ring: Rect | null, side: PracticeStep["side"], size: { w: number; h: number }): Placement {
  const { w, h } = size;
  const clampX = (x: number) => Math.max(16, Math.min(window.innerWidth - w - 16, x));
  const clampY = (y: number) => Math.max(40, Math.min(window.innerHeight - h - 16, y));
  if (!ring || side === "center") return { x: clampX(window.innerWidth / 2 - w / 2 - 120), y: clampY(window.innerHeight * 0.68), arrow: null };
  const anchor = ring.y + Math.min(ring.h / 2, 140) + (ring.h > 400 ? -60 : 0);
  if (side === "right" || side === "left") {
    const x = clampX(side === "right" ? ring.x + ring.w + 22 : ring.x - w - 22);
    const y = clampY(anchor - h / 2);
    const arrowY = Math.max(20, Math.min(h - 30, anchor - y - 8));
    return { x, y, arrow: side === "right" ? { left: -8, top: arrowY } : { right: -8, top: arrowY, boxShadow: "var(--size-hairline) var(--size-hairline) 0 var(--practice-line)" } };
  }
  if (side === "bottom") {
    const x = clampX(ring.x + ring.w / 2 - w / 2);
    return { x, y: clampY(ring.y + ring.h + 22), arrow: { left: Math.max(16, Math.min(w - 32, ring.x + ring.w / 2 - x - 8)), top: -8 } };
  }
  return { x: clampX(ring.x + 60), y: clampY(ring.y + Math.min(ring.h * 0.3, 260)), arrow: null };
}

type Say = (key: string, values?: Record<string, unknown>) => string;

function QuestCard({ chapter, index, title, say, onRestart }: { chapter: PracticeChapter; index: number; title: string; say: Say; onRestart: () => void }) {
  const { t } = useInterfaceTranslation();
  const listed = chapter.steps.flatMap((step, at) => (step.quest ? [{ step, at }] : []));
  const done = listed.filter((row) => row.at < index).length;
  return (
    <div className="pc-quest" data-practice-quest>
      <div className="pc-qh">
        <small>{t("practice.questHead", { number: chapter.number })}</small>
        <b>{title}</b>
        <div className="pc-bar"><i style={{ width: `${(done / listed.length) * 100}%` }} /></div>
      </div>
      <ol>
        {listed.map(({ step }, row) => (
          <li key={step.id} className={row < done ? "done" : row === done ? "cur" : ""}>
            <span className="pc-ck" />
            {say(`practice.${chapter.id}.${step.id}.quest`)}
            {step.command && chordOf(step.command) ? <span className="pc-k"><Keys id={step.command} /></span> : null}
          </li>
        ))}
      </ol>
      <div className="pc-qf"><span>{done}/{listed.length}</span><button type="button" onClick={onRestart}>{t("practice.questRestart")}</button></div>
    </div>
  );
}
