// The pointer a scene moves to what it presses, and the ring of the press.
import { MousePointer2Icon } from "lucide-react";

/** At `left`/`top` (percent of the scene), shown while `shown`, ringing once `pressed`. */
export function ScenePointer({ left, top, shown, pressed }: { left: number; top: number; shown: boolean; pressed: boolean }) {
  return (
    <span className="wns-glide pointer-events-none absolute z-10 flex text-foreground" style={{ left: `${left}%`, top: `${top}%`, opacity: shown ? 1 : 0 }}>
      {pressed ? <span className="wns-press absolute size-(--size-icon) rounded-full border border-primary" /> : null}
      <MousePointer2Icon className="size-(--size-icon) fill-foreground" />
    </span>
  );
}
