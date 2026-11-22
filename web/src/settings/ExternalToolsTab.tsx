// Settings › 봇 (Bots; PRD hide-external-control B2-B4, B11-B13, D-33): the
// operator makes a bot here (a name and a mode) and sends the sentence with its
// one-time code to the AI that will use it; the bots waiting for that AI, each
// with the time its code has left; the requests a program put up itself, each
// approved into the bot's own space or for all of Hide, or refused; and the
// connected bots as rows like Mobile's paired phones, each with its mode,
// Change mode, Disconnect, and how many agents and missions it started. Every
// value is the core's `access` section, every press one `access_*` event or,
// for a code, one `access_call` (`accessCall.ts`); a request reaches the
// operator here only, never as a turn, a badge or a notice (D-24).

import { ChevronDownIcon, PlusIcon } from "lucide-react";
import { useEffect, useState } from "react";
import { callAccess, type MadeBot } from "../accessCall";
import type { Actions } from "../actions";
import { BOT_MODES, type BotModeKind } from "../bots";
import { BotFace } from "../components/bot-mark";
import { useMinuteClock, useRemaining } from "../components/elapsed";
import { Group, Note } from "../components/settings-rows";
import { Badge } from "../components/ui/badge";
import { Button } from "../components/ui/button";
import { Dialog, DialogBody, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "../components/ui/dialog";
import { DropdownMenu, DropdownMenuContent, DropdownMenuRadioGroup, DropdownMenuRadioItem, DropdownMenuTrigger } from "../components/ui/dropdown-menu";
import { Input } from "../components/ui/input";
import { RadioGroup, RadioGroupItem } from "../components/ui/radio-group";
import { BOT_NAME, connectionLine, createRefusal, pendingLine, warnsOfWork } from "../externalTools";
import { useInterfaceTranslation } from "../i18n/client";
import { requireInterfaceLanguage } from "../i18n/locale";
import { ownWindow } from "../pairing";
import { coreMachineName, coreMachineOwnName } from "../screenMachine";
import type { AccessConnection, AccessInvite, AccessPending } from "../snapshot";
import { useShellStore } from "../store";

const NO_INVITES: AccessInvite[] = [];

export function ExternalToolsTab({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const access = useShellStore((s) => s.access);
  // Every bot runs on the core's machine (D-07), which a window on another machine names.
  const machine = useShellStore((s) => coreMachineName(s.rest?.navigator?.devices, t));
  const now = useMinuteClock();
  // Only this machine's own window lets a bot in or changes its mode (PRD
  // device-connect-iroh B52); another machine's view lists the bots.
  const own = useShellStore((s) => ownWindow(s.pairing));
  // The sentence names the machine by its own name: the AI that reads it calls no machine This Mac.
  const sentenceMachine = useShellStore((s) => coreMachineOwnName(s.rest?.navigator?.devices, t));
  const [creating, setCreating] = useState(false);
  // The sentence on screen, and whether the section has listed its bot yet: the answer can come before the snapshot that lists it.
  const [shown, setShown] = useState<{ bot: MadeBot; listed: boolean } | null>(null);
  const invites = access?.invites ?? NO_INVITES;
  // A sentence whose bot no longer waits (joined, taken back, or gone with a restart) closes: its code joins nothing more.
  useEffect(() => {
    if (!shown) return;
    const listed = invites.some((invite) => invite.id === shown.bot.id);
    if (listed && !shown.listed) setShown({ ...shown, listed: true });
    else if (!listed && shown.listed) setShown(null);
  }, [invites, shown]);
  const show = (bot: MadeBot) => setShown({ bot, listed: false });
  if (!access) return <Note tone="pending">{t("externalTools.connecting")}</Note>;
  return (
    <div data-external-tools="true">
      {own && invites.length > 0 ? (
        <Group title={t("externalTools.invites.title")} caption={String(invites.length)} data-access-invite-count={String(invites.length)}>
          {invites.map((invite) => <InviteRow key={invite.id} invite={invite} machine={sentenceMachine} actions={actions} onShow={show} />)}
        </Group>
      ) : null}
      {own && access.pending.length > 0 ? (
        <Group title={t("externalTools.pending.title")} caption={String(access.pending.length)} data-access-pending-count={String(access.pending.length)}>
          {access.pending.map((request) => <PendingRow key={request.id} request={request} machine={machine} now={now} actions={actions} />)}
        </Group>
      ) : null}
      <Group
        title={t("externalTools.connected.title")}
        caption={<span className="tabular-nums" data-access-count={`${access.connections.length}/${access.limit}`}>{t("externalTools.connected.count", { connected: access.connections.length, limit: access.limit })}</span>}
        action={own ? (
          <Button variant="secondary" size="sm" onClick={() => setCreating(true)} data-access-create="true">
            <PlusIcon aria-hidden="true" />
            {t("externalTools.create")}
          </Button>
        ) : undefined}
        data-access-connections={String(access.connections.length)}
      >
        {access.connections.length > 0
          ? access.connections.map((connection) => <ConnectionRow key={connection.id} connection={connection} machine={machine} now={now} actions={own ? actions : null} />)
          : <Empty />}
      </Group>
      {creating ? <CreateDialog onClose={() => setCreating(false)} onMade={(made) => { setCreating(false); show(made); }} /> : null}
      {shown ? <SentenceDialog bot={shown.bot} machine={sentenceMachine} actions={actions} onClose={() => setShown(null)} /> : null}
    </div>
  );
}

function Empty() {
  const { t } = useInterfaceTranslation();
  return (
    <div className="px-md py-sm" data-access-empty="true">
      <span className="text-subhead text-foreground">{t("externalTools.empty")}</span>
    </div>
  );
}

/** The two modes, each with the line that says what it means for the operator; its own space first, the narrower. */
function ModeChoice({ mode, onChange }: { mode: BotModeKind; onChange: (mode: BotModeKind) => void }) {
  const { t } = useInterfaceTranslation();
  return (
    <RadioGroup value={mode} onValueChange={(next) => onChange(next as BotModeKind)} aria-label={t("externalTools.mode.label")} className="grid grid-cols-[repeat(auto-fit,minmax(min(100%,var(--size-settings-control-w)),1fr))] gap-sm" data-access-choice={mode}>
      {BOT_MODES.map((option) => (
        <label key={option} className="flex cursor-pointer items-start gap-sm rounded-md border border-border px-md py-sm has-data-[state=checked]:border-primary">
          <RadioGroupItem value={option} className="mt-xxs" data-access-option={option} />
          <span className="flex min-w-0 flex-col">
            <span className="text-subhead font-medium text-foreground">{t(`externalTools.mode.${option}`)}</span>
            <span className="break-words break-keep text-body text-muted-foreground">{t(`externalTools.mode.${option}Description`)}</span>
          </span>
        </label>
      ))}
    </RadioGroup>
  );
}

/**
 * Making a bot: the name the AI will use in Hide, with the owl it gets, and
 * the mode. The core answers with the bot and its code, or why it made none,
 * which the dialog says under the name.
 */
function CreateDialog({ onClose, onMade }: { onClose: () => void; onMade: (made: MadeBot) => void }) {
  const { t } = useInterfaceTranslation();
  const [name, setName] = useState("");
  const [mode, setMode] = useState<BotModeKind>("own");
  const [refusal, setRefusal] = useState<ReturnType<typeof createRefusal> | null>(null);
  const [busy, setBusy] = useState(false);
  const ready = BOT_NAME.test(name) && !busy;
  const make = async () => {
    if (!ready) return;
    setBusy(true);
    const answer = await callAccess({ call: "invite", name, mode: { kind: mode } });
    setBusy(false);
    if (answer.ok) onMade(answer.value);
    else setRefusal(createRefusal(answer.code));
  };
  return (
    <Dialog open onOpenChange={(next) => { if (!next) onClose(); }}>
      <DialogContent data-access-create-dialog="true">
        <DialogHeader>
          <DialogTitle>{t("externalTools.create.title")}</DialogTitle>
          <DialogDescription>{t("externalTools.create.description")}</DialogDescription>
        </DialogHeader>
        <DialogBody className="flex flex-col gap-md">
          <label className="flex flex-col gap-xs">
            <span className="text-body text-foreground">
              {t("externalTools.create.name")} <span className="text-muted-foreground">· {t("externalTools.create.nameHint")}</span>
            </span>
            <span className="flex items-center gap-sm">
              <BotFace name={name} size="row" />
              <Input
                value={name}
                maxLength={64}
                autoFocus
                spellCheck={false}
                autoCapitalize="off"
                autoComplete="off"
                onChange={(event) => { setName(event.target.value); setRefusal(null); }}
                onKeyDown={(event) => { if (event.key === "Enter") void make(); }}
                data-access-create-name="true"
              />
            </span>
            {refusal ? <span className="text-body text-destructive" data-access-create-refusal="true">{t(refusal)}</span> : null}
          </label>
          <ModeChoice mode={mode} onChange={setMode} />
        </DialogBody>
        <DialogFooter>
          <Button variant="text" onClick={onClose}>{t("externalTools.create.cancel")}</Button>
          <Button disabled={!ready} onClick={() => void make()} data-access-create-submit="true">{t("externalTools.create.submit")}</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/** The sentence the operator sends to the AI that will use the bot: its name, the core's machine and the code, ready to copy. */
function sentenceOf(bot: MadeBot, machine: string, t: ReturnType<typeof useInterfaceTranslation>["t"]): string {
  return t("externalTools.sentence.text", { name: bot.name, machine, code: bot.code });
}

/** The made bot's sentence, shown once it is made or given a new code; it closes on its own once the bot no longer waits. */
function SentenceDialog({ bot, machine, actions, onClose }: { bot: MadeBot; machine: string; actions: Actions; onClose: () => void }) {
  const { t } = useInterfaceTranslation();
  const [copied, setCopied] = useState(false);
  const sentence = sentenceOf(bot, machine, t);
  return (
    <Dialog open onOpenChange={(next) => { if (!next) onClose(); }}>
      <DialogContent data-access-sentence={bot.id}>
        <DialogHeader>
          <DialogTitle>{t("externalTools.sentence.title")}</DialogTitle>
          <DialogDescription>{t("externalTools.sentence.lead", { machine, name: bot.name })}</DialogDescription>
        </DialogHeader>
        <DialogBody className="flex flex-col gap-sm">
          <p className="select-text break-words rounded-md border border-border bg-card px-md py-sm text-body text-foreground" data-access-sentence-text="true">{sentence}</p>
          <span className="text-caption text-muted-foreground">{t("externalTools.sentence.note")}</span>
        </DialogBody>
        <DialogFooter>
          <Button variant="text" onClick={onClose} data-access-sentence-close="true">{t("externalTools.sentence.close")}</Button>
          <Button onClick={() => void actions.copyText(sentence, "bot sentence").then(setCopied)} data-access-sentence-copy="true">
            {copied ? t("externalTools.sentence.copied") : t("externalTools.sentence.copy")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/**
 * A bot the operator made that waits for its AI: the owl dimmed, the time its
 * code has left, Copy sentence and Cancel; once the code ran out, Code
 * expired, New code and Cancel. Copy reads the live code again from the core,
 * which only this window gets; New code opens the sentence with the new one.
 */
function InviteRow({ invite, machine, actions, onShow }: { invite: AccessInvite; machine: string; actions: Actions; onShow: (made: MadeBot) => void }) {
  const { t } = useInterfaceTranslation();
  const remaining = useRemaining(invite.expires_ms, invite.created_ms);
  const expired = remaining === "past";
  const [copied, setCopied] = useState(false);
  const copy = async () => {
    const answer = await callAccess({ call: "code", id: invite.id });
    if (answer.ok) setCopied(await actions.copyText(sentenceOf(answer.value, machine, t), "bot sentence"));
  };
  const renew = async () => {
    const answer = await callAccess({ call: "renew", id: invite.id });
    if (answer.ok) onShow(answer.value);
  };
  const mode = invite.mode.kind;
  return (
    <div className="flex flex-wrap items-center gap-x-md gap-y-xs px-md py-sm" data-access-invite={invite.id} data-access-invite-expired={String(expired)}>
      <BotFace name={invite.name} size="tile" mode={mode} className="opacity-(--opacity-dimmed)" />
      <span className="flex min-w-[min(100%,var(--size-settings-control-w))] flex-1 flex-col gap-xxs">
        <span className="flex min-w-0 flex-wrap items-center gap-x-sm gap-y-xxs">
          <span className="break-words text-subhead font-medium text-foreground">{invite.name}</span>
          <Badge variant="secondary" data-access-mode-pill={mode}>{t(`externalTools.mode.${mode}`)}</Badge>
        </span>
        <span className="text-body text-muted-foreground" data-access-invite-line={invite.id}>
          {expired ? t("externalTools.invites.expired") : remaining ? t("externalTools.invites.left", { left: remaining.text }) : null}
        </span>
      </span>
      <span className="ml-auto flex flex-wrap items-center justify-end gap-sm">
        {expired ? (
          <Button variant="text" size="sm" onClick={() => void renew()} data-access-invite-renew={invite.id}>{t("externalTools.invites.renew")}</Button>
        ) : (
          <Button variant="text" size="sm" onClick={() => void copy()} data-access-invite-copy={invite.id}>
            {copied ? t("externalTools.sentence.copied") : t("externalTools.invites.copy")}
          </Button>
        )}
        <Button variant="text" size="sm" className="text-destructive" onClick={() => actions.cancelAccessInvite(invite.id)} data-access-invite-cancel={invite.id}>
          {t("externalTools.invites.cancel")}
        </Button>
      </span>
    </div>
  );
}

/**
 * One request (B2, B3): the owl it will get, its name, machine and program,
 * the two modes with its own space chosen (the narrower), and the approve
 * that names the choice beside Refuse.
 */
function PendingRow({ request, machine, now, actions }: { request: AccessPending; machine: string; now: number; actions: Actions }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const [mode, setMode] = useState<BotModeKind>("own");
  const line = pendingLine(request, machine, now, t, language);
  return (
    <div className="flex flex-col gap-md px-md py-md" data-access-pending={request.id}>
      <span className="flex min-w-0 items-center gap-md">
        <BotFace name={request.name} size="tile" mode={mode} />
        <span className="flex min-w-0 flex-col">
          <span className="break-words text-subhead font-medium text-foreground">{request.name}</span>
          <span className="min-w-0 break-words text-body text-muted-foreground">
            {line.machine} · <span className="break-all font-mono" data-access-program="true">{line.program}</span> · {line.requested}
          </span>
        </span>
      </span>
      <ModeChoice mode={mode} onChange={setMode} />
      <div className="flex flex-wrap items-center justify-end gap-sm">
        <Button variant="text" size="sm" onClick={() => actions.refuseAccess(request.id)} data-access-refuse={request.id}>
          {t("externalTools.refuse")}
        </Button>
        <Button variant="secondary" size="sm" onClick={() => actions.approveAccess(request.id, mode)} data-access-approve={mode}>
          {t(`externalTools.approve.${mode}`)}
        </Button>
      </div>
    </div>
  );
}

/**
 * One connected bot (B11-B13): its owl, its name with its mode, and its line,
 * which counts the agents and missions it started; Change mode, a menu of the
 * two modes, and Disconnect, which acts at once; and the warning when its work
 * would become the operator's.
 */
function ConnectionRow({ connection, machine, now, actions }: { connection: AccessConnection; machine: string; now: number; actions: Actions | null }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const mode = connection.mode.kind;
  return (
    <div className="flex flex-wrap items-center gap-x-md gap-y-xs px-md py-sm" data-access-connection={connection.id} data-access-mode={mode}>
      <BotFace name={connection.name} size="tile" mode={mode} />
      <span className="flex min-w-[min(100%,var(--size-settings-control-w))] flex-1 flex-col gap-xxs">
        <span className="flex min-w-0 flex-wrap items-center gap-x-sm gap-y-xxs">
          <span className="break-words text-subhead font-medium text-foreground">{connection.name}</span>
          <Badge variant="secondary" data-access-mode-pill={mode}>{t(`externalTools.mode.${mode}`)}</Badge>
        </span>
        <span className="break-words text-body text-muted-foreground" data-access-line={connection.id}>
          {connectionLine(connection, machine, now, t, language).map((piece, index) => (
            <span key={index}>{index > 0 ? " · " : ""}<span className="whitespace-nowrap">{piece}</span></span>
          ))}
        </span>
        {warnsOfWork(connection) ? <span className="mt-xxs" data-access-warning={connection.id}><Note tone="warn">{t("externalTools.workWarning")}</Note></span> : null}
      </span>
      {actions ? <span className="ml-auto flex flex-wrap items-center justify-end gap-sm">
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="text" size="sm" data-access-change-mode={connection.id}>
              {t("externalTools.changeMode")}
              <ChevronDownIcon aria-hidden="true" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuRadioGroup value={mode} onValueChange={(next) => next !== mode && actions.setAccessMode(connection.id, next as BotModeKind)} aria-label={t("externalTools.mode.label")}>
              {BOT_MODES.map((option) => (
                <DropdownMenuRadioItem key={option} value={option} data-access-mode-option={option}>{t(`externalTools.mode.${option}`)}</DropdownMenuRadioItem>
              ))}
            </DropdownMenuRadioGroup>
          </DropdownMenuContent>
        </DropdownMenu>
        <Button variant="text" size="sm" className="text-destructive" onClick={() => actions.revokeAccess(connection.id)} data-access-revoke={connection.id}>
          {t("externalTools.disconnect")}
        </Button>
      </span> : null}
    </div>
  );
}
