// The practice layer over the real shell: everything but the step's target
// dimmed, a ring around the target, a light bubble that says what happens, and
// the chapter's quest card. It reads the world and presses what the learner
// would press; the shell beneath answers the same way it answers the operator.

import { useCallback, useEffect, useLayoutEffect, useRef, useState, type CSSProperties } from "react";
import { hostKind, keySystem } from "../host";
import { translate } from "../i18n/client";
import type { MessageKey } from "../i18n/catalogs";
import { commandLabel } from "../shortcutLabels";
import { hostChord, hostRegistry, type CommandId } from "../shortcuts";
import { useShellStore } from "../store";
import type { PracticeChapter, PracticeStep } from "./chapters";
import { COACH_CSS } from "./coachStyle";

type Rect = { x: number; y: number; w: number; h: number };

/** Presses `id`'s chord the way a keyboard would, so the shell's own listener runs the command. */
function pressCommand(id: CommandId) {
  const host = hostKind();
  const command = hostRegistry(useShellStore.getState().rest?.ui_state, host, keySystem()).registry.find((row) => row.id === id);
  const chord = command && hostChord(command, host);
  if (!chord) throw new Error(`Practice step names ${id}, which this host has no chord for`);
  const key = chord.code.replace(/^Key|^Digit/, "");
  const target = document.activeElement ?? document.body;
  target.dispatchEvent(new KeyboardEvent("keydown", { key, code: chord.code, metaKey: !!chord.meta, altKey: !!chord.alt, shiftKey: !!chord.shift, ctrlKey: !!chord.ctrl, bubbles: true, cancelable: true }));
}

/** One box around every element `selector` matches. */
function rectOf(selector: string | Element | null | undefined): Rect | null {
  const elements = !selector ? [] : typeof selector === "string" ? [...document.querySelectorAll(selector)] : [selector];
  const boxes = elements.map((element) => element.getBoundingClientRect()).filter((box) => box.width && box.height);
  if (!boxes.length) return null;
  const x = Math.min(...boxes.map((box) => box.left));
  const y = Math.min(...boxes.map((box) => box.top));
  return { x, y, w: Math.max(...boxes.map((box) => box.right)) - x, h: Math.max(...boxes.map((box) => box.bottom)) - y };
}

const same = (a: Rect | null, b: Rect | null) => a === b || (!!a && !!b && Math.abs(a.x - b.x) < 0.5 && Math.abs(a.y - b.y) < 0.5 && Math.abs(a.w - b.w) < 0.5 && Math.abs(a.h - b.h) < 0.5);

/** The open menu's item a step goes on to press, by its words. */
function menuItem(step: PracticeStep): HTMLElement | null {
  if (!step.menuItem) return null;
  const words = translate(step.menuItem as MessageKey);
  return [...document.querySelectorAll<HTMLElement>('[role="menuitem"]')].find((item) => item.textContent?.trim().startsWith(words)) ?? null;
}

/** Where the learner is inside a step that opens a menu and a dialog of the real shell. */
type Phase = { kind: "target" } | { kind: "menu"; item: HTMLElement } | { kind: "field"; field: HTMLInputElement };

function phaseOf(step: PracticeStep): Phase {
  const field = step.fill ? document.querySelector<HTMLInputElement>(`[role="dialog"] ${step.fill.selector}`) : null;
  if (field) return { kind: "field", field };
  const item = menuItem(step);
  if (item) return { kind: "menu", item };
  return { kind: "target" };
}

/** Sets a React-controlled field as typing would. */
function fillField(field: HTMLInputElement, value: string) {
  if (field.value === value) return;
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(field, value);
  field.dispatchEvent(new Event("input", { bubbles: true }));
}

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

