import { useEffect, useRef } from "react";
import type { Actions } from "./actions";
import { Button } from "./components/ui/button";
import { useInterfaceTranslation } from "./i18n/client";
import { conductorResume } from "./missions";
import { localDeviceId, type Mission } from "./snapshot";
import { useShellStore } from "./store";
import { useSessionResume } from "./useSessionResume";

/**
 * 이어받기 (PRD mission-tool D-11, B13-B15): the gone Conductor's own
 * conversation is resumed first, so the reasons behind its plan come back
 * with it, and the core moves the mission to the pane it resumes in. A
 * Conductor with no conversation to resume, a resume the core refused or
 * the agent failed, and a second press after a resume that brought no
 * Conductor back start a new agent told to take the mission; only when that
 * start fails too does the button say so, and pressing it tries again.
 */
export function MissionTakeOver({ mission, actions }: { mission: Mission; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const node = useShellStore((state) => localDeviceId(state.rest));
  const resumed = useSessionResume();
  const fresh = useSessionResume();
  const resumeTried = useRef(false);
  const session = conductorResume(mission);
  const busy = (resumed.resume.request !== null && !resumed.resume.uncertain) || (fresh.resume.request !== null && !fresh.resume.uncertain);
  const startFresh = () => fresh.start((requestId, confirmOnly) => actions.takeOverMission(mission.id, { requestId, confirmOnly }));
  const resumeFailure = resumed.resume.failure;
  useEffect(() => {
    if (resumeFailure === null) return;
    useShellStore.getState().noteDiagnostic(`mission_take: resume failed for ${mission.id}, starting a new agent`);
    startFresh();
    // Only a new refusal of the resume starts the new agent.
  }, [resumeFailure]);
  const press = () => {
    if (busy) return;
    if (session && !resumeTried.current) {
      resumeTried.current = true;
      resumed.start((requestId, confirmOnly) => actions.startAgent({
        target: { checkoutPath: mission.checkout_path },
        ...(mission.device_id !== node ? { deviceId: mission.device_id } : {}),
        provider: session.provider,
        resumeSessionId: session.sessionId,
        requestId,
        confirmOnly,
      }));
      return;
    }
    startFresh();
  };
  const failed = fresh.resume.failure !== null;
  return <Button size="sm" variant={failed ? "destructive" : "default"} disabled={busy} onClick={press} data-mission-takeover={failed ? "failed" : "true"}>
    {t(failed ? "mission.takeOverFailed" : "mission.takeOver")}
  </Button>;
}
