#!/bin/sh
# Stand-in for an operator's gateway wrapper around `claude`: it names the
# gateway's models, marks that it ran, and hands its arguments on.
export ANTHROPIC_DEFAULT_OPUS_MODEL=glm-4.6
export ANTHROPIC_DEFAULT_SONNET_MODEL=glm-4.6
export ANTHROPIC_CUSTOM_MODEL_OPTION=deepseek-reasoner
export FAKE_WRAPPED=1
exec "$(dirname "$0")/fake-claude.py" "$@"
