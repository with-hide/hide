# Hide's bash startup for an interactive shell that is not a login shell,
# which is how Herdr starts a pane's shell on Linux. Hide starts bash with
# --init-file naming this file, which bash reads in place of ~/.bashrc
# (after the system's bashrc where bash was built to read one), so it reads
# ~/.bashrc here. Then the shell tells Herdr it has finished starting, which
# is when Hide may type.
if [ -r "$HOME/.bashrc" ]; then
  . "$HOME/.bashrc"
fi
# An agent program Hide starts runs as itself, past an alias or a function
# of its name, when the PATH finds it (a transition path, until Hide types
# the start itself); the mark names those whose alias or function went.
__hide_removed=
for __hide_program in ${HIDE_SHELL_PROGRAMS-}; do
  if type -P -- "$__hide_program" >/dev/null 2>&1 \
    && { alias -- "$__hide_program" >/dev/null 2>&1 || declare -F -- "$__hide_program" >/dev/null; }; then
    unalias -- "$__hide_program" 2>/dev/null
    unset -f -- "$__hide_program"
    __hide_removed=${__hide_removed:+$__hide_removed,}$__hide_program
  fi
done
if [ -n "${HERDR_BIN_PATH-}" ] && [ -n "${HERDR_PANE_ID-}" ]; then
  "$HERDR_BIN_PATH" pane report-metadata "$HERDR_PANE_ID" --source hide:shell --token hide_prompt=1 ${__hide_removed:+--token "hide_unshadowed=$__hide_removed"} >/dev/null 2>&1
fi
unset __hide_program __hide_removed HIDE_SHELL_PROGRAMS
