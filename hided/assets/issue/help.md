# hide issue

This is the guide for reporting a bug in Hide itself.
It only prints text: it files nothing, reads nothing from GitHub and needs no running Hide.
You file the report yourself with `gh`, after the operator agrees.

## When to report

Report only a bug in Hide that you or the operator actually saw: a crash, a `hide` command that fails or answers wrongly for no reason of yours, or a pane, tab or screen that behaves wrongly.
A question, a feature wish, a bug in the project's own code, or something you only suspect is not a report.
If this session does not run in a pane Hide manages, this guide does not apply.

## Steps

1. Search first: `gh issue list --repo with-hide/hide --state open --search "<the symptom>"`.
   When one matches, add what you saw to it with `gh issue comment` and stop.
2. Collect the raw facts, not a diagnosis.
   The more exact the error, the better:
   - the exact command you ran and its whole output, a `{"ok":false,...}` line with its `reason` and `next_action` included;
   - `hide --version`, the operating system and its version, and for an agent involved its name and `--version`;
   - `hide workspace info` and `hide ps --here`;
   - the lines around the failure, with their timestamps, from `Logs/core.jsonl` in Hide's state folder (`$HIDE_STATE_DIR`, else the `state` folder under `~/.hide`) and, for the desktop app, from its `desktop.log` (`Copy diagnostics` names it);
   - the shortest steps that show it, and whether it happens every time, sometimes or once.
3. Write the report in the template below.
   Keep it under 6,000 characters, because the repository closes a longer one automatically.
   Paste raw lines and cut a log to the lines around the failure.
4. Remove what is private before it leaves the machine: the home folder name in paths, tokens, keys and the operator's file contents.
   Do not add a root cause, a proposed fix or a patch.
5. Show the operator the title and body in one message and wait for their yes, because a report is public and cannot be fully taken back.
   Then run `gh issue create --repo with-hide/hide --title "<plain symptom>" --body-file <file>` and give them the issue's link.
   When `gh` is missing or not signed in, give the operator the text and `https://github.com/with-hide/hide/issues/new?template=bug.yml` instead.

## Template

The headings are the labels of the repository's bug form (`.github/ISSUE_TEMPLATE/bug.yml`).

```
### What happened
What was done, what was expected, what came instead. What was seen, never a guessed cause.

### Steps to reproduce
1. ...

### How often
Every time | Sometimes | Once so far

### Where
Desktop app | hide in a browser tab | hide CLI | Phone | A connected device (SSH) | Install or update | Other

### hide version
The `hide --version` line.

### Operating system
macOS | Windows | Linux

### OS version
For example macOS 15.2, Windows 11 24H2, Ubuntu 24.04.

### Agent involved
Not agent-related, or the agent and its version.

### Diagnostics
The raw command output and log lines, in a fenced text block.
```
