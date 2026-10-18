/** Literal matching of retained request excerpts, shared by History and the exact record turn. */
export function requestParts(text: string, query: string): { text: string; match: boolean }[] {
  const needle = query.trim();
  if (!needle) return [{ text, match: false }];
  // Escaped literal Unicode search preserves original UTF-16 indices even when case folding changes length.
  const escaped = needle.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const matches = text.matchAll(new RegExp(escaped, "giu"));
  const parts: { text: string; match: boolean }[] = [];
  let start = 0;
  for (const match of matches) {
    const index = match.index;
    if (index > start) parts.push({ text: text.slice(start, index), match: false });
    parts.push({ text: match[0], match: true });
    start = index + match[0].length;
  }
  if (start < text.length || parts.length === 0) parts.push({ text: text.slice(start), match: false });
  return parts;
}

export function HighlightedRequest({ text, query }: { text: string; query: string }) {
  return requestParts(text, query).map((part, index) => part.match ? <mark key={index} className="rounded-xs bg-accent text-accent-foreground">{part.text}</mark> : part.text);
}
