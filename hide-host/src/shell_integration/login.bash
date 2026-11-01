# Hide's bash startup for a login shell, which is how Herdr starts a pane's
# shell on macOS. Hide starts bash with --init-file naming this file, which
# bash reads in place of its own startup files, so it reads them here in a
# login shell's order: /etc/profile, then the first of ~/.bash_profile,
# ~/.bash_login and ~/.profile. A shell reading an init file is not a login
# shell, so `shopt login_shell` is off and ~/.bash_logout does not run, as
# in VS Code's terminal. Then the shell tells Herdr it has finished
# starting, which is when Hide may type.
if [ -r /etc/profile ]; then
  . /etc/profile
fi
if [ -r "$HOME/.bash_profile" ]; then
  . "$HOME/.bash_profile"
elif [ -r "$HOME/.bash_login" ]; then
  . "$HOME/.bash_login"
elif [ -r "$HOME/.profile" ]; then
  . "$HOME/.profile"
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
