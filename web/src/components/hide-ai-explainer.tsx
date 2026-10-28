// Hide AI saying what it does where a tab of its tool has nothing to show yet:
// the reading owl, the tab's state as its title, and the work in a few steps,
// each with its mark, so the empty space explains the tool instead of a bare line.
import type { LucideIcon } from "lucide-react";
import type { ReactNode } from "react";
import { BrandOwl } from "./brand-owl";

export type HideAiStep = { icon: LucideIcon; text: string };

export function HideAiExplainer({ title, steps = [], children, ...data }: { title: ReactNode; steps?: readonly HideAiStep[]; children?: ReactNode } & Record<`data-${string}`, string>) {
  return (
    <div className="flex min-w-0 items-start gap-md py-md" data-hide-ai-explainer="true" {...data}>
      <BrandOwl pose="ai" className="size-(--size-owl-intro)" />
      <div className="flex min-w-0 flex-1 flex-col gap-sm pt-xs">
        <span className="break-keep text-body font-medium text-foreground" data-hide-ai-explainer-title="true">{title}</span>
        {steps.length > 0 ? (
          <ol className="flex flex-col gap-xs">
            {steps.map(({ icon: Icon, text }) => (
              <li key={text} className="flex min-w-0 items-start gap-xs text-caption text-subtle-foreground">
                <Icon aria-hidden="true" className="mt-xxs size-(--size-icon-sm) shrink-0 text-muted-foreground" />
                <span className="min-w-0 break-keep">{text}</span>
              </li>
            ))}
          </ol>
        ) : null}
        {children}
      </div>
    </div>
  );
}
