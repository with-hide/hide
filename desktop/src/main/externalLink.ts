/**
 * What the shell's window may hand to the operating system's default app
 * (`Host.openExternal`): a link the shell opens in a new window, a terminal
 * link's Open in default browser and a browser display's (issue 2293) alike.
 * A web address goes to the default browser and a `mailto:` to the mail app;
 * a `file:` address, a custom scheme and anything unparsable are refused, the
 * same on macOS, Windows and Linux. What the log keeps is the scheme and the
 * host, never the address, whose path and query can carry a sign-in code.
 */
export function externalLink(url: string): { url: string; protocol: string; host: string } | { refused: "unparsable" } | { refused: "protocol"; protocol: string } {
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return { refused: "unparsable" };
  }
  if (!EXTERNAL_PROTOCOLS.has(parsed.protocol)) return { refused: "protocol", protocol: parsed.protocol };
  return { url, protocol: parsed.protocol, host: parsed.host };
}

const EXTERNAL_PROTOCOLS = new Set(["http:", "https:", "mailto:"]);
