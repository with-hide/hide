// The practice layer over the real shell: a ring around what the current step
// points at, a bubble that says what happens, and the chapter's quest card.
// It reads the world and presses what the learner would press; the shell
// beneath answers the same way it answers the operator.

import { CheckIcon, RotateCcwIcon } from "lucide-react";
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { Kbd } from "../components/ui/kbd";
import { hostKind, keySystem } from "../host";
import { translate } from "../i18n/client";
import type { MessageKey } from "../i18n/catalogs";
import { commandLabel } from "../shortcutLabels";
import { hostChord, hostRegistry, type CommandId } from "../shortcuts";
import { useShellStore } from "../store";
import { cn } from "../lib/utils";
import type { PracticeChapter, PracticeStep } from "./chapters";
import type { PracticeModel } from "./practiceWorld";

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

function rectOf(element: Element | null): Rect | null {
  if (!element) return null;
  const box = element.getBoundingClientRect();
  return box.width && box.height ? { x: box.left, y: box.top, w: box.width, h: box.height } : null;
}

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

export function PracticeCoach({ chapter, model, onRestart }: { chapter: PracticeChapter; model: PracticeModel; onRestart: () => void }) {
  const [index, setIndex] = useState(0);
  const [flash, setFlash] = useState<string | null>(null);
  const [ring, setRing] = useState<Rect | null>(null);
  const [phase, setPhase] = useState<Phase["kind"]>("target");
  const step = chapter.steps[index] ?? null;
  const finished = step === null;

  // A step is done when the world says so, however the learner did it.
  useEffect(() => {
    if (!step || !step.done(model)) return;
    if (step.command) setFlash(commandLabel(step.command));
    setIndex((value) => value + 1);
  }, [model, step]);
  useEffect(() => {
    if (!flash) return undefined;
    const timer = window.setTimeout(() => setFlash(null), 900);
    return () => window.clearTimeout(timer);
  }, [flash]);

  // The ring follows its target every frame: panes resize, menus open, tabs appear.
  const bubble = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    let frame = requestAnimationFrame(function follow() {
      const target = finished ? (document.querySelector('[data-sidebar-group-select]:not([data-sidebar-group-select="ungrouped"])')?.closest("li") ?? document.querySelector("[data-practice-quest]")) : null;
      if (finished) setRing(rectOf(target));
      else if (step) {
        const current = phaseOf(step);
        setPhase(current.kind);
        if (current.kind === "field" && step.fill) fillField(current.field, step.fill.value);
        setRing(rectOf(current.kind === "menu" ? current.item : current.kind === "field" ? current.field : document.querySelector(step.target)));
      }
      frame = requestAnimationFrame(follow);
    });
    return () => cancelAnimationFrame(frame);
  }, [finished, step]);

  const perform = useCallback(() => {
    if (!step) return;
    const current = phaseOf(step);
    if (current.kind === "field") {
      current.field.focus();
      current.field.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", code: "Enter", bubbles: true, cancelable: true }));
      return;
    }
    if (current.kind === "menu") return current.item.click();
    if (step.command) return pressCommand(step.command);
    const target = document.querySelector<HTMLElement>(step.target);
    if (!target) return;
    const box = target.getBoundingClientRect();
    const at = { bubbles: true, cancelable: true, clientX: box.left + box.width / 2, clientY: box.top + box.height / 2, button: step.gesture === "contextmenu" ? 2 : 0 };
    if (step.gesture === "contextmenu") target.dispatchEvent(new MouseEvent("contextmenu", at));
    else target.click();
  }, [step]);

  // ⏎ performs the step, as the bubble says; every other key reaches the shell untouched.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "Enter" || event.metaKey || event.ctrlKey || event.altKey || event.shiftKey || event.isComposing) return;
      if (!event.isTrusted) return;
      event.preventDefault();
      event.stopPropagation();
      if (finished) onRestart();
      else perform();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [finished, onRestart, perform]);

  const say = finished ? chapter.outro : step && phase === "menu" ? `'${translate(step.menuItem as MessageKey)}'을 눌러요` : step && phase === "field" ? `이름은 '${step.fill!.value}'로 넣어 뒀어요. 저장하면 끝` : (step?.say ?? "");
  const how = finished ? "⏎ 처음부터 다시" : phase === "target" && step?.command ? null : phase === "target" && step?.gesture === "contextmenu" ? "⏎ 또는 우클릭" : "⏎ 또는 클릭";
  const place = bubblePlace(ring, finished ? "right" : phase === "menu" ? "right" : phase === "field" ? "bottom" : (step?.side ?? "bottom"), bubble.current, phase === "field" ? document.querySelector("[role=\"dialog\"]") : null);

  return (
    <div className="pointer-events-none fixed inset-0 z-[60]" data-practice-layer data-practice-step={step?.id ?? "done"}>
      {ring ? <div className="absolute rounded-md ring-2 ring-[var(--accent-violet,#a78bfa)] shadow-[0_0_0_4px_color-mix(in_srgb,#a78bfa_25%,transparent)] transition-all duration-200" style={{ left: ring.x - 3, top: ring.y - 3, width: ring.w + 6, height: ring.h + 6 }} /> : null}
      <div ref={bubble} className="absolute max-w-[300px] rounded-lg border border-border bg-popover px-md py-sm text-body text-popover-foreground shadow-lg transition-all duration-200" style={{ left: place.x, top: place.y }}>
        {index === 0 && phase === "target" ? <div className="mb-xs text-caption text-muted-foreground">{chapter.intro}</div> : null}
        <div>{say}</div>
        <div className="mt-xs flex items-center gap-xs text-caption text-muted-foreground">
          {how ? <span>{how}</span> : <><span>⏎ 눌러서 하기 · 단축키</span><Kbd>{commandLabel(step!.command!)}</Kbd></>}
        </div>
      </div>
      {flash ? <div className="absolute left-1/2 top-[38%] -translate-x-1/2 rounded-xl bg-foreground/85 px-lg py-sm text-[28px] font-semibold text-background shadow-xl">{flash}</div> : null}
      <QuestCard chapter={chapter} index={index} onRestart={onRestart} />
    </div>
  );
}

