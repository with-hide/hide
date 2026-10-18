# agy 1.4.0 error reports

The `AGY_ERROR:` line agy 1.4.0 writes to stderr when a print-mode turn ends with an error, copied unchanged from the nightly `agy signed in` job's log.

- `quota-spent.txt`: the CI key spent the day's 20 requests on `gemini-3.8-flash` (run 38958746826, macOS, 2026-10-16); its `google.rpc.QuotaFailure` names `quotaValue:20`.
- `quota-zero.txt`: the key has no quota for `gemini-3.1-pro`, agy's API-key default (run 38885907433, macOS, 2026-10-16); its violations name no `quotaValue`, which the API leaves out at 0.
