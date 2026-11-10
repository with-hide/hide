// A small mission map: the plan a Conductor made, the workers on it and the pull request one of them opened,
// drawn by the product's own `MissionMap` over an invented mission.
import { useLayoutEffect, useMemo } from "react";
import { createActions } from "../../actions";
import { MissionMap } from "../../MissionMap";
import { useShellStore } from "../../store";
import { useUiStore } from "../../ui";
import { MISSION_ID, missionFixture } from "../fixtures/mission";
import { PartFrame } from "../frame";

export default function MissionMapPart() {
  const { rest } = useMemo(missionFixture, []);
  const actions = useMemo(() => createActions(() => true), []);
  useLayoutEffect(() => {
    useShellStore.setState({ rest, agents: [], connection: "gone" });
    useUiStore.setState({ screen: { kind: "missions" } });
  }, [rest]);
  return (
    <PartFrame part="mission-map" height={640}>
      <div className="flex h-full min-h-0 min-w-0 flex-col rounded-md border border-border"><MissionMap missionId={MISSION_ID} actions={actions} /></div>
    </PartFrame>
  );
}
