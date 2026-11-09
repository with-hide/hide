// Settings › Agents' profiles (PRD agent-profiles B1-B8, B26-B28, B30): each
// agent's profiles unfolded under its row, added and edited in a dialog,
// deleted after the core said what the delete meets. The list is the core's
// (`kit.profiles`); this keeps only which dialog is open, its draft and the
// one request it waits on.

import { useEffect, useRef, useState, type ReactNode } from "react";
import type { Actions } from "../actions";
import { hostWays } from "../addProject";
import {
  addChange,
  agentProfilesView,
  answerRefusal,
  editChange,
  listedProfiles,
  profileAnswer,
  profileDraft,
  profileRequestId,
  profilesMachine,
  sameDraft,
  type ProfileChange,
} from "../agentProfiles";
import { InlineProfiles, ProfileDeleteDialog, ProfileFormDialog, ProfilesMachineNote, type ProfileDraft, type ProfileView } from "../components/agent-profiles";
import { Note } from "../components/settings-rows";
import { hostBridge } from "../host";
import { useInterfaceTranslation } from "../i18n/client";
import type { Device, KitAgent } from "../snapshot";
import { useShellStore } from "../store";
import { useUiStore } from "../ui";

type Open =
  | { kind: "add"; agent: string; draft: ProfileDraft }
  | { kind: "edit"; agent: string; id: string; draft: ProfileDraft; saved: ProfileDraft | null }
  | { kind: "delete"; agent: string; id: string; checked: boolean };

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
  const [open, setOpen] = useState<Open | null>(null);
  const [sent, setSent] = useState<Sent | null>(null);
  const again = useRef<ProfileDraft | null>(null);
  const answer = useShellStore((s) => profileAnswer(s.rest, sent?.id ?? null));
  const waiting = sent !== null && (answer === null || answer.state === "pending");

  const send = (change: ProfileChange, draft?: ProfileDraft) => {
    const id = profileRequestId();
    setSent({ id, change, draft });
    actions.changeAgentProfile(device.id, id, change);
  };

  // What the answer to the one request changes on screen.
  useEffect(() => {
    if (!sent || !answer || answer.state === "pending") return;
    const { change } = sent;
    setOpen((now) => {
      if (!now) return now;
      if (change.op === "values" && now.kind === "edit" && now.id === change.id && now.saved === null) {
        if (answer.state !== "done") return now;
        const draft = profileDraft(view(now.agent)?.profiles.find((row) => row.id === now.id) ?? null, answer.values ?? []);
        return { ...now, draft, saved: draft };
      }
      if (change.op === "add" && answer.state === "done") return null;
      if (change.op === "edit" && answer.state === "done" && now.kind === "edit" && sent.draft) return { ...now, saved: sent.draft };
      if (change.op === "remove_check" && now.kind === "delete" && answer.state === "done") return { ...now, checked: true };
      if (change.op === "remove") return answer.state === "refused" && answer.code === "profile_in_use" && now.kind === "delete" ? { ...now, checked: true } : null;
      return now;
    });
    // An edit made while the last one was on its way goes now.
    if (change.op === "edit" && again.current) {
      const draft = again.current;
      again.current = null;
      send(editChange(change.id, draft), draft);
    }
    // `view` and `send` read this render's values; the answer is what moves.
  }, [sent?.id, answer?.state]);

  function view(agent: string) {
    const kit = device.kit?.agents.find((row) => row.id === agent);
    return agentProfilesView(listed, agent, kit?.label ?? agent);
  }

  const refusal = answerRefusal(answer);
  const errorText = refusal.text ?? (refusal.key ? t(refusal.key as "agentProfiles.error.other") : null);
  const close = () => {
    setOpen(null);
    again.current = null;
  };
  const openEdit = (agent: string, profile: ProfileView) => {
    if (profile.builtIn) {
      setOpen({ kind: "edit", agent, id: profile.id, draft: profileDraft(profile, []), saved: profileDraft(profile, []) });
      return;
    }
    setOpen({ kind: "edit", agent, id: profile.id, draft: profileDraft(profile), saved: null });
    send({ op: "values", id: profile.id });
  };
  const commit = (id: string, saved: ProfileDraft | null, draft: ProfileDraft) => {
    // Nothing is saved before the variables' values are read: an edit
    // replaces them all.
    if (saved === null || sameDraft(saved, draft) || !draft.name.trim()) return;
    if (waiting) {
      again.current = draft;
      return;
    }
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
      const checked = open.checked && answer && (answer.state === "done" || answer.code === "profile_in_use") ? answer : null;
      overlay = (
        <ProfileDeleteDialog
          open={checked !== null}
          onOpenChange={(next) => !next && close()}
          profile={profile.name}
          location={profile.account ?? profile.program ?? ""}
          inUse={(checked?.in_use ?? []).map((agent) => ({ id: agent.pane_id, title: agent.title, place: agent.place, asleep: agent.asleep }))}
          projects={checked?.projects ?? []}
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
          onOpenChange={(next) => !next && close()}
          view={shown}
          mode={open.kind}
          profile={profile ?? undefined}
          draft={open.draft}
          refusals={refusal.refusals}
          error={errorText}
          busy={waiting}
          envLoading={editing?.saved === null}
          canChoose={native}
          onChooseFolder={() => void chooseFolder()}
          onDraft={(draft) => setOpen((now) => (now && now.kind !== "delete" ? { ...now, draft } : now))}
          onCommit={(draft) => editing && commit(editing.id, editing.saved, draft)}
          onSubmit={() => send(addChange(open.agent, shown.way, open.draft))}
          onMakeDefault={() => send({ op: "default", kind: open.agent, id: open.kind === "edit" ? open.id : "" })}
          onDelete={() => {
            if (open.kind !== "edit") return;
            setOpen({ kind: "delete", agent: open.agent, id: open.id, checked: false });
            send({ op: "remove_check", id: open.id });
          }}
        />
      );
    }
  }
  return { inline, note, overlay };
}
