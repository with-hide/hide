import { CheckIcon, MinusIcon, SquareIcon, XIcon } from "lucide-react";
import { useMinuteClock } from "./components/elapsed";
import { useInterfaceTranslation } from "./i18n/client";
import { requireInterfaceLanguage, type InterfaceLanguage } from "./i18n/locale";
import { cn } from "./lib/utils";
import { useShellStore } from "./store";
import { lookText, type RecordTone, type RunFacts } from "./automationWords";

// What the automation screen and a run's summary card draw alike: the words
// every line of a run is read with (its language, zone and the minute's
// clock), and a record's mark and line.

export type Words = { t: ReturnType<typeof useInterfaceTranslation>["t"]; language: InterfaceLanguage; zone: string | undefined; now: number };

export function useWords(): Words {
  const { t, i18n } = useInterfaceTranslation();
  const zone = useShellStore((s) => s.automations?.zone);
  const now = useMinuteClock();
  return { t, language: requireInterfaceLanguage(i18n.language), zone, now };
}

const TONE_CLASS: Record<RecordTone, string> = {
  ok: "text-success",
  failed: "text-destructive",
  skipped: "text-muted-foreground",
  running: "text-agent-working",
  stopped: "text-muted-foreground",
};

/** A record's mark: ✓, ✗, –, ● or ■ (B3, B16, B20, B22). */
export function ToneMark({ tone }: { tone: RecordTone }) {
  const className = cn("size-(--size-icon-sm) shrink-0", TONE_CLASS[tone]);
  switch (tone) {
    case "ok":
      return <CheckIcon aria-hidden="true" className={className} />;
    case "failed":
      return <XIcon aria-hidden="true" className={className} />;
    case "skipped":
      return <MinusIcon aria-hidden="true" className={className} />;
    case "running":
      return <span aria-hidden="true" className="flex size-(--size-icon-sm) shrink-0 items-center justify-center"><span className="size-(--size-tab-status-dot) rounded-full bg-agent-working" /></span>;
    case "stopped":
      // ■, filled, so it never reads as an empty checkbox.
      return <SquareIcon aria-hidden="true" className={cn(className, "fill-current")} />;
  }
}

/**
 * `자동화 · 한 번 · ✓ 끝남 · 4분`: what a once automation's run came to, its result in its tone with its mark (S-4, S-5); the one
 * line the sidebar, the Overview, the phone and its gathered list draw. A caption line stays on one line and cuts its result;
 * a body line (the phone) wraps.
 */
export function LookLine({ run, size = "caption", className }: { run: RunFacts | null; size?: "caption" | "body"; className?: string }) {
  const { t, i18n } = useInterfaceTranslation();
  const shown = run ? lookText(run, requireInterfaceLanguage(i18n.language), t) : null;
  return (
    <span className={cn("flex min-w-0 items-center gap-x-xs text-muted-foreground", size === "caption" ? "text-caption" : "flex-wrap text-body", className)} data-automation-look-line={shown?.tone ?? "none"}>
      <span className="shrink-0">{t("automationScreen.look.kind")}</span>
      {shown ? (
        <>
          <span aria-hidden="true" className="shrink-0">{" · "}</span>
          <ToneMark tone={shown.tone} />
          <span className={cn("min-w-0", size === "caption" && "truncate", TONE_CLASS[shown.tone])}>{shown.text}</span>
        </>
      ) : null}
    </span>
  );
}

export function RecordWords({ tone, text }: { tone: RecordTone; text: string }) {
  return (
    <span className="flex min-w-0 items-center gap-xs text-caption text-subtle-foreground" data-automation-record-tone={tone}>
      <ToneMark tone={tone} />
      <span className={cn("min-w-0 truncate", tone === "failed" && "text-destructive")} title={text}>{text}</span>
    </span>
  );
}
