# Hide's bash startup for an interactive shell that is not a login shell,
# which is how Herdr starts a pane's shell on Linux. Hide starts bash with
# --init-file naming this file, which bash reads in place of ~/.bashrc
# (after the system's bashrc where bash was built to read one), so it reads
# ~/.bashrc here. Then the shell tells Herdr it has finished starting, which
# is when Hide may type; outside a Herdr pane it marks its prompt on the
# screen (OSC 133;A) for the pane's holder, and says its folder there (OSC 7)
# at each prompt, with builtins and no process.
if [ -r "$HOME/.bashrc" ]; then
  . "$HOME/.bashrc"
fi
if [ -n "${HERDR_BIN_PATH-}" ] && [ -n "${HERDR_PANE_ID-}" ]; then
  "$HERDR_BIN_PATH" pane report-metadata "$HERDR_PANE_ID" --source hide:shell --token hide_prompt=1 >/dev/null 2>&1
else
  # The folder, said on the screen (OSC 7) at each prompt, which Herdr reads
  # from the process instead; one with a control character in its name is
  # not said, since it would end the sequence early. The last command's
  # status is left as it was for the user's prompt.
  __hide_cwd() {
    local __hide_status=$?
    case $PWD in
      *[[:cntrl:]]*) ;;
      *) builtin printf '\033]7;file://%s\007' "${PWD//\%/%25}" ;;
    esac
    return "$__hide_status"
  }
  PROMPT_COMMAND="${PROMPT_COMMAND:+$PROMPT_COMMAND
}__hide_cwd"
  builtin printf '\033]133;A\007'
fi
