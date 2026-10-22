// Hide's own background AI inside the Usage history: the feature names the full
// table uses for its rows and the lower-bound mark unknown calls put on an amount.

/** The closed feature groups the core counts under; one it does not know is Other. */
export const HIDE_AI_FEATURES = {
  labels: "usage.hideAi.feature.labels",
  memory: "usage.hideAi.feature.memory",
  factory: "usage.hideAi.feature.factory",
  issue: "usage.hideAi.feature.issue",
  other: "usage.hideAi.feature.other",
} as const;

/** `≥ 1.2M *` for a known amount that unknown calls make a lower bound, the plain number otherwise. */
export function BoundedTokens({ text, bounded }: { text: string; bounded: boolean }) {
  return <>{bounded ? `≥ ${text}` : text}{bounded ? <span className="text-muted-foreground"> *</span> : null}</>;
}
