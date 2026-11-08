/*
 * `claude` in a release-state container: runs the stand-in agent
 * (`/opt/agent.py`) as its child, so the pane's foreground program is named
 * `claude` the way the released engine detects Claude Code, while the agent
 * itself is a short Python program. Signals that end a pane's program reach
 * the child too.
 */
#include <signal.h>
#include <stdlib.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

static pid_t child = 0;

static void forward(int signal_number) {
  if (child > 0) kill(child, signal_number);
}

int main(int argc, char **argv) {
  char **args = calloc((size_t)argc + 3, sizeof *args);
  if (!args) return 1;
  args[0] = "python3";
  args[1] = "/opt/agent.py";
  for (int index = 1; index < argc; index++) args[index + 1] = argv[index];
  child = fork();
  if (child < 0) return 1;
  if (child == 0) {
    execvp(args[0], args);
    _exit(127);
  }
  signal(SIGHUP, forward);
  signal(SIGTERM, forward);
  signal(SIGINT, forward);
  int status = 0;
  while (waitpid(child, &status, 0) < 0) {
  }
  if (WIFSIGNALED(status)) return 128 + WTERMSIG(status);
  return WEXITSTATUS(status);
}
