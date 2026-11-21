// A request the host sends and nobody awaits. Whatever goes wrong with it ends in `failed`,
// the caller's log line; nothing reaches the process as an unhandled rejection.

/**
 * Sends one request and hands its response to `answered`; any failure of either ends in
 * `failed`. `send` runs inside the guard because `fetch` can throw instead of returning a
 * rejected promise: its first call in a process loads the HTTP client, and a load that
 * fails throws from the call itself, where a `.catch` chained on its result never sees it.
 * It came out of `sendWake` into the discovery that attached the window, whose promise
 * nobody awaited, as an unhandled rejection that failed an otherwise passing test.
 */
export function inBackground(send: () => Promise<Response>, answered: (response: Response) => void | Promise<void>, failed: (error: unknown) => void): void {
  void (async () => {
    try {
      await answered(await send());
    } catch (error) {
      failed(error);
    }
  })();
}
