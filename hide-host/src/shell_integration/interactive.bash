# Hide's bash startup for an interactive shell that is not a login shell,
# which is how Herdr starts a pane's shell on Linux. Hide starts bash with
# --init-file naming this file, which bash reads in place of ~/.bashrc
# (after the system's bashrc where bash was built to read one), so it reads
# ~/.bashrc here. Then the shell tells Herdr it has finished starting, which
# is when Hide may type.
if [ -r "$HOME/.bashrc" ]; then
  . "$HOME/.bashrc"
fi
if [ -n "${HERDR_BIN_PATH-}" ] && [ -n "${HERDR_PANE_ID-}" ]; then
  "$HERDR_BIN_PATH" pane report-metadata "$HERDR_PANE_ID" --source hide:shell --token hide_prompt=1 >/dev/null 2>&1
fi
