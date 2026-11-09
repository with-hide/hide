@echo off
rem Stand-in for an operator's gateway wrapper around codex: it names the
rem gateway's model, marks that it ran, and hands its arguments on.
set FAKE_GATEWAY_MODEL=deepseek-chat
set FAKE_WRAPPED=1
python "%~dp0fake-app-server.py" %*
