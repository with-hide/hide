import { OfflineText } from "./OfflineText";
// The phone's start sheet (PRD home-device-rail D-24, B42-B46): what to do, where
// (default This Mac's Home), which agent kind and model, and Start. The kind and
// model start from the desktop's remembered choice. The text stays until a
// start succeeds, so a refusal or a lost connection never costs it.

import { Loader2Icon } from "lucide-react";
import type { ReactNode } from "react";
import { Button } from "../components/ui/button";
import { useInterfaceTranslation } from "../i18n/translator";
import { Sheet, SheetContent, SheetDescription, SheetTitle } from "../components/ui/sheet";
import { closeStartSheet, editStartSheet, submitStart } from "./connection";
import { noticeText, type StartKind } from "./protocol";
import { START_KINDS, modelsOf, selectionOf, startFailure, startProblem, targetText } from "./start";
import { usePhone } from "./store";

const SELECT_CLASS =
  "h-(--size-touch-target) w-full min-w-0 truncate rounded-lg border border-border bg-input px-md text-title text-foreground outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:opacity-50";

function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <label className="flex min-w-0 flex-col gap-xs text-body text-muted-foreground">
      <span>{label}</span>
      {children}
    </label>
  );
}

export function StartSheet() {
  const { t } = useInterfaceTranslation();
  const sheet = usePhone((s) => s.startSheet);
  const catalog = usePhone((s) => s.startCatalog);
  const connected = usePhone((s) => s.connected);
  const selection = selectionOf(catalog, sheet.choice);
  const models = modelsOf(selection);
  const problem = startProblem(sheet.text);
  const tooLong = problem === "too_long";
  const shown = tooLong ? startFailure("too_long") : sheet.error;
  const pending = sheet.pending !== null;
  const ready = connected && !pending && problem === null && selection.target !== null;
  return (
    <Sheet open={sheet.open} onOpenChange={(open) => !open && closeStartSheet()}>
      <SheetContent side="bottom" className="phone-safe-bottom max-h-full gap-md overflow-y-auto rounded-t-xl px-lg pt-lg pb-lg" data-phone-start-sheet="true">
        <SheetTitle className="text-headline">{t("mobile.start.title")}</SheetTitle>
        <SheetDescription className="sr-only">{t("mobile.start.description")}</SheetDescription>
        {!connected ? (
          <p role="status" className="flex items-center gap-md rounded-lg bg-secondary px-md py-sm text-subhead text-subtle-foreground" data-phone-start-unreachable="true">
            <Loader2Icon aria-hidden="true" className="size-(--size-icon-lg) shrink-0 animate-spin" />
            <span><OfflineText /></span>
          </p>
        ) : null}
        <textarea
          value={sheet.text}
          onChange={(event) => editStartSheet({ text: event.target.value })}
          placeholder={t("mobile.start.prompt")}
          aria-label={t("mobile.start.task")}
          rows={4}
          autoCorrect="off"
          data-phone-start-text="true"
          aria-invalid={tooLong || undefined}
          className="min-h-(--size-touch-target) w-full resize-none rounded-lg border border-border bg-input px-md py-sm text-title text-foreground outline-none placeholder:text-muted-foreground focus-visible:ring-1 focus-visible:ring-ring"
        />
        <Field label={t("mobile.start.target")}>
          <select
            value={selection.target?.id ?? ""}
            disabled={!selection.target}
            onChange={(event) => editStartSheet({ choice: { ...sheet.choice, target: event.target.value } })}
            data-phone-start-target="true"
            className={SELECT_CLASS}
          >
            {selection.target ? (
              catalog?.targets.map((target) => (
                <option key={target.id} value={target.id} disabled={!target.connected}>
                  {target.connected ? targetText(t, target, catalog.core_platform) : `${targetText(t, target, catalog.core_platform)} · ${t("mobile.start.targetDisconnected")}`}
                </option>
              ))
            ) : (
              <option value="">{t("mobile.start.home")}</option>
            )}
          </select>
        </Field>
        <div className="grid grid-cols-2 gap-sm">
          <Field label={t("mobile.start.kind")}>
            <select
              value={selection.kind}
              onChange={(event) => editStartSheet({ choice: { ...sheet.choice, kind: event.target.value as StartKind, model: undefined } })}
              data-phone-start-kind="true"
              className={SELECT_CLASS}
            >
              {START_KINDS.map((kind) => (
                <option key={kind.id} value={kind.id}>
                  {kind.label}
                </option>
              ))}
            </select>
          </Field>
          <Field label={t("mobile.start.model")}>
            <select
              value={selection.model}
              disabled={models.length === 0}
              onChange={(event) => editStartSheet({ choice: { ...sheet.choice, model: event.target.value } })}
              data-phone-start-model="true"
              className={SELECT_CLASS}
            >
              <option value="">{t("mobile.start.defaultModel")}</option>
              {selection.model && !models.includes(selection.model) ? <option value={selection.model}>{selection.model}</option> : null}
              {models.map((model) => (
                <option key={model} value={model}>
                  {model}
                </option>
              ))}
            </select>
          </Field>
        </div>
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
