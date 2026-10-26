import type { SVGProps } from "react";

/**
 * The Hide AI tool's line icon (PRD hide-ai-suggestions D-25, icon A): Hide's
 * owl face, round with two large eyes, and a small sparkle at its top right.
 * One colour, `currentColor`, drawn on the same 24 grid and stroke as the
 * lucide icons beside it on the rail, so it takes their size classes.
 */
export function HideAiIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round" data-hide-ai-icon="true" {...props}>
      <path d="M3 9.3C3 5.7 5.85 3.9 9.6 3.9s6.6 1.8 6.6 5.4v4.95c0 3.6-3 6-6.6 6S3 17.85 3 14.25z" />
      <circle cx="7.05" cy="11.1" r="2.4" />
      <circle cx="12.15" cy="11.1" r="2.4" />
      <path d="M19.5 2.1l.83 2.17 2.17.83-2.17.83-.83 2.17-.83-2.17-2.17-.83 2.17-.83z" fill="currentColor" stroke="none" />
    </svg>
  );
}
