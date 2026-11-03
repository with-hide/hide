#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <errno.h>
// Constant-only diagnostics: never serialize command arguments or environment.
static const char *run_phase = "exited";
static unsigned long run_error = 0, cleanup_error = 0;
static int cleanup_confirmed = 1;
static unsigned long command_ms = 20000;
static unsigned long sequence = 0;
static long long elapsed_ms = 0;
static int failed(const char *phase, unsigned long error) {
  run_phase = phase; run_error = error; return 1;
}
static void reset_outcome(void) {
  run_phase = "exited"; run_error = cleanup_error = 0; cleanup_confirmed = 1;
}
#ifdef _WIN32
#include <windows.h>
#include <wchar.h>
#include <io.h>
#include <fcntl.h>
#include <process.h>
#define process_id _getpid
typedef int fixture_count_t;
static void initialize_runner(void) {}
static WCHAR *wide(const char *value) {
  int size = MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, value, -1, NULL, 0);
  WCHAR *result = size ? malloc((size_t)size * sizeof(WCHAR)) : NULL;
  if (!result || !MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, value, -1, result, size)) exit(1);
  return result;
}
static FILE *file_open(const char *value) {
  WCHAR *filename = wide(value);
  FILE *result = _wfopen(filename, L"wb");
  free(filename);
  return result;
}
static void raw_terminal(void) {
  HANDLE input = GetStdHandle(STD_INPUT_HANDLE);
  DWORD mode;
  if (!GetConsoleMode(input, &mode)) { fputs("fixture console input unavailable\n", stderr); exit(1); }
  mode &= ~(ENABLE_LINE_INPUT | ENABLE_ECHO_INPUT);
  mode |= ENABLE_VIRTUAL_TERMINAL_INPUT;
  if (!SetConsoleMode(input, mode) || !SetConsoleCP(CP_UTF8) || !SetConsoleOutputCP(CP_UTF8)) exit(1);
}
static fixture_count_t terminal_read(char *bytes, size_t size) {
  static WCHAR pending = 0;
  WCHAR input[4096];
  for (;;) {
    DWORD prefix = pending ? 1 : 0, count;
    input[0] = pending;
    if (!ReadConsoleW(GetStdHandle(STD_INPUT_HANDLE), input + prefix, 4095 - prefix, &count, NULL)) return -1;
    if (!count) return pending ? -1 : 0;
    count += prefix;
    pending = input[count - 1] >= 0xd800 && input[count - 1] <= 0xdbff ? input[--count] : 0;
    if (count) {
      int converted = WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, input, (int)count, bytes, (int)size, NULL, NULL);
      return converted ? converted : -1;
    }
  }
}
static fixture_count_t terminal_write(const char *bytes, size_t size) {
  DWORD count;
  return WriteFile(GetStdHandle(STD_OUTPUT_HANDLE), bytes, (DWORD)size, &count, NULL) ? (fixture_count_t)count : -1;
}
// Follow the same kernel boundary as hide-platform's Windows job owner:
// ActiveProcesses reaches zero before process objects finish releasing handles.
#define MAX_JOB_PROCESSES 512
typedef struct {
  HANDLE port, job, held[MAX_JOB_PROCESSES];
  DWORD count, announced;
} job_members;
static int member_error(DWORD error) {
  cleanup_confirmed = 0; cleanup_error = error; return 0;
}
static int follow_members(job_members *members, DWORD *active) {
  for (DWORD i = 0; i < members->count;) {
    DWORD waited = WaitForSingleObject(members->held[i], 0);
    if (waited == WAIT_TIMEOUT) { i++; continue; }
    if (waited != WAIT_OBJECT_0) return member_error(GetLastError());
    CloseHandle(members->held[i]);
    members->held[i] = members->held[--members->count];
  }
  // Bound one poll even if children are being created continuously.
  for (int n = 0; n < 64; n++) {
    DWORD message;
    ULONG_PTR key;
    LPOVERLAPPED value;
    if (!GetQueuedCompletionStatus(members->port, &message, &key, &value, 0)) {
      DWORD error = GetLastError();
      if (error == WAIT_TIMEOUT) break;
      return member_error(error);
    }
    if (message != JOB_OBJECT_MSG_NEW_PROCESS) continue;
    if (++members->announced > MAX_JOB_PROCESSES) return member_error(ERROR_TOO_MANY_OPEN_FILES);
    HANDLE process = OpenProcess(SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION, FALSE, (DWORD)(ULONG_PTR)value);
    if (!process) {
      DWORD error = GetLastError();
      // A process object already destroyed has released its handles.
      if (error == ERROR_INVALID_PARAMETER) continue;
      return member_error(error);
    }
    BOOL inside;
    if (!IsProcessInJob(process, members->job, &inside)) {
      DWORD error = GetLastError(); CloseHandle(process); return member_error(error);
    }
    // A reused pid outside this job means the original object was destroyed.
    if (!inside) { CloseHandle(process); continue; }
    members->held[members->count++] = process;
  }
  JOBOBJECT_BASIC_ACCOUNTING_INFORMATION accounting;
  if (!QueryInformationJobObject(members->job, JobObjectBasicAccountingInformation, &accounting, sizeof accounting, NULL)) return member_error(GetLastError());
  *active = accounting.ActiveProcesses;
  if (accounting.TotalProcesses > MAX_JOB_PROCESSES) return member_error(ERROR_TOO_MANY_OPEN_FILES);
  return !*active && members->announced == accounting.TotalProcesses && !members->count ? 2 : 1;
}
static int run_child_within(char *const argv[], unsigned long within_ms) {
  // Quote each UTF-8 argument for CreateProcessW without a command interpreter.
  WCHAR command[32768];
  size_t at = 0;
  for (size_t i = 0; argv[i]; i++) {
    WCHAR *argument = wide(argv[i]);
    size_t size = wcslen(argument);
    if (at + size * 2 + 4 >= sizeof command / sizeof command[0]) { free(argument); return failed("arguments", ERROR_BUFFER_OVERFLOW); }
    if (i) command[at++] = L' ';
    command[at++] = L'"';
    for (size_t n = 0; n < size;) {
      size_t slashes = 0;
      while (n < size && argument[n] == L'\\') { slashes++; n++; }
      size_t copies = slashes * ((n == size || argument[n] == L'"') ? 2 : 1);
      while (copies--) command[at++] = L'\\';
      if (n < size) {
        if (argument[n] == L'"') command[at++] = L'\\';
        command[at++] = argument[n++];
      }
    }
    command[at++] = L'"';
    free(argument);
  }
  command[at] = 0;
  WCHAR *binary = wide(argv[0]);
  HANDLE job = CreateJobObjectW(NULL, NULL);
  JOBOBJECT_EXTENDED_LIMIT_INFORMATION limits = {0};
  limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
  if (!job || !SetInformationJobObject(job, JobObjectExtendedLimitInformation, &limits, sizeof limits)) {
    DWORD error = GetLastError();
    free(binary); if (job) CloseHandle(job); return failed("job", error);
  }
  job_members members = {0};
  members.job = job;
  members.port = CreateIoCompletionPort(INVALID_HANDLE_VALUE, NULL, 0, 1);
  JOBOBJECT_ASSOCIATE_COMPLETION_PORT association = {job, members.port};
  if (!members.port || !SetInformationJobObject(job, JobObjectAssociateCompletionPortInformation, &association, sizeof association)) {
    DWORD error = GetLastError();
    free(binary); if (members.port) CloseHandle(members.port); CloseHandle(job); return failed("job_port", error);
  }
  STARTUPINFOW startup = {0};
  PROCESS_INFORMATION process = {0};
  startup.cb = sizeof startup;
  if (!CreateProcessW(binary, command, NULL, NULL, TRUE, CREATE_SUSPENDED, NULL, NULL, &startup, &process)) {
    DWORD error = GetLastError();
    free(binary); CloseHandle(members.port); CloseHandle(job); return failed("spawn", error);
  }
  free(binary);
  int owned = AssignProcessToJobObject(job, process.hProcess);
  if (!owned) failed("assign_job", GetLastError());
  int resumed = owned && ResumeThread(process.hThread) != (DWORD)-1;
  if (owned && !resumed) failed("resume", GetLastError());
  DWORD status = 1;
  DWORD waited = WAIT_OBJECT_0, active = 0;
  ULONGLONG deadline = GetTickCount64() + within_ms;
  while (resumed) {
    if (!follow_members(&members, &active)) { failed("job_members", cleanup_error); waited = WAIT_FAILED; break; }
    waited = WaitForSingleObject(process.hProcess, 0);
    if (waited != WAIT_TIMEOUT || GetTickCount64() >= deadline) break;
    Sleep(10);
  }
  if (resumed && waited != WAIT_OBJECT_0 && strcmp(run_phase, "exited") == 0) failed(waited == WAIT_TIMEOUT ? "deadline" : "wait", waited == WAIT_FAILED ? GetLastError() : 0);
  if (!resumed || waited != WAIT_OBJECT_0) {
    TerminateProcess(process.hProcess, 1);
    if (owned) TerminateJobObject(job, 1);
    DWORD cleanup_wait = WaitForSingleObject(process.hProcess, 10000);
    if (cleanup_wait != WAIT_OBJECT_0) { cleanup_confirmed = 0; cleanup_error = cleanup_wait == WAIT_FAILED ? GetLastError() : cleanup_wait; }
  } else if (!GetExitCodeProcess(process.hProcess, &status)) status = failed("exit_code", GetLastError());
  CloseHandle(process.hThread);
  CloseHandle(process.hProcess);
  // Confirm every owned descendant exited before the next fixture command.
  if (!TerminateJobObject(job, 1)) { cleanup_confirmed = 0; cleanup_error = GetLastError(); }
  deadline = GetTickCount64() + 10000;
  for (;;) {
    int state = follow_members(&members, &active);
    if (!state || state == 2) break;
    if (GetTickCount64() >= deadline) { cleanup_confirmed = 0; cleanup_error = WAIT_TIMEOUT; break; }
    Sleep(10);
  }
  for (DWORD i = 0; i < members.count; i++) CloseHandle(members.held[i]);
  CloseHandle(job);
  CloseHandle(members.port);
  return cleanup_confirmed ? (int)status : 1;
}
static long long monotonic_ms(void) { return (long long)GetTickCount64(); }
#else
#include <termios.h>
#include <time.h>
#include <unistd.h>
#include <signal.h>
#include <sys/wait.h>
#include <fcntl.h>
#ifdef __linux__
#include <sys/prctl.h>
#endif
#define process_id getpid
#define file_open(value) fopen(value, "wb")
typedef ssize_t fixture_count_t;
#define terminal_read(bytes, size) read(0, bytes, size)
#define terminal_write(bytes, size) write(1, bytes, size)
static volatile sig_atomic_t stopping = 0;
static void stop_runner(int signal_number) { stopping = signal_number; close(0); }
static void initialize_runner(void) {
  struct sigaction action;
  memset(&action, 0, sizeof action);
  action.sa_handler = stop_runner;
  sigemptyset(&action.sa_mask);
  if (sigaction(SIGTERM, &action, NULL) != 0 || sigaction(SIGHUP, &action, NULL) != 0 || sigaction(SIGINT, &action, NULL) != 0) exit(1);
}
static void raw_terminal(void) {
  struct termios tio;
  if (tcgetattr(0, &tio) != 0) exit(1);
  tio.c_lflag &= ~(ICANON | ECHO | IEXTEN);
  tio.c_cc[VMIN] = 1;
  tio.c_cc[VTIME] = 0;
  if (tcsetattr(0, TCSANOW, &tio) != 0) exit(1);
}
static long long monotonic_ms(void) {
  struct timespec now;
  if (clock_gettime(CLOCK_MONOTONIC, &now) != 0) exit(1);
  return (long long)now.tv_sec * 1000 + now.tv_nsec / 1000000;
}
#ifdef __linux__
static int adopted_children_remain(unsigned int *reaped) {
  // This single-threaded runner starts only this command's children. Reap
  // adopted descendants here, rather than waiting on an outer subreaper.
  for (int n = 0; n < 64; n++) {
    int status;
    pid_t child = waitpid(-1, &status, WNOHANG | __WALL);
    if (child > 0) {
      if (++*reaped <= 512) continue;
      cleanup_confirmed = 0; cleanup_error = EOVERFLOW; return -1;
    }
    if (child == 0) return 1;
    if (errno == ECHILD) return 0;
    if (errno == EINTR) continue;
    cleanup_confirmed = 0; cleanup_error = errno; return -1;
  }
  return 1;
}
#endif
static int run_child_within(char *const argv[], unsigned long within_ms) {
#ifdef __linux__
  struct sigaction action = {0};
  action.sa_handler = SIG_DFL;
  sigemptyset(&action.sa_mask);
  if (sigaction(SIGCHLD, &action, NULL) != 0) return failed("child_signal", errno);
  if (prctl(PR_SET_CHILD_SUBREAPER, 1L, 0L, 0L, 0L) != 0) return failed("subreaper", errno);
#endif
  int errors[2];
  if (pipe(errors) != 0) return failed("error_pipe", errno);
  if (fcntl(errors[1], F_SETFD, FD_CLOEXEC) != 0 || fcntl(errors[0], F_SETFL, O_NONBLOCK) != 0) {
    int error = errno; close(errors[0]); close(errors[1]); return failed("error_pipe", error);
  }
  pid_t child = fork();
  if (child < 0) { int error = errno; close(errors[0]); close(errors[1]); return failed("fork", error); }
  if (child == 0) {
    close(errors[0]);
    if (setpgid(0, 0) != 0) { int error = errno; (void)write(errors[1], &error, sizeof error); _exit(1); }
    execv(argv[0], argv);
    int error = errno; (void)write(errors[1], &error, sizeof error);
    _exit(1);
  }
  close(errors[1]);
  long long deadline = monotonic_ms() + within_ms;
  int status = 0, killed = 0;
  for (;;) {
    siginfo_t ended = {0};
    int waited = waitid(P_PID, (id_t)child, &ended, WEXITED | WNOHANG | WNOWAIT);
    if (waited == 0 && ended.si_pid == child) break;
    if (waited < 0 && errno != EINTR) { int error = errno; close(errors[0]); cleanup_confirmed = 0; return failed("wait", error); }
    if ((!killed && stopping) || monotonic_ms() >= deadline) {
      if (killed) { close(errors[0]); cleanup_confirmed = 0; cleanup_error = ETIMEDOUT; return 1; }
      failed(stopping ? "cancelled" : "deadline", 0);
      kill(-child, SIGKILL);
      kill(child, SIGKILL);
      killed = 1;
      deadline = monotonic_ms() + 10000;
    }
    struct timespec delay = {0, 10000000};
    nanosleep(&delay, NULL);
  }
  int error = 0;
  if (read(errors[0], &error, sizeof error) == sizeof error) failed("spawn", error);
  close(errors[0]);
  // Keep the unreaped leader's identity until the last group signal, so a
  // recycled pid can never receive a signal belonging to this command.
  kill(-child, SIGKILL);
  if (waitpid(child, &status, 0) != child) { cleanup_confirmed = 0; return failed("reap", errno); }
  long long cleanup_deadline = monotonic_ms() + 10000;
#ifdef __linux__
  unsigned int reaped = 0;
#endif
  for (;;) {
    int remaining = 0;
#ifdef __linux__
    remaining = adopted_children_remain(&reaped);
    if (remaining < 0) break;
#endif
    if (kill(-child, 0) == -1 && errno == ESRCH && !remaining) break;
    if (monotonic_ms() >= cleanup_deadline) { cleanup_confirmed = 0; cleanup_error = ETIMEDOUT; break; }
    struct timespec delay = {0, 10000000};
    nanosleep(&delay, NULL);
  }
  if (!killed && !WIFEXITED(status)) failed("signal", WIFSIGNALED(status) ? (unsigned long)WTERMSIG(status) : 0);
  return !killed && WIFEXITED(status) ? WEXITSTATUS(status) : 1;
}
#endif
static int run_child(char *const argv[], unsigned long within_ms) {
  reset_outcome();
  long long began = monotonic_ms();
  int status = run_child_within(argv, within_ms);
  elapsed_ms = monotonic_ms() - began;
  return cleanup_confirmed ? status : 1;
}
static int publish(const char *filename, const char *text) {
  size_t size = strlen(filename) + sizeof ".pending";
  char *pending = malloc(size);
  if (!pending) return 1;
  snprintf(pending, size, "%s.pending", filename);
  FILE *file = file_open(pending);
  if (!file) { free(pending); return 1; }
  int written = fputs(text, file) >= 0;
  if (fclose(file) != 0) written = 0;
  int moved = -1;
  if (written) {
#ifdef _WIN32
    WCHAR *from = wide(pending), *to = wide(filename);
    moved = MoveFileExW(from, to, MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH) ? 0 : -1;
    free(from); free(to);
#else
    moved = rename(pending, filename);
#endif
  }
  free(pending);
  return moved != 0;
}
static int complete_command(const char *filename, int status, unsigned long within_ms) {
  size_t size = strlen(filename) + sizeof ".json";
  char *details = malloc(size);
  if (!details) return 1;
  snprintf(details, size, "%s.json", filename);
  char text[768];
  snprintf(text, sizeof text,
    "{\"id\":%lu,\"exit\":%d,\"phase\":\"%s\",\"os_error\":%lu,\"cleanup_confirmed\":%s,\"cleanup_error\":%lu,\"elapsed_ms\":%lld,\"within_ms\":%lu}\n",
    ++sequence, status, run_phase, run_error, cleanup_confirmed ? "true" : "false", cleanup_error, elapsed_ms, within_ms);
  int failed = publish(details, text);
  free(details);
  if (failed) return 1;
  snprintf(text, sizeof text, "%d\n", status);
  return publish(filename, text);
}
// Per-run values come from `provider.config` in $HIDE_E2E_ROOT, the folder the
// fixture's panes are told about: lines 1 and 2 are the program and script
// that report the agent's session the way the pane's engine learns it, run as
// `<program> <script> <pane> <session>`; line 3 is the file the command's
// status is written to, and every line after is one argument of the command
// run on `!`. An optional `within-ms=N` before those arguments gives the
// journey's explicit command budget (desktop keeps 20s). Every fixture writes
// this format through --write-config below.
// The executable itself is built once.
#define MAX_COMMAND 16
static char config_text[32768];
static const char *reporter_program;
static const char *reporter_script;
static const char *completion_file;
static char *command[MAX_COMMAND + 1];
/* Whether the command just run was the agent's own hook (`AgentRun.hook`,
   marked by a file beside the completion file): Claude Code draws nothing
   for a hook, so the screen stays as it was, a permission prompt included,
   while a tool call's command returns to the composer. */
