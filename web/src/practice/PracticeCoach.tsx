// The practice layer over the real shell: a card in a strip under the screen
// (so it never covers what it explains), a ring or the three zones on the
// screen above it, and ⏎ for the next card. The last card hands over to a real
// mission. It reads the screen it points at every frame; the shell beneath
// answers the learner exactly as it answers the operator.

import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { useInterfaceTranslation } from "../i18n/client";
import type { MessageKey } from "../i18n/catalogs";
import { composingKey } from "../keys";
import { hostKind, keySystem } from "../host";
import { commandLabel } from "../shortcutLabels";
import { hostChord, hostRegistry, type CommandId } from "../shortcuts";
import { useShellStore } from "../store";
import type { PracticeChapter } from "./chapter";
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

type Props = {
  chapter: PracticeChapter;
  onStep: (step: number, finished: boolean) => void;
  /** The last card's button: leaves the practice for a real mission. */
  onMission: () => void;
  onHub: () => void;
  /** Leaves the practice for the real Hide. */
  onLeave: () => void;
};

export function PracticeCoach({ chapter, onStep, onMission, onHub, onLeave }: Props) {
  const { t } = useInterfaceTranslation();
  const say = (key: string, values?: Record<string, unknown>) => t(key as MessageKey, values as never) as string;
  const [index, setIndex] = useState(0);
  const [ring, setRing] = useState<Rect | null>(null);
  const [zones, setZones] = useState<(Rect | null)[]>([]);
  const step = chapter.steps[index]!;
  const last = index === chapter.steps.length - 1;
  const go = useRef<HTMLButtonElement>(null);

  useEffect(() => onStep(index, last), [index, last, onStep]);

  // The keyboard rests on the card's button, so ⏎ is the card's and nothing waits.
  useEffect(() => go.current?.focus({ preventScroll: true }), [index]);

  // The ring and the zones follow what they mark every frame: panes resize, rows appear.
  useLayoutEffect(() => {
    let frame = requestAnimationFrame(function follow() {
      const marked = rectOf(step.target);
      setRing((before) => (same(before, marked) ? before : marked));
      setZones((before) => {
        const now = step.zones ? ZONES.map((zone) => rectOf(zone.selector)) : [];
        return now.length === before.length && now.every((rect, at) => same(rect, before[at] ?? null)) ? before : now;
      });
      frame = requestAnimationFrame(follow);
    });
    return () => cancelAnimationFrame(frame);
  }, [step]);

  const forward = useCallback(() => (last ? onMission() : setIndex((value) => value + 1)), [last, onMission]);
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
    <>
      <div className="pc-layer" data-practice-layer data-practice-step={step.id} data-practice-index={index} data-practice-steps={chapter.steps.length} onPointerDown={(event) => event.stopPropagation()}>
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
          <button type="button" className="pc-leave" onClick={onLeave} data-practice-leave>{say("practice.leave")}</button>
        </div>
      </div>
      {/* The card is not part of the screen it teaches: it sits in its own strip, and a press on it leaves the screen's focus alone. */}
      <div key={step.id} className="pc-dock" data-practice-dock onMouseDown={(event) => event.preventDefault()}>
        <div className="pc-dock-text">
          <div className="pc-kick"><span>{words("kick")}</span><span>{say("practice.count", { index: index + 1, total: chapter.steps.length })}</span></div>
          <h3>{words("title")}</h3>
          <p>{words("text")}</p>
          {step.command && hasChord(step.command) ? <div className="pc-via">{say(`commands.${step.command}`)} <span className="pc-keys">{keyParts(commandLabel(step.command)).map((part, at) => <span key={at} className="pc-kc">{part}</span>)}</span></div> : null}
        </div>
        <div className="pc-row">
          <div className="pc-dots">{chapter.steps.map((row, at) => <i key={row.id} className={at === index ? "on" : at < index ? "done" : ""} />)}</div>
          {back ? <button type="button" className="pc-back" onClick={back} data-practice-back>{say("practice.back")}</button> : null}
          <button ref={go} type="button" className="pc-go" onClick={forward} data-practice-go>{step.mission ? t("practice.tryMission") : t("practice.next")} <span className="pc-kc">⏎</span></button>
        </div>
      </div>
    </>
  );
}
