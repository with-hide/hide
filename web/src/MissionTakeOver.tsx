import { useEffect, useRef } from "react";
import type { Actions } from "./actions";
import { Button } from "./components/ui/button";
import { useInterfaceTranslation } from "./i18n/client";
import { conductorResume, missionTakeOverCheckout } from "./missions";
import { localDeviceId, type Mission } from "./snapshot";
import { useShellStore } from "./store";
import { useSessionResume } from "./useSessionResume";

/**
 * 이어받기 (PRD mission-tool D-11, B13-B15): the gone Conductor's own
 * conversation is resumed first, so the reasons behind its plan come back
 * with it, and the core moves the mission to the pane it resumes in. A
 * A removed worktree's conversation resumes at the project's primary
 * checkout. An unanswered launch is confirmed on its original destination.
 * A Conductor with no conversation, or a failed resume in its original
 * primary checkout, can start a new agent told to take the mission.
 * While a start is on its way the button says 이어받는 중… and takes no press,
 * nor while the mission's device does not answer (`disabled`, PRD
 * mission-screen B9).
 */
export function MissionTakeOver({ mission, actions, disabled = false, className }: { mission: Mission; actions: Actions; disabled?: boolean; className?: string }) {
  const { t } = useInterfaceTranslation();
  const node = useShellStore((state) => localDeviceId(state.rest));
  const resumed = useSessionResume();
  const fresh = useSessionResume();
  const resumePath = useRef<string | null>(null);
  const session = conductorResume(mission);
  const busy = (resumed.resume.request !== null && !resumed.resume.uncertain) || (fresh.resume.request !== null && !fresh.resume.uncertain);
  const checkoutPath = () => missionTakeOverCheckout(mission, useShellStore.getState().rest);
  const startFresh = () => {
    const path = checkoutPath();
    fresh.start((requestId, confirmOnly) => actions.takeOverMission(mission.id, path, { requestId, confirmOnly }));
  };
  const resumeFailure = resumed.resume.failure;
  const resumeUncertain = resumed.resume.uncertain;
  const preserveConversation = mission.checkout_path !== mission.project
    || (resumePath.current !== null && resumePath.current !== mission.checkout_path);
  const startResume = (path: string) => {
    if (!session) return;
    const changedDestination = resumePath.current !== null && resumePath.current !== path;
    resumePath.current = path;
    resumed.start((requestId, confirmOnly) => actions.startAgent({
      target: { checkoutPath: path },
      ...(mission.device_id !== node ? { deviceId: mission.device_id } : {}),
      provider: session.provider,
      resumeSessionId: session.sessionId,
      requestId,
      confirmOnly,
    }), changedDestination);
  };
  useEffect(() => {
    if (resumeFailure === null || resumeUncertain) return;
    if (preserveConversation) {
      const path = checkoutPath();
      if (path !== resumePath.current) startResume(path);
      return;
    }
    useShellStore.getState().noteDiagnostic(`mission_take: resume failed for ${mission.id}, starting a new agent`);
    startFresh();
    // Only a newly settled failure can change the destination or start a new agent.
  }, [resumeFailure, resumeUncertain]);
  const press = () => {
    if (busy) return;
    if (session && (resumePath.current === null || resumeUncertain || (preserveConversation && resumeFailure !== null))) {
      startResume(resumeUncertain && resumePath.current ? resumePath.current : checkoutPath());
      return;
    }
    startFresh();
  };
  const failed = fresh.resume.failure !== null || fresh.resume.uncertain || resumeUncertain || (preserveConversation && resumeFailure !== null);
  return <Button size="sm" variant={failed ? "destructive" : "secondary"} className={className} disabled={busy || disabled} onClick={press} data-mission-takeover={failed ? "failed" : "true"}>
    {t(busy ? "mission.takingOver" : failed ? "mission.takeOverFailed" : "mission.takeOver")}
  </Button>;
}
