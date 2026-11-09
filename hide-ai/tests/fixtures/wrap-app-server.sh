#!/bin/sh
# Stand-in for an operator's gateway wrapper around `codex`: it names the
# gateway's model, marks that it ran, and hands its arguments on.
export FAKE_GATEWAY_MODEL=deepseek-chat
export FAKE_WRAPPED=1
exec "$(dirname "$0")/fake-app-server.py" "$@"
