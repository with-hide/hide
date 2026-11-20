// One JSON document over the network, bounded in time and in size: the
// updater's release list and the releases file withhide.dev serves both come
// through here. The size is counted as the body arrives, so an answer with
// no length, or a wrong one, stops at the cap instead of being read whole.

export type FetchImpl = (url: string, init: RequestInit) => Promise<Response>;

export async function fetchBoundedJson(
  fetchImpl: FetchImpl,
  url: string,
  { headers, cap, timeoutMs, what }: { headers: Record<string, string>; cap: number; timeoutMs: number; what: string },
): Promise<unknown> {
  const response = await fetchImpl(url, { headers, signal: AbortSignal.timeout(timeoutMs) });
  const over = `${what} exceeds ${Math.round(cap / (1024 * 1024))} MiB`;
  if (!response.ok) {
    await response.body?.cancel();
    throw new Error(`${what} answered HTTP ${response.status}`);
  }
  if (Number(response.headers.get("content-length") ?? "0") > cap) {
    await response.body?.cancel();
    throw new Error(over);
  }
  if (!response.body) throw new Error(`${what} had no body`);
  const reader = response.body.getReader();
  const chunks: Uint8Array[] = [];
  let length = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    length += value.byteLength;
    if (length > cap) {
      await reader.cancel();
      throw new Error(over);
    }
    chunks.push(value);
  }
  const body = new Uint8Array(length);
  let at = 0;
  for (const chunk of chunks) {
    body.set(chunk, at);
    at += chunk.byteLength;
  }
  return JSON.parse(new TextDecoder().decode(body));
}
