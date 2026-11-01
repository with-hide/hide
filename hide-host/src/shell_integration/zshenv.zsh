# Hide's shell integration for zsh. Hide starts a pane's zsh with ZDOTDIR
# naming this folder, so zsh reads its startup files from here. Each file here
# reads the user's own file of the same name from where zsh would have, with
# ZDOTDIR as the user has it, and the last one of the run gives ZDOTDIR back
# for good. At the first prompt the shell tells Herdr it has finished
# starting, which is when Hide may type.
#
# What the user had before Hide's value replaced it comes in
# HIDE_SHELL_ZDOTDIR, absent when the user had none. A user's file is read at
# the top level, never inside a function, so what it declares stays global.
typeset -g __hide_zdotdir=$ZDOTDIR
typeset -g __hide_user_set=${+HIDE_SHELL_ZDOTDIR} __hide_user_zdotdir=${HIDE_SHELL_ZDOTDIR-}
typeset -g __hide_user_type=scalar-export
builtin unset HIDE_SHELL_ZDOTDIR

# ZDOTDIR as the user has it, unset included, for their next file.
__hide_give() {
  if (( __hide_user_set )); then
    ZDOTDIR=$__hide_user_zdotdir
    if [[ $__hide_user_type == *export* ]]; then
      export ZDOTDIR
    else
      typeset -g +x ZDOTDIR
    fi
  else
    builtin unset ZDOTDIR
  fi
}

# Keeps ZDOTDIR as the user's file left it, and points zsh here again for
# the next startup file.
__hide_take() {
  typeset -g __hide_user_set=${+ZDOTDIR} __hide_user_zdotdir=${ZDOTDIR-}
  typeset -g __hide_user_type=${(t)ZDOTDIR}
  export ZDOTDIR=$__hide_zdotdir
}

# Gives ZDOTDIR back for good once no startup file is left to read, and
# marks the first prompt of an interactive shell.
__hide_restore() {
  __hide_give
  builtin unset __hide_zdotdir __hide_user_set __hide_user_zdotdir __hide_user_type
  builtin unfunction __hide_give __hide_take __hide_restore
  if [[ -o interactive ]]; then
    builtin autoload -Uz add-zsh-hook && add-zsh-hook precmd __hide_first_prompt
  else
    builtin unfunction __hide_first_prompt
  fi
}

# Runs before the first prompt, after every startup file and anything they
# asked: Herdr keeps the mark on the pane, where Hide reads it.
__hide_first_prompt() {
  add-zsh-hook -d precmd __hide_first_prompt
  builtin unfunction __hide_first_prompt
  if [[ -n ${HERDR_BIN_PATH-} && -n ${HERDR_PANE_ID-} ]]; then
    "$HERDR_BIN_PATH" pane report-metadata "$HERDR_PANE_ID" --source hide:shell --token hide_prompt=1 >/dev/null 2>&1
  fi
}

__hide_give
[[ -r ${ZDOTDIR:-$HOME}/.zshenv ]] && builtin source "${ZDOTDIR:-$HOME}/.zshenv"
__hide_take
# zsh reads no other startup file for a shell that is neither interactive
# nor a login shell, or once the user's file turned RCS off.
if [[ ! -o rcs ]] || [[ ! -o interactive && ! -o login ]]; then
  __hide_restore
fi
