// Settings › Agents' profiles (PRD agent-profiles B1-B8, B26-B28, B30): each
// agent's profiles unfolded under its row, added and edited in a dialog,
// deleted after the core said what the delete meets. The list is the core's
// (`kit.profiles`); this keeps only which dialog is open, its draft and the
// one request it waits on.

import { useEffect, useState, type ReactNode } from "react";
import type { Actions } from "../actions";
import { hostWays } from "../addProject";
import {
  addChange,
  agentProfilesView,
  answerRefusal,
  editChange,
  listedProfiles,
  ownAgentUsage,
  profileAnswer,
  profileDraft,
  profileRequestId,
  profilesMachine,
  sameDraft,
  type ProfileChange,
  type Worded,
} from "../agentProfiles";
import { InlineProfiles, ProfileDeleteDialog, ProfileFormDialog, ProfilesMachineNote, type ProfileDraft, type ProfileView } from "../components/agent-profiles";
import { Note } from "../components/settings-rows";
import { hostBridge } from "../host";
import { useInterfaceTranslation } from "../i18n/client";
import type { AgentProfileAnswer, Device, KitAgent } from "../snapshot";
import { useShellStore } from "../store";
import { useUiStore } from "../ui";

type Open =
  | { kind: "add"; agent: string; draft: ProfileDraft }
  // `saved` is the machine's values once read (null before: nothing is saved
  // then, since an edit replaces every variable); `closing` holds a close
  // the operator asked for until the last draft is saved.
  | { kind: "edit"; agent: string; id: string; draft: ProfileDraft; saved: ProfileDraft | null; closing: boolean }
  // `found` is what the core's last check, or a refused delete, says the
  // delete meets: kept while the delete itself is on its way.
  | { kind: "delete"; agent: string; id: string; found: Found | null };

type Found = { inUse: NonNullable<AgentProfileAnswer["in_use"]>; projects: string[] };

type Sent = { id: string; change: ProfileChange; draft?: ProfileDraft };

/**
 * One machine's profiles for the Agents tab: `inline(agent)` under each
 * installed agent's row, `overlay` once for the dialogs and the line a
 * refused change leaves when no dialog is open.
 */
