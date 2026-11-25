// The phone's start sheet (PRD home-device-rail D-24, B42-B46), in the desktop
// Start panel's order: where (the device, the project `›` the branch), what to
// do, who (the agent and its model), and Start. The kind and model start from
// the desktop's remembered choice. The text stays until a start succeeds, so a
// refusal or a lost connection never costs it.

import { useRef } from "react";
import { Loader2Icon } from "lucide-react";
import { Button } from "../components/ui/button";
import { useInterfaceTranslation } from "../i18n/translator";
import { Sheet, SheetContent, SheetDescription, SheetTitle } from "../components/ui/sheet";
import { closeStartSheet, editStartSheet, submitStart } from "./connection";
import { noticeText } from "./protocol";
import { OfflineText } from "./OfflineText";
import { selectionOf, startFailure, startProblem } from "./start";
import { WhereWords, WhoWords } from "./StartWords";
import { usePhone } from "./store";

export function StartSheet() {
  const { t } = useInterfaceTranslation();
  const sheet = usePhone((s) => s.startSheet);
  const catalog = usePhone((s) => s.startCatalog);
  const connected = usePhone((s) => s.connected);
  const selection = selectionOf(catalog, sheet.choice);
  const problem = startProblem(sheet.text);
  const tooLong = problem === "too_long";
  const shown = tooLong ? startFailure("too_long") : sheet.error;
  const pending = sheet.pending !== null;
  const text = useRef<HTMLTextAreaElement>(null);
  const ready = connected && !pending && problem === null && selection.target !== null;
  return (
    <Sheet open={sheet.open} onOpenChange={(open) => !open && closeStartSheet()}>
      <SheetContent side="bottom" className="phone-safe-bottom max-h-full gap-md overflow-y-auto rounded-t-xl px-lg pt-lg pb-lg" data-phone-start-sheet="true" onOpenAutoFocus={(event) => {
        // The text box takes the keyboard, as it did when it was the sheet's first control.
        event.preventDefault();
        text.current?.focus();
      }}>
        <SheetTitle className="text-headline">{t("mobile.start.title")}</SheetTitle>
        <SheetDescription className="sr-only">{t("mobile.start.description")}</SheetDescription>
        {!connected ? (
          <p role="status" className="flex items-center gap-md rounded-lg bg-secondary px-md py-sm text-subhead text-subtle-foreground" data-phone-start-unreachable="true">
            <Loader2Icon aria-hidden="true" className="size-(--size-icon-lg) shrink-0 animate-spin" />
            <span><OfflineText /></span>
          </p>
        ) : null}
        <WhereWords catalog={catalog} selection={selection} choice={sheet.choice} disabled={pending} onChange={(choice) => editStartSheet({ choice })} />
        <textarea
          ref={text}
          value={sheet.text}
          onChange={(event) => editStartSheet({ text: event.target.value })}
          placeholder={t("shell.instructionPrompt")}
          aria-label={t("mobile.start.task")}
          rows={4}
          autoCorrect="off"
          data-phone-start-text="true"
          aria-invalid={tooLong || undefined}
          className="min-h-(--size-touch-target) w-full resize-none rounded-lg border border-border bg-input px-md py-sm text-title text-foreground outline-none placeholder:text-muted-foreground focus-visible:ring-1 focus-visible:ring-ring"
        />
        <WhoWords selection={selection} choice={sheet.choice} disabled={pending} onChange={(choice) => editStartSheet({ choice })} />
        {shown ? (
          <p role="alert" className="text-body text-destructive" data-phone-start-error="true">
            {noticeText(t, shown)}
          </p>
        ) : null}
        <Button disabled={!ready} onClick={() => submitStart()} className="h-(--size-touch-target) w-full rounded-lg text-title font-semibold" data-phone-start-submit="true">
          {pending ? <Loader2Icon aria-hidden="true" className="size-(--size-icon-lg) animate-spin" /> : null}
          {t("mobile.start.submit")}
        </Button>
      </SheetContent>
    </Sheet>
  );
}
