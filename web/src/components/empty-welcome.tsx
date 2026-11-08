// The one next step of a screen that has nothing on it yet: the owl, what to
// do in one line, and the action that does it. The Home with no project and
// the Overview's Work with no session draw it alike.
import type { ReactNode } from "react";
import { BrandOwl, type OwlPose } from "./brand-owl";

/** A faint lime light behind the owl; decoration that is neither read nor pressed. */
const GLOW = { background: "radial-gradient(circle, color-mix(in srgb, var(--primary) 12%, transparent), transparent 65%)" };

export function EmptyWelcome({ pose, title, detail, children, owl = true, ...data }: {
  pose: OwlPose;
  title: string;
  /** A second line under the title, quieter than it. */
  detail?: ReactNode;
  /** The action, and under it at most one quiet way on. */
  children: ReactNode;
  /** False while another first-run surface already shows an owl. */
  owl?: boolean;
} & Record<`data-${string}`, string | undefined>) {
  return (
    <div className="flex min-h-full flex-1 flex-col items-center justify-center gap-md p-xl text-center" {...data}>
      {owl ? (
        <div className="relative flex items-center justify-center">
          <div aria-hidden="true" className="pointer-events-none absolute -inset-xxl rounded-full" style={GLOW} />
          <BrandOwl pose={pose} className="relative size-(--size-owl-empty)" />
        </div>
      ) : null}
      <h2 className="text-subhead font-semibold break-keep text-foreground">{title}</h2>
      {detail ? <p className="-mt-xs text-body break-keep text-subtle-foreground">{detail}</p> : null}
      {children}
    </div>
  );
}
