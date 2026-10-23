import type { Actions } from "../actions";
import { Button } from "../components/ui/button";
import { Note, Status } from "../components/settings-rows";
import { useInterfaceTranslation } from "../i18n/client";
import { kitConsentTerms, kitPartLine, kitPartText, kitProblems, kitRefusalLine } from "../settings";
import { kitUnavailableText, kitWhyText } from "../kitWhy";
import type { Device } from "../snapshot";

export function KitTerms({ helperRoot, cliDir }: { helperRoot: string | null; cliDir: string | null }) {
  const { t } = useInterfaceTranslation();
  return (
    <ul className="list-disc space-y-xs pl-md text-body text-subtle-foreground" data-kit-terms="true">
      {kitConsentTerms(helperRoot, cliDir, t).map((term) => (
        <li key={term}>{term}</li>
      ))}
    </ul>
  );
}

/**
 * What a machine's row says about Hide's kit (PRD settings-cleanup B54, B56):
 * nothing while it is in place. Only a part that needs the operator shows, as
 * one line under the machine with Reinstall, and the line goes when the
 * Reinstall lands. A machine whose kit does not run says why, a pass that
 * stopped before changing anything says what stopped it once, with Try again,
 * and the first install says it is under way.
 */
export function KitProblem({ device, actions, onAct }: { device: Device; actions: Actions; onAct: () => void }) {
  const { t } = useInterfaceTranslation();
  const kit = device.kit;
  if (!kit) return null;
  if (kit.unavailable) {
    return (
      <div data-machine-kit={`${device.id}:unavailable`}>
        <Note data-kit-unavailable={kit.unavailable}>{kitUnavailableText(kit.unavailable, t)}</Note>
      </div>
    );
  }
  if (kit.refused) {
    // Try again runs the same pass, which checks again what stopped it.
    return (
      <div className="flex flex-wrap items-center gap-x-sm gap-y-xs" data-kit-refused={device.id} data-machine-kit={`${device.id}:${kit.busy ? "busy" : "refused"}`}>
        <Status tone="error">{kitRefusalLine(kit.refused, t)}</Status>
        <Button
          variant="secondary"
          size="sm"
          disabled={kit.busy}
          onClick={() => {
            onAct();
            actions.reinstallKit(device.id);
          }}
          data-kit-retry={device.id}
        >
          {kit.busy ? t("devices.installingKit") : t("common.retry")}
        </Button>
      </div>
    );
  }
  const problems = kitProblems(kit);
  if (problems.length === 0) {
    return kit.busy ? (
      <div data-machine-kit={`${device.id}:busy`}>
        <Status tone="pending">{t("devices.installingKit")}</Status>
      </div>
    ) : null;
  }
  const worst = problems.some((part) => part.state === "failed") ? "error" : "warn";
  return (
    <div className="flex flex-wrap items-center gap-x-sm gap-y-xs" data-kit-problem={device.id} data-machine-kit={`${device.id}:${kit.busy ? "busy" : "attention"}`}>
      <Status tone={worst}>{t("devices.kitProblem", { parts: problems.map((part) => `${part.label}: ${kitPartText(part, t)}`).join("; ") })}</Status>
      {kit.offers_reinstall ? (
        <Button
          variant="secondary"
          size="sm"
          disabled={kit.busy}
          onClick={() => {
            onAct();
            actions.reinstallKit(device.id);
          }}
          data-kit-reinstall={device.id}
        >
          {kit.busy ? t("settings.reinstalling") : t("settings.reinstall")}
        </Button>
      ) : null}
    </div>
  );
}

/**
 * Each part of Hide's kit on one machine, in the same form for This Mac and
 * every device (PRD device-parity B7): a mark, the part, and where it is or
 * why it is not. It lives in Connection details, so a healthy kit is
 * answerable on request and never on the row.
 */
export function KitParts({ device }: { device: Device }) {
  const { t } = useInterfaceTranslation();
  const kit = device.kit;
  if (!kit) return null;
  if (kit.unavailable) return <Note data-kit-unavailable={kit.unavailable}>{kitUnavailableText(kit.unavailable, t)}</Note>;
  if (kit.refused) return <Status tone="error">{kitRefusalLine(kit.refused, t)}</Status>;
  if (kit.components.length === 0) return <Status tone="pending">{kit.busy ? t("devices.installingKit") : t("devices.kitOnConnection")}</Status>;
  return (
    <div className="grid grid-cols-[auto_auto_minmax(0,1fr)] gap-x-xs gap-y-xxs" data-machine-kit={`${device.id}:${kit.busy ? "busy" : "read"}`}>
      {kit.components.map((part) => {
        const line = kitPartLine(part, t);
        const mark = part.state === "installed" ? "✓" : part.state === "absent" ? "–" : part.state === "off" ? "○" : part.state === "failed" ? "✕" : "!";
        const markTone = line.tone === "ok" ? "text-success" : line.tone === "muted" ? "text-muted-foreground" : line.tone === "error" ? "text-destructive" : "text-warning";
        return (
          <div key={part.id} className="col-span-3 grid grid-cols-subgrid text-caption" data-kit-part={`${device.id}:${part.id}:${part.state}`}>
            <span className={markTone} aria-hidden="true">
              {mark}
            </span>
            <span className="whitespace-nowrap text-foreground">{part.label}</span>
            {part.state === "installed" ? <span className="sr-only">{line.text}</span> : null}
            <span className="min-w-0 break-words text-subtle-foreground">
              {part.state === "installed" ? <span className="break-all font-mono">{part.location}</span> : kitPartText(part, t)}
              {/* An installed part can still carry a reason, such as a setting that applies to newly opened sessions. */}
              {part.state === "installed" && kitWhyText(part.why, t) ? <span className="block text-muted-foreground" data-kit-part-note={part.why?.cause}>{kitWhyText(part.why, t)}</span> : null}
            </span>
          </div>
        );
      })}
    </div>
  );
}
