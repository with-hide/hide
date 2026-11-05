//! The stand-in for the helper a host script starts (OpenCode's plugin, Pi's
//! and omp's extension): `hide-agent-hooks <agent> <operation>
//! --state-dir-b64 <route>`. It logs the call and answers from the answers
//! file the test writes (`tests/stand-in-answers.mjs`).
//!
//! The host script kills a helper that outlives its budget (1.5 s for a tool
//! result's letters, 1.85 s for a prompt), so the stand-in costs what the
//! helper costs: POSIX sh with builtins only, and `sleep` for an answer the
//! test holds back. One that started a JavaScript runtime per call spent
//! those budgets on a loaded machine before it logged the call
//! (with-hide/hide-private#517).

/// The stand-in's text. `$1`, `$2` and the route are the helper's arguments;
/// the files are the ones the environment variables `log` and `answers`
/// name, which the host hands on.
#[cfg(unix)]
pub(crate) fn body(log: &str, answers: &str) -> String {
    const BODY: &str = r#"#!/bin/sh
if [ "$3 $4" != "--state-dir-b64 L2NhcHR1cmVkLXN0YXRl" ]; then
  echo "the route did not reach the helper: $*" >&2
  exit 97
fi
agent=$1
operation=$2
# The host writes its input as one line of JSON and closes the pipe.
IFS= read -r input || [ -n "$input" ] || exit 96
if [ "${HIDE_PANE_ID+set}" = set ]; then
  case $HIDE_PANE_ID in *[\"\\]*) exit 95 ;; esac
  env="{\"HIDE_PANE_ID\":\"$HIDE_PANE_ID\"}"
else
  env='{}'
fi
# The answer is taken before the call is logged: the test may replace the
# answers as soon as it sees the call. One line per operation: its name, a
# delay in seconds, an exit status and the output, by tabs, `-` for none.
delay=- status=- output='{}'
if [ -f "$@ANSWERS@" ]; then
  while IFS='	' read -r name wait code text; do
    if [ "$name" = "$operation" ]; then
      delay=$wait status=$code output=$text
      break
    fi
  done < "$@ANSWERS@"
fi
printf '{"agent":"%s","operation":"%s","input":%s,"env":%s}\n' "$agent" "$operation" "$input" "$env" >> "$@LOG@"
[ "$delay" = - ] || sleep "$delay"
[ "$status" = - ] || exit "$status"
printf '%s' "$output"
"#;
    BODY.replace("@LOG@", log).replace("@ANSWERS@", answers)
}
