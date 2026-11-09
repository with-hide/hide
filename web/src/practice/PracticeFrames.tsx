// A guide's moves, drawn one after another: each frame is a small picture of the
// shell (`PracticePreview.tsx`) with a few words under it, and the one the
// story is at plays while the others hold still, so a flow reads in about ten
// seconds with nothing to press.

import { useEffect, useState } from "react";
import { useInterfaceTranslation } from "../i18n/client";
import type { PracticeGuide } from "./guides";
import { PracticePreview } from "./PracticePreview";

/** How long each frame holds the story before the next one plays. */
const FRAME_MS = 2600;

export function PracticeFrames({ guide }: { guide: PracticeGuide }) {
  const { t } = useInterfaceTranslation();
  const [at, setAt] = useState(0);
  useEffect(() => {
    const timer = window.setInterval(() => setAt((value) => (value + 1) % guide.frames.length), FRAME_MS);
    return () => window.clearInterval(timer);
  }, [guide]);
  return (
    <ol className="pf" data-practice-frames={guide.frames.length}>
      {guide.frames.map((kind, index) => (
        <li key={index} className={index === at ? "on" : undefined} data-frame-active={index === at}>
          <PracticePreview kind={kind} playing={index === at} />
          <span><b>{index + 1}</b>{t(`practice.${guide.id}.frame${index + 1}` as never)}</span>
        </li>
      ))}
    </ol>
  );
}