const ZONES = [
  { selector: "[data-sidebar-box]", tone: "z1", title: "① 보는 곳", sub: "무엇을 볼지 고르기" },
  { selector: "[data-agent-area]", tone: "z2", title: "② 일하는 곳", sub: "에이전트 터미널" },
  { selector: "[data-lane-slot]", tone: "z3", title: "③ 프로젝트와 그 작업", sub: "현황 · 할 일 · 파일" },
] as const;

type Props = { chapter: PracticeChapter; model: unknown; next: PracticeChapter | null; onNext: () => void; onRestart: () => void };

export function PracticeCoach({ chapter, model, next, onNext, onRestart }: Props) {
  const [index, setIndex] = useState(0);
  const [flash, setFlash] = useState<string | null>(null);
  const [ring, setRing] = useState<Rect | null>(null);
  const [zones, setZones] = useState<(Rect | null)[]>([]);
  const [measured, setPhase] = useState<Phase["kind"]>("target");
  const step = chapter.steps[index]!;
  // The phase is measured a frame late; one the current step cannot be in is the last step's.
  const phase = measured === "field" && step.fill ? "field" : measured === "menu" && step.menuItem ? "menu" : "target";
  const last = index === chapter.steps.length - 1;

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

  // The ring and the zones follow what they mark every frame: panes resize, menus open, tabs appear.
  const bubble = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState({ w: 340, h: 180 });
  useLayoutEffect(() => {
    let frame = requestAnimationFrame(function follow() {
      const current = phaseOf(step);
      setPhase(current.kind);
      if (current.kind === "field" && step.fill) fillField(current.field, step.fill.value);
      const marked = rectOf(current.kind === "menu" ? current.item : current.kind === "field" ? current.field.closest('[role="dialog"]') : step.target);
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
    else onRestart();
  }, [last, next, onNext, onRestart]);

  const perform = useCallback(() => {
    const current = phaseOf(step);
    if (current.kind === "field") {
      current.field.focus();
      current.field.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", code: "Enter", bubbles: true, cancelable: true }));
      return;
    }
    if (current.kind === "menu") return current.item.click();
    if (!step.done) return advance();
    if (step.command) return pressCommand(step.command);
    const target = step.target ? document.querySelector<HTMLElement>(step.target) : null;
    if (!target) return;
    const box = target.getBoundingClientRect();
    const at = { bubbles: true, cancelable: true, clientX: box.left + box.width / 2, clientY: box.top + box.height / 2, button: step.gesture === "contextmenu" ? 2 : 0 };
    if (step.gesture === "contextmenu") target.dispatchEvent(new MouseEvent("contextmenu", at));
    else target.click();
  }, [advance, step]);

  const back = index > 0 && !step.done && !chapter.steps[index - 1]!.done;

  // ⏎ and → perform the step, ← goes back over a look; every other key reaches the shell untouched.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (!event.isTrusted || event.metaKey || event.ctrlKey || event.altKey || event.shiftKey || event.isComposing) return;
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

  const words = phase === "menu" ? { title: `'${translate(step.menuItem as MessageKey)}'`, text: "우클릭 메뉴엔 이름 바꾸기, 다른 그룹으로 옮기기도 있어요." }
    : phase === "field" ? { title: `이름은 '${step.fill!.value}'로 넣어 뒀어요`, text: "저장하면 끝." }
    : { title: step.title, text: step.text };
  const via = phase === "target" && step.command ? <>단축키 <Keys id={step.command} /> 를 눌러도 돼요</> : phase === "target" && step.gesture === "contextmenu" ? "화면에서 직접 우클릭해도 돼요" : phase === "menu" ? "화면에서 직접 클릭해도 돼요" : phase === "field" ? "화면에서 직접 저장해도 돼요" : null;
  const go = last ? (next ? `${next.number}편으로` : "처음부터 다시") : step.done ? "해 보기" : "다음";
  const side = phase === "menu" ? "right" : phase === "field" ? "bottom" : step.side;
  const place = bubblePlace(ring, side, size);

  return (
    <div className="pc-layer" data-practice-layer data-practice-step={step.id} data-practice-phase={phase}>
      <style>{COACH_CSS}</style>
      {zones.length ? <div className="pc-zdim" /> : null}
      {zones.map((rect, at) => rect ? (
        <div key={ZONES[at]!.tone} className={`pc-zone ${ZONES[at]!.tone}`} style={{ left: rect.x + 6, top: rect.y + 6, width: rect.w - 12, height: rect.h - 12, animationDelay: `${150 + at * 260}ms` }}>
          <div className="pc-tag">{ZONES[at]!.title}<small>{ZONES[at]!.sub}</small></div>
        </div>
      ) : null)}
      <div className={`pc-spot${ring && !step.zones ? "" : " off"}`} style={ring ? { left: ring.x - 3, top: ring.y - 3, width: ring.w + 6, height: ring.h + 6 } : undefined} />
      <div className="pc-mode">연습 모드 · {chapter.number}편 {chapter.title}</div>
      <div ref={bubble} key={`${step.id}:${phase}`} className="pc-bubble" style={{ left: place.x, top: place.y }}>
        {place.arrow ? <div className="pc-arrow" style={place.arrow} /> : null}
        <div className="pc-kick"><span>{step.kick}</span><span>{index + 1} / {chapter.steps.length}</span></div>
        <h3>{words.title}</h3>
        <p>{words.text}</p>
        {via ? <div className="pc-via">{via}</div> : null}
        <div className="pc-row">
          <div className="pc-dots">{chapter.steps.map((row, at) => <i key={row.id} className={at === index ? "on" : at < index ? "done" : ""} />)}</div>
          {back ? <button type="button" className="pc-back" onClick={() => setIndex((value) => value - 1)}>이전</button> : null}
          <button type="button" className="pc-go" onClick={perform}>{go} <span className="pc-kc">⏎</span></button>
        </div>
      </div>
      {flash ? <div className="pc-flash">{keyParts(flash).map((part, at) => <span key={at} className="pc-kc">{part}</span>)}</div> : null}
      {chapter.quest ? <QuestCard chapter={chapter} index={index} onRestart={onRestart} /> : null}
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
    return { x, y, arrow: side === "right" ? { left: -8, top: arrowY } : { right: -8, top: arrowY, boxShadow: "1px 1px 0 var(--pc-line)" } };
  }
  if (side === "bottom") {
    const x = clampX(ring.x + ring.w / 2 - w / 2);
    return { x, y: clampY(ring.y + ring.h + 22), arrow: { left: Math.max(16, Math.min(w - 32, ring.x + ring.w / 2 - x - 8)), top: -8 } };
  }
  return { x: clampX(ring.x + 60), y: clampY(ring.y + Math.min(ring.h * 0.3, 260)), arrow: null };
}

function QuestCard({ chapter, index, onRestart }: { chapter: PracticeChapter; index: number; onRestart: () => void }) {
  const listed = chapter.steps.flatMap((step, at) => (step.quest ? [{ step, at }] : []));
  const done = listed.filter((row) => row.at < index).length;
  return (
    <div className="pc-quest" data-practice-quest>
      <div className="pc-qh">
        <small>연습 모드 · {chapter.number}편</small>
        <b>{chapter.title}</b>
        <div className="pc-bar"><i style={{ width: `${(done / listed.length) * 100}%` }} /></div>
      </div>
      <ol>
        {listed.map(({ step }, row) => (
          <li key={step.id} className={row < done ? "done" : row === done ? "cur" : ""}>
            <span className="pc-ck" />
            {step.quest}
            {step.command ? <span className="pc-k"><Keys id={step.command} /></span> : null}
          </li>
        ))}
      </ol>
      <div className="pc-qf"><span>{done}/{listed.length}</span><button type="button" onClick={onRestart}>↺ 처음부터</button></div>
    </div>
  );
}
