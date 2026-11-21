/**
 * Whether a URL's `hostname` (an IPv6 literal in brackets, as `URL` spells
 * it) names the machine that loads it: `localhost` with or without its
 * trailing dot, `0.0.0.0`, `127/8`, `[::1]` and IPv4-mapped or -compatible
 * IPv6 loopback. One decision for every caller: the desktop host's route and
 * partition for a device's page (`browserSync.ts`), and the browser display's
 * Open in default browser (`defaultBrowserAddress`), kept equal to the
 * loopback hided routes to a device (`hided/src/browser_routes.rs`).
 */
export function isLoopbackHost(hostname: string): boolean {
  const host = hostname.toLowerCase().replace(/\.$/, "");
  const mapped = /^\[::(?:(?:ffff:)?([0-9a-f]{1,4}):([0-9a-f]{1,4}))\]$/.exec(host);
  const embeddedLoopback = mapped ? Number.parseInt(mapped[1]!, 16) >> 8 === 127 : false;
  return host === "localhost" || host === "0.0.0.0" || host === "[::1]" || /^127\./.test(host) || embeddedLoopback;
}
