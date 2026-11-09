@echo off
rem Stand-in for an operator's gateway wrapper around claude: it names the
rem gateway's models, marks that it ran, and hands its arguments on.
set ANTHROPIC_DEFAULT_OPUS_MODEL=glm-4.6
set ANTHROPIC_DEFAULT_SONNET_MODEL=glm-4.6
set ANTHROPIC_CUSTOM_MODEL_OPTION=deepseek-reasoner
set FAKE_WRAPPED=1
python "%~dp0fake-claude.py" %*
