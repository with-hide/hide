/**
 * Whether a URL's `hostname` (an IPv6 literal in brackets, as `URL` spells
 * it) names the machine that loads it: `localhost` with or without its
 * trailing dot, `127/8`, `[::1]` and IPv4-mapped or -compatible IPv6
 * loopback, and the unspecified addresses (`0.0.0.0`, `[::]`,
 * `[::ffff:0.0.0.0]`), which a dial connects to the dialing machine. One decision for every caller: the desktop host's route and
 * partition for a device's page (`browserSync.ts`), and the browser display's
 * Open in default browser (`defaultBrowserAddress`), kept equal to the
 * loopback hided routes to a device (`hided/src/browser_routes.rs`).
 */
export function isLoopbackHost(hostname: string): boolean {
  const host = hostname.toLowerCase().replace(/\.$/, "");
  const mapped = /^\[::(?:(?:ffff:)?([0-9a-f]{1,4}):([0-9a-f]{1,4}))\]$/.exec(host);
  const embeddedLoopback = mapped ? Number.parseInt(mapped[1]!, 16) >> 8 === 127 : false;
  const embeddedUnspecified = mapped ? Number.parseInt(mapped[1]!, 16) === 0 && Number.parseInt(mapped[2]!, 16) === 0 : false;
  return host === "localhost" || host === "0.0.0.0" || host === "[::]" || host === "[::1]" || /^127\./.test(host) || embeddedLoopback || embeddedUnspecified;
}

/**
 * Whether `hostname` (as a URL spells it, an IPv6 literal in brackets) names
 * this machine or a private network: loopback, link-local, private and
 * shared ranges, and the names a resolver answers from the local network
 * rather than the internet (`localhost`, `.local`, and a name with no dot).
 * One decision for the desktop host's local pages (`browserSync.ts`), a
 * page of another computer's screen reaching that computer's network, and
 * what that screen may hand to this computer's default browser.
 */
export function isLocalNetworkHost(hostname: string): boolean {
  const host = hostname.toLowerCase().replace(/\.$/, "");
  if (host === "" || host === "localhost" || /\.(localhost|local|localdomain|internal|lan|home\.arpa|intranet)$/.test(host)) return true;
  const v4 = /^(\d+)\.(\d+)\.(\d+)\.(\d+)$/.exec(host);
  if (v4) return privateV4(v4.slice(1).map(Number) as [number, number, number, number]);
  if (host.startsWith("[")) return privateV6(host.slice(1, -1));
  // A name with no dot is the local network's (a router, a NAS, a build host).
  return !host.includes(".");
}

function privateV4([a, b, c]: [number, number, number, number]): boolean {
  return a === 0 || a === 10 || a === 127 || a >= 224
    || (a === 100 && b >= 64 && b <= 127) || (a === 169 && b === 254) || (a === 172 && b >= 16 && b <= 31)
    || (a === 192 && b === 168) || (a === 192 && b === 0 && c === 0) || (a === 198 && (b === 18 || b === 19));
}

function privateV6(address: string): boolean {
  const groups = expandV6(address);
  if (!groups) return true;
  const [g0, g1, , , , g5, g6, g7] = groups as [number, number, number, number, number, number, number, number];
  if (groups.every((group) => group === 0) || (groups.slice(0, 7).every((group) => group === 0) && g7 === 1)) return true;
  if ((g0 & 0xffc0) === 0xfe80 || (g0 & 0xfe00) === 0xfc00 || (g0 & 0xffc0) === 0xfec0 || (g0 & 0xff00) === 0xff00) return true;
  // IPv4-mapped (::ffff:a.b.c.d) and the NAT64 prefix carry an IPv4 address in their last 32 bits.
  const embedded = groups.slice(0, 5).every((group) => group === 0) && g5 === 0xffff || (g0 === 0x64 && g1 === 0xff9b);
  return embedded && privateV4([g6 >> 8, g6 & 255, g7 >> 8, g7 & 255]);
}

/** Eight 16-bit groups of an IPv6 literal, or null when it does not parse. */
function expandV6(address: string): number[] | null {
  const [head, tail, extra] = address.split("::");
  if (extra !== undefined) return null;
  const read = (part: string) => part === "" ? [] : part.split(":").map((group) => (/^[0-9a-f]{1,4}$/.test(group) ? Number.parseInt(group, 16) : Number.NaN));
  const first = read(head ?? "");
  const last = tail === undefined ? [] : read(tail);
  const missing = 8 - first.length - last.length;
  if (tail === undefined ? missing !== 0 : missing < 1) return null;
  const groups = [...first, ...Array<number>(Math.max(0, missing)).fill(0), ...last];
  return groups.length === 8 && groups.every(Number.isInteger) ? groups : null;
}
