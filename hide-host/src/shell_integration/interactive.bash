# Hide's bash startup for an interactive shell that is not a login shell. Hide starts bash with
# --init-file naming this file, which bash reads in place of ~/.bashrc
# (after the system's bashrc where bash was built to read one), so it reads
# ~/.bashrc here. Then the shell marks its first prompt on the screen
# (OSC 133;A) for the pane's holder, which is when Hide may type, and says
# its folder there (OSC 7) at each prompt, with builtins and no process.
if [ -r "$HOME/.bashrc" ]; then
  . "$HOME/.bashrc"
fi
# The folder, said on the screen (OSC 7) at each prompt; one with a control
# character in its name is not said, since it would end the sequence early.
# The last command's status is left as it was for the user's prompt.
__hide_cwd() {
  local __hide_status=$?
  case $PWD in
    *[[:cntrl:]]*) ;;
    *) builtin printf '\033]7;file://%s\007' "${PWD//\%/%25}" ;;
  esac
  return "$__hide_status"
}
# Bash 5.1 runs each element of a PROMPT_COMMAND array, which the user's
# may be, so the folder is said by an element of its own after theirs;
# an older bash runs one string.
if ((BASH_VERSINFO[0] > 5 || (BASH_VERSINFO[0] == 5 && BASH_VERSINFO[1] >= 1))); then
  PROMPT_COMMAND+=(__hide_cwd)
else
  PROMPT_COMMAND="${PROMPT_COMMAND:+$PROMPT_COMMAND
}__hide_cwd"
fi
builtin printf '\033]133;A\007'
