#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <errno.h>
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
static int run_child(char *const argv[]) {
  // Quote each UTF-8 argument for CreateProcessW without a command interpreter.
  WCHAR command[32768];
  size_t at = 0;
  for (size_t i = 0; argv[i]; i++) {
    WCHAR *argument = wide(argv[i]);
    size_t size = wcslen(argument);
    if (at + size * 2 + 4 >= sizeof command / sizeof command[0]) { free(argument); return 1; }
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
    free(binary); if (job) CloseHandle(job); return 1;
  }
  STARTUPINFOW startup = {0};
  PROCESS_INFORMATION process = {0};
  startup.cb = sizeof startup;
  if (!CreateProcessW(binary, command, NULL, NULL, TRUE, CREATE_SUSPENDED, NULL, NULL, &startup, &process)) {
    free(binary); CloseHandle(job); return 1;
  }
  free(binary);
  int owned = AssignProcessToJobObject(job, process.hProcess);
  int resumed = owned && ResumeThread(process.hThread) != (DWORD)-1;
  DWORD status = 1;
  if (!resumed || WaitForSingleObject(process.hProcess, 20000) != WAIT_OBJECT_0) {
    TerminateProcess(process.hProcess, 1);
    if (owned) TerminateJobObject(job, 1);
    if (WaitForSingleObject(process.hProcess, 10000) != WAIT_OBJECT_0) exit(1);
  } else if (!GetExitCodeProcess(process.hProcess, &status)) status = 1;
  CloseHandle(process.hThread);
  CloseHandle(process.hProcess);
  // Confirm every owned descendant exited before the next fixture command.
  if (!TerminateJobObject(job, 1)) status = 1;
  ULONGLONG deadline = GetTickCount64() + 10000;
  for (;;) {
    JOBOBJECT_BASIC_ACCOUNTING_INFORMATION accounting;
    if (!QueryInformationJobObject(job, JobObjectBasicAccountingInformation, &accounting, sizeof accounting, NULL)) { status = 1; break; }
    if (!accounting.ActiveProcesses) break;
    if (GetTickCount64() >= deadline) { CloseHandle(job); exit(1); }
    Sleep(10);
  }
  CloseHandle(job);
  return (int)status;
}
#else
#include <termios.h>
#include <time.h>
#include <unistd.h>
#include <signal.h>
#include <sys/wait.h>
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
static int run_child(char *const argv[]) {
  pid_t child = fork();
  if (child < 0) return 1;
  if (child == 0) {
    if (setpgid(0, 0) != 0) _exit(1);
    execv(argv[0], argv);
    _exit(1);
  }
  long long deadline = monotonic_ms() + 20000;
  int status = 0, killed = 0;
  for (;;) {
    pid_t waited = waitpid(child, &status, WNOHANG);
    if (waited == child) break;
    if (waited < 0 && errno != EINTR) { kill(-child, SIGKILL); kill(child, SIGKILL); exit(1); }
    if ((!killed && stopping) || monotonic_ms() >= deadline) {
      if (killed) exit(1);
      kill(-child, SIGKILL);
      kill(child, SIGKILL);
      killed = 1;
      deadline = monotonic_ms() + 10000;
    }
    struct timespec delay = {0, 10000000};
    nanosleep(&delay, NULL);
  }
  kill(-child, SIGKILL); // The script execs Hide; discard any owned remnants.
  return !killed && WIFEXITED(status) ? WEXITSTATUS(status) : 1;
}
#endif
static int complete_command(const char *filename, int status) {
  size_t size = strlen(filename) + sizeof ".pending";
  char *pending = malloc(size);
  if (!pending) return 1;
  snprintf(pending, size, "%s.pending", filename);
  FILE *file = file_open(pending);
  if (!file) { free(pending); return 1; }
  int written = fprintf(file, "%d\n", status) >= 0;
  if (fclose(file) != 0) written = 0;
  int moved = -1;
  if (written) {
#ifdef _WIN32
    WCHAR *from = wide(pending), *to = wide(filename);
    moved = _wrename(from, to);
    free(from); free(to);
#else
    moved = rename(pending, filename);
#endif
  }
  free(pending);
  return moved != 0;
}
// Per-run values come from `provider.config` in $HIDE_E2E_ROOT, the folder the
// fixture's Herdr panes are told about: line 1 is the Herdr binary, line 2 the
// file the command's status is written to, and every line after is one
// argument of the command run on `!`. The executable itself is built once.
#define MAX_COMMAND 16
static char config_text[32768];
static const char *herdr_binary;
static const char *completion_file;
static char *command[MAX_COMMAND + 1];
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
  int count = 0;
  for (char *line = config_text; *line;) {
    char *end = strchr(line, '\n');
    if (end) *end = 0;
    size_t length = strlen(line);
    if (length && line[length - 1] == '\r') line[length - 1] = 0;
    if (count == 0) herdr_binary = line;
    else if (count == 1) completion_file = line;
    else if (count - 2 < MAX_COMMAND) command[count - 2] = line;
    else { fputs("fixture provider: the command has too many arguments\n", stderr); return 1; }
    count++;
    if (!end) break;
    line = end + 1;
  }
  if (count < 3) { fprintf(stderr, "fixture provider: %s is incomplete\n", path); return 1; }
  command[count - 2] = NULL;
  return 0;
}
int main(int argc, char **argv) {
#ifdef _WIN32
  _setmode(0, _O_BINARY);
  _setmode(1, _O_BINARY);
#endif
  if (argc > 1 && strcmp(argv[1], "--version") == 0) { puts("fixture"); return 0; }
  if (argc > 1 && strcmp(argv[1], "auth") == 0) { puts("{\"loggedIn\":true}"); return 0; }
  for (int i = 1; i < argc; i++) if (strcmp(argv[i], "--json-schema") == 0) return 1;
  initialize_runner();
  if (load_config() != 0) return 1;
  const char *pane = getenv("HERDR_PANE_ID");
  if (!pane || !*pane) return 1;
  /* A resumed conversation keeps its id, as Claude Code's own does. */
  char session[80];
  snprintf(session, sizeof session, "fixture-%d", process_id());
  for (int i = 1; i + 1 < argc; i++)
    if (strcmp(argv[i], "--resume") == 0) snprintf(session, sizeof session, "%s", argv[i + 1]);
  char *report[] = { (char *)herdr_binary, "pane", "report-agent-session", (char *)pane,
    "--source", "herdr:claude", "--agent", "claude", "--agent-session-id", session, "--seq", "1", NULL };
  if (run_child(report) != 0) return 1;
  raw_terminal();
  static const char ready[] = "\r\nClaude Code fixture\r\nclaude fixture ready\r\n❯ ";
  if (terminal_write(ready, sizeof ready - 1) != (fixture_count_t)(sizeof ready - 1)) return 1;
  char bytes[16384];
  fixture_count_t count;
  while ((count = terminal_read(bytes, sizeof bytes)) > 0) {
    for (fixture_count_t i = 0; i < count; i++) {
      if (bytes[i] != '!') continue;
      int status = run_child(command);
      if (complete_command(completion_file, status) != 0) return 1;
      static const char prompt[] = "\r\n❯ ";
      if (terminal_write(prompt, sizeof prompt - 1) != (fixture_count_t)(sizeof prompt - 1)) return 1;
    }
  }
  return count < 0 ? 1 : 0;
}