export function useMachineProfiles(device: Device, live: boolean, actions: Actions, name: string): { inline: (agent: KitAgent) => ReactNode; note: ReactNode; overlay: ReactNode } {
  const { t } = useInterfaceTranslation();
  const listed = listedProfiles(device);
  const machine = profilesMachine(device, live);
  // The folder chooser is this computer's, so only its own machine offers it (B30).
  const native = useShellStore((s) => hostWays(device.id, s.rest?.navigator?.devices, hostBridge() !== null).native);
  // Limits are read on this machine alone, for its own profiles (B23).
  const ownUsage = useShellStore((s) => (s.usage?.nodes[0]?.node === device.id ? s.usage : null));
  const [open, setOpen] = useState<Open | null>(null);
  const [sent, setSent] = useState<Sent | null>(null);
  const answer = useShellStore((s) => profileAnswer(s.rest, sent?.id ?? null));
  const waiting = sent !== null && (answer === null || answer.state === "pending");

  const send = (change: ProfileChange, draft?: ProfileDraft) => {
    const id = profileRequestId();
    setSent({ id, change, draft });
    actions.changeAgentProfile(device.id, id, change);
  };

  // What the answer to the one request changes on screen. `open`, `view`
  // and `send` are this render's; the answer is what moves.
  useEffect(() => {
    if (!sent || !answer || answer.state === "pending" || !open) return;
    const { change } = sent;
    const done = answer.state === "done";
    if (open.kind === "edit") {
      let next = open;
      if (change.op === "values" && change.id === open.id && open.saved === null) {
        // Only a read gives the variables; what was typed meanwhile stays.
        if (done) {
          const values = answer.values ?? [];
          const saved = profileDraft(view(open.agent)?.profiles.find((row) => row.id === open.id) ?? null, values);
          next = { ...open, saved, draft: { ...open.draft, env: saved.env } };
        } else next = { ...open, closing: false };
      } else if (change.op === "edit" && change.id === open.id && sent.draft) {
        next = done ? { ...open, saved: sent.draft } : { ...open, closing: false };
      }
      // The last draft goes once the one before it is answered; a refused
      // one waits for the operator's next change.
      const dirty = next.saved !== null && !sameDraft(next.saved, next.draft) && next.draft.name.trim() !== "";
      if (done && dirty) {
        setOpen(next);
        send(editChange(next.id, next.draft), next.draft);
      } else if (done && next.closing && next.saved !== null) setOpen(null);
      else if (next !== open) setOpen(next);
      return;
    }
    if (change.op === "add" && done && open.kind === "add") setOpen(null);
    if (open.kind !== "delete") return;
    if (change.op === "remove_check" && done) setOpen({ ...open, found: { inUse: answer.in_use ?? [], projects: answer.projects ?? [] } });
    // An agent that started with it since the check refuses the delete: the
    // dialog lists it then, as the check would have.
    else if (change.op === "remove") setOpen(answer.code === "profile_in_use" ? { ...open, found: { inUse: answer.in_use ?? [], projects: [] } } : null);
  }, [sent?.id, answer?.state]);

  function view(agent: string) {
    const kit = device.kit?.agents.find((row) => row.id === agent);
    return agentProfilesView(listed, agent, kit?.label ?? agent, ownUsage ? ownAgentUsage(ownUsage, agent) : null);
  }

  const refusal = answerRefusal(answer);
  const word = ({ key, params }: Worded) => t(key as "agentProfiles.error.other", params);
  const errorText = refusal.line ? word(refusal.line) : null;
  const fieldRefusals = refusal.refusals.map((row) => ({ field: row.field, text: word(row), ...(row.envKey ? { envKey: row.envKey } : {}) }));
  const close = () => setOpen(null);
  /**
   * Closing an edit saves its last draft first (B26): the dialog stays,
   * saving, until the machine took it, and a refusal keeps it open with the
   * values and the reason. Values never read are not saved.
   */
  const requestClose = () => {
    if (open?.kind !== "edit") return close();
    // A nameless draft cannot be saved, as on leaving a field.
    const dirty = open.saved !== null && !sameDraft(open.saved, open.draft) && open.draft.name.trim() !== "";
    if (!dirty && !waiting) return close();
    setOpen({ ...open, closing: true });
    if (dirty && !waiting) send(editChange(open.id, open.draft), open.draft);
  };
  const openEdit = (agent: string, profile: ProfileView) => {
    if (profile.builtIn) {
      setOpen({ kind: "edit", agent, id: profile.id, draft: profileDraft(profile, []), saved: profileDraft(profile, []), closing: false });
      return;
    }
    setOpen({ kind: "edit", agent, id: profile.id, draft: profileDraft(profile), saved: null, closing: false });
    send({ op: "values", id: profile.id });
  };
  const commit = (id: string, saved: ProfileDraft | null, draft: ProfileDraft) => {
    // Nothing is saved before the variables' values are read (an edit
    // replaces them all), and one on its way sends the latest draft when
    // it is answered.
    if (saved === null || waiting || sameDraft(saved, draft) || !draft.name.trim()) return;
    send(editChange(id, draft), draft);
  };
  const chooseFolder = async () => {
    const folder = await actions.pickFolder();
    if (folder) setOpen((now) => (now?.kind === "add" ? { ...now, draft: { ...now.draft, account: folder } } : now));
  };

  const inline = (agent: KitAgent) => {
    if (!machine || machine === "unsupported") return null;
    const shown = view(agent.id);
    if (!shown) return null;
    return (
      <InlineProfiles
        view={shown}
        machine={machine}
        onAdd={() => {
          if (!waiting) setSent(null);
          setOpen({ kind: "add", agent: agent.id, draft: profileDraft(null) });
        }}
        onOpen={(id) => {
          const profile = shown.profiles.find((row) => row.id === id);
          if (profile) openEdit(agent.id, profile);
        }}
        onAction={(id, action) => {
          if (action === "reinstall") send({ op: "reinstall", id });
          else if (action === "retry") send({ op: "remove", id });
          // Signing in happens in the agent the profile starts: from the start panel.
          else actions.openStartPanel();
        }}
      />
    );
  };

  const shown = open ? view(open.agent) : null;
  const profile = open && open.kind !== "add" ? (shown?.profiles.find((row) => row.id === open.id) ?? null) : null;
  // A refusal is the open form's (its requests are the last ones sent), else a line under the list.
  const note =
    !open && answer?.state === "refused" && errorText ? (
      <Note tone="error" data-profiles-error={answer.code ?? "refused"}>
        {errorText}
      </Note>
    ) : machine === "unsupported" && device.kit?.agents.some((row) => agentProfilesView(null, row.id, row.label)) ? (
      <ProfilesMachineNote machine="unsupported" device={name} />
    ) : null;

  let overlay: ReactNode = null;
  if (open && shown && (open.kind === "add" || profile)) {
    if (open.kind === "delete" && profile) {
      const found = open.found;
      overlay = (
        <ProfileDeleteDialog
          open={found !== null}
          onOpenChange={(next) => !next && close()}
          profile={profile.name}
          location={profile.account ?? profile.program ?? ""}
          inUse={(found?.inUse ?? []).map((agent) => ({ id: agent.pane_id, title: agent.title, place: agent.place, asleep: agent.asleep, starting: agent.starting }))}
          projects={found?.projects ?? []}
          busy={waiting}
          onConfirm={() => send({ op: "remove", id: profile.id })}
          onShow={(pane) => {
            close();
            useUiStore.getState().closeOverlay();
            actions.openListedAgent(pane);
          }}
        />
      );
    } else if (open.kind !== "delete") {
      const editing = open.kind === "edit" ? open : null;
      overlay = (
        <ProfileFormDialog
          open
          onOpenChange={(next) => !next && requestClose()}
          view={shown}
          mode={open.kind}
          profile={profile ?? undefined}
          draft={open.draft}
          refusals={fieldRefusals}
          error={errorText}
          busy={waiting || editing?.closing === true}
          envLoading={editing?.saved === null}
          canChoose={native}
          onChooseFolder={() => void chooseFolder()}
          onDraft={(draft) => setOpen((now) => (now && now.kind !== "delete" ? { ...now, draft } : now))}
          onCommit={(draft) => editing && commit(editing.id, editing.saved, draft)}
          onSubmit={() => send(addChange(open.agent, shown.way, open.draft))}
          onMakeDefault={() => send({ op: "default", kind: open.agent, id: open.kind === "edit" ? open.id : "" })}
          onDelete={() => {
            if (open.kind !== "edit") return;
            setOpen({ kind: "delete", agent: open.agent, id: open.id, found: null });
            send({ op: "remove_check", id: open.id });
          }}
        />
      );
    }
  }
  return { inline, note, overlay };
}
