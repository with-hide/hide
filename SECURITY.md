# Security policy

## Reporting a vulnerability

Report it privately through GitHub's private vulnerability reporting for this repository: open the **Security** tab and choose **Report a vulnerability**.
Do not open a public issue.

You will get an acknowledgement within a week.
Fixes ship as a normal release; the advisory is published once the release is out.

## What is in scope

- The hide app: the Electron desktop host under `desktop/`, the web shell under `web/`, the daemon under `hided/`, and the Rust core under `hide-core/`.
- The build and release scripts under `scripts/` and the workflows under `.github/workflows/`.
- The pane holder under `hide-holder/` and the terminal parser under `hide-vt/`, which run every pane on macOS, Linux and Windows.

## What hide does not do

Hide delegates provider sign-in to the installed agent CLIs and remote authentication to the operator's existing SSH configuration.
It does not ask the operator to paste a provider API key or an SSH password into the app.
For Codex usage status, hide reads the installed CLI's existing `auth.json` access token and account ID and sends a bearer-authenticated request to the provider's usage endpoint.
Hide does generate and store its own authentication material, including the daemon's local client token, phone pairing credentials and Project Memory receipt keys.
Treat local state, app databases, pairing information and run artifacts as private; do not attach them unredacted to a public issue or pull request.
A report that exposes credentials, bypasses an authentication boundary, or discloses private local content is in scope.

Project Memory and background labels can send bounded session content through the user's authenticated provider CLI.
The current disclosure, local redaction and provider boundary are documented in [AI providers](docs/AI_PROVIDERS.md); do not assume that local storage makes all analysis offline.

## Supported versions

Only the latest release receives fixes.