/** Beside the ring on `side`; below `clear` instead when that box (an open dialog) would otherwise be covered. */
function bubblePlace(ring: Rect | null, side: PracticeStep["side"], bubble: HTMLDivElement | null, clear: Element | null): { x: number; y: number } {
  const w = bubble?.offsetWidth ?? 280;
  const h = bubble?.offsetHeight ?? 60;
  const box = rectOf(clear);
  if (box) return { x: Math.max(8, box.x + box.w / 2 - w / 2), y: Math.min(window.innerHeight - h - 8, box.y + box.h + 12) };
  if (!ring) return { x: window.innerWidth / 2 - w / 2, y: window.innerHeight / 3 };
  const gap = 12;
  const x = side === "right" ? ring.x + ring.w + gap : side === "left" ? ring.x - w - gap : ring.x + ring.w / 2 - w / 2;
  const y = side === "top" ? ring.y - h - gap : side === "bottom" ? Math.min(ring.y + ring.h + gap, ring.y + 120) : ring.y + ring.h / 2 - h / 2;
  return { x: Math.max(8, Math.min(window.innerWidth - w - 8, x)), y: Math.max(8, Math.min(window.innerHeight - h - 8, y)) };
}

function QuestCard({ chapter, index, onRestart }: { chapter: PracticeChapter; index: number; onRestart: () => void }) {
  const total = chapter.steps.length;
  return (
    <div data-practice-quest className="pointer-events-auto absolute bottom-md left-md w-[256px] rounded-lg border border-border bg-card p-sm text-caption shadow-lg">
      <div className="flex items-center justify-between gap-xs">
        <span className="font-medium text-foreground">{chapter.number}편 {chapter.title}</span>
        <span className="text-muted-foreground">{Math.min(index, total)}/{total}</span>
      </div>
      <ol className="mt-xs grid gap-xxs">
        {chapter.steps.map((row, at) => (
          <li key={row.id} className={cn("flex items-center gap-xs rounded-sm px-xs py-xxs", at === index ? "bg-accent text-foreground" : at < index ? "text-muted-foreground" : "text-subtle-foreground")}>
            <span className={cn("flex size-[14px] shrink-0 items-center justify-center rounded-xs border", at < index ? "border-transparent bg-[var(--status-done,#4ade80)] text-background" : "border-border")}>{at < index ? <CheckIcon className="size-[10px]" /> : null}</span>
            <span className="min-w-0 flex-1 truncate">{row.title}</span>
            {row.command ? <Kbd>{commandLabel(row.command)}</Kbd> : null}
          </li>
        ))}
      </ol>
      <button type="button" onClick={onRestart} className="mt-xs flex items-center gap-xxs text-muted-foreground hover:text-foreground"><RotateCcwIcon className="size-[12px]" />처음부터</button>
    </div>
  );
}
