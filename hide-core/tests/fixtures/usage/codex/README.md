# Codex account window samples

`weekly-only.json` is a read-only capture of the usage response on 2026-10-12 UTC.
Only numeric window fields remain; account identifiers, plan, credits and all other response fields were removed before this file was written.
The primary window reports 604800 seconds and the secondary window is null.
This is live evidence for a plan without a five-hour window, not evidence for a five-hour plan.

`five-hour-and-weekly.json` retains only the window facts in the public response example from [CodexBar OAuth documentation](https://github.com/steipete/CodexBar/blob/f66d18fc6fbb85c7df056bdeef56992de0619038/docs/codex-oauth.md#api-endpoint).
Its five-hour window is 18000 seconds at 15%, resetting at 1735401600; its week is 604800 seconds at 5%, resetting at 1735920000.
It is a documented response example, not a capture from this machine or a claim that a five-hour plan was tested live.
No implementation code was copied or executed.
