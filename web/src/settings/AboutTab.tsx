import { useEffect, useState } from "react";
import type { Actions } from "../actions";
import { Button } from "../components/ui/button";
import { Switch } from "../components/ui/switch";
import { Disclosure, Group, Note, Row, Status, Value, type Tone } from "../components/settings-rows";
import { useInterfaceTranslation } from "../i18n/client";
import { formatDateTime, formatPercent } from "../i18n/format";
import { requireInterfaceLanguage } from "../i18n/locale";
import { diagnosticsInputOf, diagnosticsText, environmentTone, engineLine, shownIn } from "../settings";
import { useShellStore } from "../store";
import type { UpdateStatus } from "../host";
import { checkedAgo, checkForUpdates, restartIntoUpdate, setPreviewUpdates, useUpdateStatus } from "../updates";

/** What this hide is: its version and update, and the details folded away unless something is wrong. */
export function AboutTab({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const daemon = useShellStore((s) => s.daemon);
  const connection = useShellStore((s) => s.connection);
  const engine = useShellStore((s) => s.rest?.status?.herdr);
  const environment = useShellStore((s) => s.rest?.status?.environment);
  const [copy, setCopy] = useState<"idle" | "copied" | "failed">("idle");
  const line = engineLine(engine, t);
  const copyDiagnostics = () => {
    const text = diagnosticsText(diagnosticsInputOf(useShellStore.getState(), navigator.userAgent));
    void actions.copyText(text, "diagnostics").then((copied) => setCopy(copied ? "copied" : "failed"));
  };
  return (
    <Group data-settings-group="about">
      <Row label={t("settings.hideName")}>
        <Value>{daemon ? daemon.version : t("settings.unavailable")}</Value>
      </Row>
      <UpdateRows />
      <Disclosure title={t("settings.details")} data-settings-details="true">
        <Row label={t("settings.process")}>
          <Value>{daemon ? t("settings.processValue", { pid: String(daemon.pid), schema: String(daemon.schema_version) }) : t("settings.unavailable")}</Value>
        </Row>
        <Row label={t("settings.lifetime")}>
          <Value mono={false}>{daemon ? (daemon.keep_alive ? t("settings.keptAlive") : t("settings.exitsAfter", { minutes: Math.round(daemon.idle_secs / 60) })) : t("settings.unavailable")}</Value>
        </Row>
        <Row label={t("settings.stateFile")}>
          <Value>{shownIn(daemon?.core_state_path, t)}</Value>
        </Row>
        <Row label={t("settings.pageConnection")}>
          <Status tone={connection === "live" ? "ok" : "warn"} data-page-connection={connection}>
            {t(`settings.connection.${connection}`)}
          </Status>
        </Row>
        <Row
          label={t("settings.herdrRuntime")}
          detail={
            line.tone !== "ok" && engine?.message ? (
              <Note tone={line.tone === "error" ? "error" : "warn"} data-engine-message="true">
                {engine.message}
              </Note>
            ) : null
          }
        >
          <Status tone={line.tone === "ok" ? "ok" : "warn"} data-engine-state={engine?.state ?? "unavailable"}>
            {line.text}
          </Status>
        </Row>
        {(environment ?? []).map((row) => (
          <Row key={row.key} label={<span className="font-mono">{row.key}</span>} detail={row.message ? <Note>{row.message}</Note> : null}>
            <Status tone={environmentTone(row.state, row.required)}>{row.state.replace(/_/g, " ")}</Status>
          </Row>
        ))}
        <Row label={t("settings.recent")} detail={<DiagnosticList />}>
          <Button variant="secondary" onClick={copyDiagnostics} data-copy-diagnostics="true">
            {t("settings.copyDiagnostics")}
          </Button>
          {copy === "copied" ? <Status tone="ok">{t("common.copied")}</Status> : null}
          {copy === "failed" ? <Status tone="error">{t("settings.clipboardRefused")}</Status> : null}
        </Row>
      </Disclosure>
    </Group>
  );
}

/**
 * This app's update and its preview switch (PRD auto-update D-20, B25-B28):
 * the one place a check, a download or a failure shows. Only the desktop app
 * has them; a browser tab and the phone draw neither row (B24).
 */
export function UpdateRows() {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const status = useUpdateStatus();
  const idle = status?.phase.kind === "idle";
  const [now, setNow] = useState(() => Date.now());
  // "checked 10 min. ago" stays true while the sheet is open.
  useEffect(() => {
    if (!idle) return;
    setNow(Date.now());
    const timer = window.setInterval(() => setNow(Date.now()), 30_000);
    return () => window.clearInterval(timer);
  }, [idle]);
  if (!status) return null;
  const line = updateLine(status, t, (checkedAt) => checkedAgo(language, checkedAt, now), (percent) => formatPercent(language, percent));
  return (
    <>
      <Row label={t("settings.update.label")} data-settings-update={status.phase.kind}>
        <Status tone={line.tone}>{line.text}</Status>
        {line.action ? (
          <Button variant="secondary" size="sm" onClick={line.action === "restart" ? restartIntoUpdate : checkForUpdates} data-update-action={line.action}>
            {t(line.action === "restart" ? "settings.update.restartNow" : line.action === "retry" ? "settings.update.checkAgain" : "settings.update.checkNow")}
          </Button>
        ) : null}
      </Row>
      <Row
        label={
          <span className="flex min-w-0 flex-col gap-xxs">
            <span>{t("settings.update.prerelease")}</span>
            <span className="text-body text-muted-foreground">{t("settings.update.prereleaseNote")}</span>
          </span>
        }
      >
        <Switch
          checked={status.prerelease}
          disabled={status.phase.kind === "off"}
          onCheckedChange={setPreviewUpdates}
          aria-label={t("settings.update.prerelease")}
          data-update-prerelease={String(status.prerelease)}
        />
      </Row>
    </>
  );
}

type UpdateLine = { tone: Tone; text: string; action: "check" | "retry" | "restart" | null };

/** What the update row says and offers in each phase. */
function updateLine(status: UpdateStatus, t: ReturnType<typeof useInterfaceTranslation>["t"], ago: (checkedAt: number) => string, percent: (value: number) => string): UpdateLine {
  const { phase } = status;
  switch (phase.kind) {
    case "off":
      return phase.reason === "location"
        ? { tone: "warn", text: t("settings.update.moveToApplications"), action: null }
        : { tone: "muted", text: t("settings.update.development"), action: null };
    case "idle":
      return phase.checkedAt === null
        ? { tone: "muted", text: t("settings.update.notChecked"), action: "check" }
        : { tone: "ok", text: t("settings.update.latest", { when: ago(phase.checkedAt) }), action: "check" };
    case "checking":
      return { tone: "pending", text: t("settings.update.checking"), action: null };
    case "downloading":
      return { tone: "pending", text: t("settings.update.downloading", { version: phase.version, percent: percent(phase.percent) }), action: null };
    case "ready":
      return { tone: "local", text: t(status.installOnQuit ? "settings.update.readyOnQuit" : "settings.update.readyOnRestart", { version: phase.version }), action: "restart" };
    case "failed":
      return { tone: "error", text: t(phase.stage === "check" ? "settings.update.checkFailed" : "settings.update.downloadFailed"), action: "retry" };
  }
}

function DiagnosticList() {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const diagnostics = useShellStore((s) => s.rest?.status?.diagnostics);
  const rows = (diagnostics ?? []).slice(-8).reverse();
  if (rows.length === 0) return <Note>{t("settings.noDiagnostics")}</Note>;
  return (
    <ul className="space-y-xxs" data-diagnostics={rows.length}>
      {rows.map((row) => (
        <li key={`${row.occurred_at}-${row.kind}`} className="break-words font-mono text-micro text-subtle-foreground">
          <span className="text-muted-foreground">{formatDateTime(language, row.occurred_at, { timeStyle: "medium" })}</span> {row.kind}: {row.message}
        </li>
      ))}
    </ul>
  );
}