static int ran_as_hook(void) {
  size_t size = strlen(completion_file) + sizeof ".hook";
  char *marker = malloc(size);
  if (!marker) exit(1);
  snprintf(marker, size, "%s.hook", completion_file);
#ifdef _WIN32
  WCHAR *name = wide(marker);
  int found = GetFileAttributesW(name) != INVALID_FILE_ATTRIBUTES;
  free(name);
#else
  int found = access(marker, F_OK) == 0;
#endif
  free(marker);
  return found;
}
static int write_config(int argc, char **argv) {
  // --write-config FILE REPORTER SCRIPT COMPLETION COMMAND [ARG...]
  if (argc < 7 || argc - 6 > MAX_COMMAND) {
    fputs("fixture provider: expected reporter, script, completion and 1..16 command arguments\n", stderr);
    return 1;
  }
  size_t size = 0;
  for (int i = 3; i < argc; i++) {
    if (!*argv[i] || strpbrk(argv[i], "\r\n")) {
      fputs("fixture provider: configuration fields must be nonempty single lines\n", stderr);
      return 1;
    }
    size += strlen(argv[i]) + 1;
  }
  if (size >= sizeof config_text) { fputs("fixture provider: configuration exceeds its byte limit\n", stderr); return 1; }
  FILE *file = file_open(argv[2]);
  if (!file) { perror("fixture provider: write configuration"); return 1; }
  int failed = 0;
  for (int i = 3; i < argc; i++) if (fprintf(file, "%s\n", argv[i]) < 0) failed = 1;
  if (fclose(file) != 0) failed = 1;
  if (failed) fputs("fixture provider: writing configuration failed\n", stderr);
  return failed;
}
static int load_config(void) {
  const char *root = getenv("HIDE_E2E_ROOT");
  if (!root || !*root) { fputs("fixture provider: HIDE_E2E_ROOT is not set\n", stderr); return 1; }
  char path[4096];
  snprintf(path, sizeof path, "%s/provider.config", root);
#ifdef _WIN32
  WCHAR *wide_path = wide(path);
  FILE *file = _wfopen(wide_path, L"rb");
  free(wide_path);
#else
  FILE *file = fopen(path, "rb");
#endif
  if (!file) { fprintf(stderr, "fixture provider: cannot read %s\n", path); return 1; }
  size_t size = fread(config_text, 1, sizeof config_text - 1, file);
  fclose(file);
  config_text[size] = 0;
  int count = 0, arguments = 0;
  for (char *line = config_text; *line;) {
    char *end = strchr(line, '\n');
    if (end) *end = 0;
    size_t length = strlen(line);
    if (length && line[length - 1] == '\r') line[length - 1] = 0;
    if (count == 0) reporter_program = line;
    else if (count == 1) reporter_script = line;
    else if (count == 2) completion_file = line;
    else if (count == 3 && strncmp(line, "within-ms=", 10) == 0) {
      char *end;
      command_ms = strtoul(line + 10, &end, 10);
      if (*end || command_ms < 1 || command_ms > 120000) { fputs("fixture provider: invalid command budget\n", stderr); return 1; }
    }
    else if (arguments < MAX_COMMAND) command[arguments++] = line;
    else { fputs("fixture provider: the command has too many arguments\n", stderr); return 1; }
    count++;
    if (!end) break;
    line = end + 1;
  }
  if (count < 3 || !arguments) { fprintf(stderr, "fixture provider: %s is incomplete\n", path); return 1; }
  command[arguments] = NULL;
  return 0;
}
int main(int argc, char **argv) {
#ifdef _WIN32
  _setmode(0, _O_BINARY);
  _setmode(1, _O_BINARY);
#endif
  if (argc > 1 && strcmp(argv[1], "--write-config") == 0) return write_config(argc, argv);
  if (argc > 1 && strcmp(argv[1], "--version") == 0) { puts("fixture"); return 0; }
  if (argc > 1 && strcmp(argv[1], "auth") == 0) { puts("{\"loggedIn\":true}"); return 0; }
  for (int i = 1; i < argc; i++) if (strcmp(argv[i], "--json-schema") == 0) return 1;
  initialize_runner();
  // Exercise this exact supervisor without a terminal or a Herdr server.
  if (argc > 4 && strcmp(argv[1], "--run-child") == 0) {
    char *end;
    unsigned long within_ms = strtoul(argv[2], &end, 10);
    if (*end || within_ms < 1 || within_ms > 120000) return 1;
    int status = run_child(&argv[4], within_ms);
    if (complete_command(argv[3], status, within_ms) != 0) return 1;
    return status;
  }
  if (load_config() != 0) return 1;
  /* The pane names itself as `hide_agent_hooks::pane` reads it: HIDE_PANE_ID first. */
  const char *pane = getenv("HIDE_PANE_ID");
  if (!pane || !*pane) pane = getenv("HERDR_PANE_ID");
  if (!pane || !*pane) return 1;
  /* A resumed conversation keeps its id, as Claude Code's own does. */
  char session[80];
  snprintf(session, sizeof session, "fixture-%d", process_id());
  for (int i = 1; i + 1 < argc; i++)
    if (strcmp(argv[i], "--resume") == 0) snprintf(session, sizeof session, "%s", argv[i + 1]);
  /* Run under another agent's name (a Grok pane's), the pane is that agent's, which Herdr reads from the
     program; only the Claude Code stand-in reports a session of its own. */
  const char *base = argv[0];
  for (const char *at = argv[0]; *at; at++) if (*at == '/' || *at == '\\') base = at + 1;
  int claude = strcmp(base, "claude") == 0 || strcmp(base, "claude.exe") == 0;
  char *report[] = { (char *)reporter_program, (char *)reporter_script, (char *)pane, session, NULL };
  if (claude && run_child(report, 20000) != 0) return 1;
  raw_terminal();
  static const char ready[] = "\r\nClaude Code fixture\r\nclaude fixture ready\r\n";
  if (terminal_write(ready, sizeof ready - 1) != (fixture_count_t)(sizeof ready - 1)) return 1;
  static const char prompt[] = "\r\n────────────────\r\n❯ \r\n────────────────\r\n";
  if (terminal_write(prompt, sizeof prompt - 1) != (fixture_count_t)(sizeof prompt - 1)) return 1;
  char bytes[16384];
  /* Everything else typed is echoed, as the `claude` stand-in echoes it
     (`claude-shim.c`), so a spec draws the screens an engine reads an
     agent's state from; a console's lone CR is echoed as a line end. */
  static char echo[sizeof bytes * 2];
  fixture_count_t count;
  while ((count = terminal_read(bytes, sizeof bytes)) > 0) {
    size_t out = 0;
    for (fixture_count_t i = 0; i < count; i++) {
      if (bytes[i] == '!') continue;
      echo[out++] = bytes[i];
#ifdef _WIN32
      if (bytes[i] == '\r' && (i + 1 == count || bytes[i + 1] != '\n')) echo[out++] = '\n';
#endif
    }
    if (out && terminal_write(echo, out) != (fixture_count_t)out) return 1;
    for (fixture_count_t i = 0; i < count; i++) {
      if (bytes[i] != '!') continue;
      int status = run_child(command, command_ms);
      int hook = ran_as_hook();
      if (complete_command(completion_file, status, command_ms) != 0 || !cleanup_confirmed) return 1;
      if (!hook && terminal_write(prompt, sizeof prompt - 1) != (fixture_count_t)(sizeof prompt - 1)) return 1;
    }
  }
  return count < 0 ? 1 : 0;
}
