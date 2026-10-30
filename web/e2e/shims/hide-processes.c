// The fixture's view of Windows processes, read from the system and handles
// directly instead of through PowerShell and WMI, whose cold start and query
// time on a loaded runner once outlasted a 30 s limit (`spawnSync powershell.exe
// ETIMEDOUT`, #560).
//
//   hide-processes tree <pid>
//       prints `<id> <created>` for the process and every live descendant,
//       one per line; `created` is its start as a FILETIME in decimal.
//   hide-processes end <root|-> [<id>:<created> ...]
//       ends those processes, everything they started (also what a process
//       started after it was listed), and, given a root folder, every process
//       running an executable under it; returns once none is left. Each end is
//       waited for on the process's own handle. Exits 1, naming the survivors
//       on stderr, when some remain after five seconds.
//   hide-processes run <program> [<argument> ...]
//       runs the program in a job object from its first instruction, so
//       everything it starts, at any depth and whatever exits first, is in
//       the job (`runBounded` in web/e2e/bounded-child.ts). The program gets
//       this one's standard output and error and no input, and its
//       arguments exactly as this one was given them. When the program
//       exits, or this one's standard input ends or has a byte (the
//       caller's end), the job is ended and waited for until none of it is
//       left, and one line is written to descriptor 3: `ended <status>
//       <processes>`, the program's exit status and how many processes the
//       job had in all; `survivors <processes> <id> ...` after five
//       seconds; `unstarted <reason> (error <n>)` when the program did not
//       start; `failed <reason> (error <n>)` when its end could not be
//       followed but the job was ended and is empty; or `unconfirmed <reason>
//       (error <n>)` when the job could not be ended or counted. Exits with
//       the program's status when it ended by itself, 1 when it was ended,
//       and 64-71 when it did not start. The program is created inside the
//       job, and a run that is killed takes the job with it.
//
// A process is its id and its start together, so a reused id is never taken
// for it. A process is a child of a parent when it names the parent's id and
// started no earlier than the parent; if a live process with that id started
// before the child and is not the parent, it is the child's real parent
// instead (Windows reuses the id of an exited parent).
#include <windows.h>
#include <shellapi.h>
#include <tlhelp32.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <wchar.h>

#pragma comment(lib, "shell32.lib")

// Windows 10's attribute that creates a process inside a job; older SDK
// headers do not name it.
#ifndef PROC_THREAD_ATTRIBUTE_JOB_LIST
#define PROC_THREAD_ATTRIBUTE_JOB_LIST ProcThreadAttributeValue(13, FALSE, TRUE, FALSE)
#endif

#define MAX_ROWS 8192
#define LIMIT 512
#define END_MS 5000

typedef struct {
  DWORD id;
  DWORD parent;
  ULONGLONG created;
  char path[MAX_PATH];
  int marked;
} Row;

static Row rows[MAX_ROWS];
static int row_count;

// When `process` started, in FILETIME units; 0 when Windows does not say.
static ULONGLONG started(HANDLE process) {
  FILETIME created, exited, kernel, user;
  if (!GetProcessTimes(process, &created, &exited, &kernel, &user)) return 0;
  return ((ULONGLONG)created.dwHighDateTime << 32) | created.dwLowDateTime;
}

// Reads the live processes into `rows`.
static int snapshot(void) {
  HANDLE all = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
  if (all == INVALID_HANDLE_VALUE) return 0;
  PROCESSENTRY32 entry;
  entry.dwSize = sizeof entry;
  row_count = 0;
  for (BOOL more = Process32First(all, &entry); more && row_count < MAX_ROWS; more = Process32Next(all, &entry)) {
    Row *row = &rows[row_count];
    row->id = entry.th32ProcessID;
    row->parent = entry.th32ParentProcessID;
    row->created = 0;
    row->path[0] = 0;
    row->marked = 0;
    HANDLE process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, row->id);
    if (process) {
      row->created = started(process);
      DWORD size = sizeof row->path;
      if (!QueryFullProcessImageNameA(process, 0, row->path, &size)) row->path[0] = 0;
      CloseHandle(process);
    }
    row_count += 1;
  }
  CloseHandle(all);
  return 1;
}

// Owned identities, the dead ones the caller named included: a child starts
// after its parent and names its id, so a child of a dead owner is found.
typedef struct {
  DWORD id;
  ULONGLONG created;
} Identity;

static Identity owned[LIMIT];
static int owned_count;

static int is_owned(DWORD id, ULONGLONG created) {
  for (int i = 0; i < owned_count; i += 1) {
    if (owned[i].id == id && owned[i].created == created) return 1;
  }
  return 0;
}

// Grows `owned` with every live process that is a child of one of them.
static int grow(void) {
  int from = 0;
  while (from < owned_count) {
    int until = owned_count;
    for (; from < until; from += 1) {
      Identity parent = owned[from];
      for (int i = 0; i < row_count; i += 1) {
        Row *row = &rows[i];
        if (row->parent != parent.id || row->created == 0 || row->created < parent.created) continue;
        if (is_owned(row->id, row->created)) continue;
        // A live process holding the parent's id that is not the parent and
        // started no later than this one is its real parent.
        int taken = 0;
        for (int j = 0; j < row_count; j += 1) {
          Row *holder = &rows[j];
          if (holder->id == parent.id && holder->created != parent.created && holder->created != 0 && holder->created <= row->created) taken = 1;
        }
        if (taken) continue;
        if (owned_count >= LIMIT) {
          fprintf(stderr, "fixture process tree exceeded %d processes\n", LIMIT);
          return 0;
        }
        owned[owned_count].id = row->id;
        owned[owned_count].created = row->created;
        owned_count += 1;
      }
    }
  }
  return 1;
}

static int starts_with(const char *text, const char *prefix) {
  size_t length = strlen(prefix);
  return length > 0 && _strnicmp(text, prefix, length) == 0;
}

static int tree(DWORD pid) {
  if (!snapshot()) return 1;
  int found = 0;
  for (int i = 0; i < row_count; i += 1) {
    if (rows[i].id == pid && rows[i].created != 0) {
      owned[0].id = pid;
      owned[0].created = rows[i].created;
      owned_count = 1;
      found = 1;
    }
  }
  // A process that is not there has no tree to list.
  if (!found) return 0;
  if (!grow()) return 1;
  for (int i = 0; i < owned_count; i += 1) printf("%lu %llu\n", (unsigned long)owned[i].id, owned[i].created);
  return 0;
}

// Ends every process in `rows` marked for it, and waits for each to be gone on
// its own handle. Returns how many were marked.
static int end_marked(DWORD deadline) {
  int marked = 0;
  for (int i = 0; i < row_count; i += 1) {
    Row *row = &rows[i];
    if (!row->marked || row->id == GetCurrentProcessId()) continue;
    HANDLE process = OpenProcess(PROCESS_TERMINATE | SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION, FALSE, row->id);
    if (!process) continue;
    // The id may have been reused since the snapshot.
    if (started(process) == row->created) {
      marked += 1;
      TerminateProcess(process, 1);
      DWORD now = GetTickCount();
      DWORD left = deadline > now ? deadline - now : 0;
      WaitForSingleObject(process, left);
    }
    CloseHandle(process);
  }
  return marked;
}

static int end(const char *root, int argc, char **argv) {
  owned_count = 0;
  for (int i = 0; i < argc; i += 1) {
    unsigned long id = 0;
    unsigned long long created = 0;
    if (sscanf(argv[i], "%lu:%llu", &id, &created) != 2 || owned_count >= LIMIT) {
      fprintf(stderr, "bad process identity: %s\n", argv[i]);
      return 64;
    }
    owned[owned_count].id = (DWORD)id;
    owned[owned_count].created = created;
    owned_count += 1;
  }
  DWORD deadline = GetTickCount() + END_MS;
  for (;;) {
    if (!snapshot()) return 1;
    if (!grow()) return 1;
    int targets = 0;
    for (int i = 0; i < row_count; i += 1) {
      Row *row = &rows[i];
      row->marked = (row->created != 0 && is_owned(row->id, row->created)) || (root[0] && row->path[0] && starts_with(row->path, root));
      if (row->marked && row->id != GetCurrentProcessId()) targets += 1;
    }
    if (targets == 0) return 0;
    end_marked(deadline);
    if ((LONG)(GetTickCount() - deadline) >= 0) break;
  }
  // What is still there is named; a process that ended meanwhile is not.
  if (!snapshot()) return 1;
  int survivors = 0;
  for (int i = 0; i < row_count; i += 1) {
    Row *row = &rows[i];
    if (row->id == GetCurrentProcessId()) continue;
    if ((row->created != 0 && is_owned(row->id, row->created)) || (root[0] && row->path[0] && starts_with(row->path, root))) {
      if (survivors == 0) fprintf(stderr, "fixture processes still running after %d s:", END_MS / 1000);
      fprintf(stderr, " %lu %s", (unsigned long)row->id, row->path);
      survivors += 1;
    }
  }
  if (survivors == 0) return 0;
  fprintf(stderr, "\n");
  return 1;
}

// The caller's descriptor 3: libuv, which starts this program for Node,
// hands a child its descriptors in STARTUPINFO's reserved block, an int
// count, then one flag byte and then one HANDLE per descriptor.
static HANDLE report_pipe(void) {
  STARTUPINFOW startup;
  GetStartupInfoW(&startup);
  int count = 0;
  if (!startup.lpReserved2 || startup.cbReserved2 < sizeof count) return INVALID_HANDLE_VALUE;
  memcpy(&count, startup.lpReserved2, sizeof count);
  if (count < 4 || startup.cbReserved2 < sizeof count + (size_t)count + (size_t)count * sizeof(HANDLE)) return INVALID_HANDLE_VALUE;
  HANDLE pipe;
  memcpy(&pipe, startup.lpReserved2 + sizeof count + count + 3 * sizeof(HANDLE), sizeof pipe);
  return pipe;
}

static void report(HANDLE pipe, const char *line) {
  DWORD written;
  WriteFile(pipe, line, (DWORD)strlen(line), &written, NULL);
}

// Where the command line's argument after `from` starts, read as the C
// runtime reads it: an unquoted space or tab ends an argument, a quote opens
// or closes quoting, and backslashes are plain unless a quote follows them.
static const wchar_t *after_argument(const wchar_t *from) {
  const wchar_t *at = from;
  while (*at == L' ' || *at == L'\t') at += 1;
  int quoted = 0;
  while (*at && (quoted || (*at != L' ' && *at != L'\t'))) {
    if (*at == L'\\') {
      size_t slashes = 0;
      while (*at == L'\\') at += 1, slashes += 1;
      if (*at == L'"' && slashes % 2 == 1) at += 1;
      continue;
    }
    if (*at == L'"') quoted = !quoted;
    at += 1;
  }
  return at;
}

static int is_file(const wchar_t *path) {
  DWORD attributes = GetFileAttributesW(path);
  return attributes != INVALID_FILE_ATTRIBUTES && !(attributes & FILE_ATTRIBUTE_DIRECTORY);
}

// The program a name means, as Node finds one: a path as given, a bare name
// in the working folder and then in each `PATH` folder; a name with no
// extension is looked for with `.exe`.
static int find_program(const wchar_t *name, wchar_t *found, size_t size) {
  const wchar_t *base = name;
  for (const wchar_t *at = name; *at; at += 1) {
    if (*at == L'\\' || *at == L'/' || *at == L':') base = at + 1;
  }
  const wchar_t *extension = wcschr(base, L'.') ? L"" : L".exe";
  if (base != name) return _snwprintf(found, size, L"%ls%ls", name, extension) >= 0 && is_file(found);
  static wchar_t folders[32768];
  DWORD cwd = GetCurrentDirectoryW(MAX_PATH, folders);
  if (cwd == 0 || cwd >= MAX_PATH) return 0;
  folders[cwd] = L';';
  DWORD path = GetEnvironmentVariableW(L"PATH", folders + cwd + 1, (DWORD)(32768 - cwd - 1));
  if (path >= 32768 - cwd - 1) return 0;
  folders[cwd + 1 + path] = 0;
  wchar_t *rest = NULL;
  for (wchar_t *folder = wcstok_s(folders, L";", &rest); folder; folder = wcstok_s(NULL, L";", &rest)) {
    // A `PATH` entry may be quoted.
    size_t length = wcslen(folder);
    if (length >= 2 && folder[0] == L'"' && folder[length - 1] == L'"') folder[length - 1] = 0, folder += 1;
    if (*folder && _snwprintf(found, size, L"%ls\\%ls%ls", folder, name, extension) >= 0 && is_file(found)) return 1;
  }
  return 0;
}

// Ends when this program's standard input does: the caller's end of a run.
static DWORD WINAPI caller_end(LPVOID ended) {
  char byte;
  DWORD read;
  ReadFile(GetStdHandle(STD_INPUT_HANDLE), &byte, 1, &read, NULL);
  SetEvent((HANDLE)ended);
  return 0;
}

// How many processes of the job still run, and how many it had in all;
// FALSE, with the Win32 error left to read, when Windows does not say.
static BOOL job_count(HANDLE job, DWORD *active, DWORD *total) {
  JOBOBJECT_BASIC_ACCOUNTING_INFORMATION accounting;
  if (!QueryInformationJobObject(job, JobObjectBasicAccountingInformation, &accounting, sizeof accounting, NULL)) return FALSE;
  *active = accounting.ActiveProcesses;
  *total = accounting.TotalProcesses;
  return TRUE;
}

// A program that never ran: nothing of it is left to end.
static int unstarted(HANDLE pipe, const char *what, DWORD error, int code) {
  char line[160];
  snprintf(line, sizeof line, "unstarted %s (error %lu)\n", what, (unsigned long)error);
  report(pipe, line);
  return code;
}

static int run(HANDLE pipe) {
  int argc = 0;
  wchar_t **argv = CommandLineToArgvW(GetCommandLineW(), &argc);
  if (!argv) return unstarted(pipe, "the command line could not be read", GetLastError(), 64);
  if (argc < 3) return unstarted(pipe, "no program to run", ERROR_BAD_ARGUMENTS, 64);
  static wchar_t program[32768];
  if (!find_program(argv[2], program, 32768)) return unstarted(pipe, "the program to run is not found", ERROR_FILE_NOT_FOUND, 65);
  // The program's own arguments, as this one was given them.
  const wchar_t *arguments = after_argument(after_argument(after_argument(GetCommandLineW())));
  size_t capacity = wcslen(program) + wcslen(arguments) + 3;
  wchar_t *command = malloc(capacity * sizeof *command);
  if (!command) return unstarted(pipe, "no memory for the command line", ERROR_NOT_ENOUGH_MEMORY, 70);
  if (_snwprintf(command, capacity, L"\"%ls\"%ls", program, arguments) < 0) return unstarted(pipe, "the command line does not fit", ERROR_BUFFER_OVERFLOW, 70);
  // Killed with this program when it is killed; a program that crashes ends
  // instead of waiting on an error report.
  HANDLE job = CreateJobObjectW(NULL, NULL);
  if (!job) return unstarted(pipe, "no job object", GetLastError(), 70);
  JOBOBJECT_EXTENDED_LIMIT_INFORMATION limits = {0};
  limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION;
  if (!SetInformationJobObject(job, JobObjectExtendedLimitInformation, &limits, sizeof limits)) return unstarted(pipe, "the job's limits cannot be set", GetLastError(), 70);
  HANDLE port = CreateIoCompletionPort(INVALID_HANDLE_VALUE, NULL, 0, 1);
  if (!port) return unstarted(pipe, "no completion port for the job", GetLastError(), 70);
  JOBOBJECT_ASSOCIATE_COMPLETION_PORT notify = {job, port};
  if (!SetInformationJobObject(job, JobObjectAssociateCompletionPortInformation, &notify, sizeof notify)) return unstarted(pipe, "the job's completion port cannot be set", GetLastError(), 70);
  // The program inherits its output and an empty input and nothing else, so
  // nothing it starts holds this program's input or report.
  SECURITY_ATTRIBUTES inherit = {sizeof inherit, NULL, TRUE};
  HANDLE nothing = CreateFileW(L"NUL", GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE, &inherit, OPEN_EXISTING, 0, NULL);
  if (nothing == INVALID_HANDLE_VALUE) return unstarted(pipe, "NUL cannot be opened", GetLastError(), 70);
  HANDLE out = GetStdHandle(STD_OUTPUT_HANDLE);
  HANDLE err = GetStdHandle(STD_ERROR_HANDLE);
  HANDLE inherited[3] = {nothing, out, err};
  DWORD inherited_count = err == out ? 2 : 3;
  for (DWORD i = 1; i < inherited_count; i += 1) {
    if (!SetHandleInformation(inherited[i], HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT)) return unstarted(pipe, "the output cannot be handed on", GetLastError(), 70);
  }
  // The program is created in the job, so no moment exists in which it runs,
  // or this one dies leaving it, outside the job.
  SIZE_T attributes_size = 0;
  InitializeProcThreadAttributeList(NULL, 2, 0, &attributes_size);
  STARTUPINFOEXW startup = {0};
  startup.StartupInfo.cb = sizeof startup;
  startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
  startup.StartupInfo.hStdInput = nothing;
  startup.StartupInfo.hStdOutput = out;
  startup.StartupInfo.hStdError = err;
  startup.lpAttributeList = malloc(attributes_size);
  if (!startup.lpAttributeList) return unstarted(pipe, "no memory for the start attributes", ERROR_NOT_ENOUGH_MEMORY, 70);
  if (!InitializeProcThreadAttributeList(startup.lpAttributeList, 2, 0, &attributes_size) ||
      !UpdateProcThreadAttribute(startup.lpAttributeList, 0, PROC_THREAD_ATTRIBUTE_HANDLE_LIST, inherited, inherited_count * sizeof(HANDLE), NULL, NULL) ||
      !UpdateProcThreadAttribute(startup.lpAttributeList, 0, PROC_THREAD_ATTRIBUTE_JOB_LIST, &job, sizeof job, NULL, NULL)) {
    return unstarted(pipe, "the start attributes cannot be set", GetLastError(), 70);
  }
  // What the run waits on is ready before the program exists, so nothing
  // can fail between its start and the wait.
  HANDLE ended = CreateEventW(NULL, TRUE, FALSE, NULL);
  if (!ended) return unstarted(pipe, "no event for the caller's end", GetLastError(), 70);
  HANDLE watcher = CreateThread(NULL, 0, caller_end, ended, 0, NULL);
  if (!watcher) return unstarted(pipe, "no thread to watch for the caller's end", GetLastError(), 70);
  PROCESS_INFORMATION started;
  if (!CreateProcessW(program, command, NULL, NULL, TRUE, EXTENDED_STARTUPINFO_PRESENT, NULL, NULL, &startup.StartupInfo, &started)) return unstarted(pipe, "the program could not start", GetLastError(), 71);
  CloseHandle(started.hThread);
  HANDLE waits[2] = {started.hProcess, ended};
  DWORD waited = WaitForMultipleObjects(2, waits, FALSE, INFINITE);
  int by_itself = waited == WAIT_OBJECT_0;
  // A run that could not be followed to its end still ends its job, and says why.
  const char *failure = NULL;
  DWORD failure_error = 0;
  DWORD status = 1;
  if (waited == WAIT_FAILED) {
    failure = "waiting on the program failed";
    failure_error = GetLastError();
  } else if (by_itself && !GetExitCodeProcess(started.hProcess, &status)) {
    failure = "the program's exit status cannot be read";
    failure_error = GetLastError();
  }
  // Ended again while any is left: a process that was starting one as the
  // job ended can still add it.
  DWORD deadline = GetTickCount() + END_MS;
  DWORD active = 0;
  DWORD total = 0;
  char line[160];
  for (;;) {
    if (!TerminateJobObject(job, 1)) {
      snprintf(line, sizeof line, "unconfirmed the job cannot be ended (error %lu)\n", (unsigned long)GetLastError());
      report(pipe, line);
      return 1;
    }
    if (!job_count(job, &active, &total)) {
      snprintf(line, sizeof line, "unconfirmed the job cannot be counted (error %lu)\n", (unsigned long)GetLastError());
      report(pipe, line);
      return 1;
    }
    if (active == 0 || (LONG)(GetTickCount() - deadline) >= 0) break;
    // Woken by the job's last process ending; a message the system did not
    // deliver costs one slice, after which the count is read again.
    DWORD message, left = deadline - GetTickCount();
    ULONG_PTR key;
    LPOVERLAPPED overlapped;
    GetQueuedCompletionStatus(port, &message, &key, &overlapped, left < 100 ? left : 100);
  }
  if (active == 0 && failure) {
    snprintf(line, sizeof line, "failed %s (error %lu)\n", failure, (unsigned long)failure_error);
    report(pipe, line);
    return 1;
  }
  if (active == 0) {
    snprintf(line, sizeof line, "ended %lu %lu\n", (unsigned long)status, (unsigned long)total);
    report(pipe, line);
    return by_itself ? (int)status : 1;
  }
  snprintf(line, sizeof line, "survivors %lu", (unsigned long)total);
  report(pipe, line);
  struct {
    JOBOBJECT_BASIC_PROCESS_ID_LIST list;
    ULONG_PTR more[LIMIT];
  } ids;
  if (QueryInformationJobObject(job, JobObjectBasicProcessIdList, &ids, sizeof ids, NULL)) {
    for (DWORD i = 0; i < ids.list.NumberOfProcessIdsInList; i += 1) {
      snprintf(line, sizeof line, " %lu", (unsigned long)ids.list.ProcessIdList[i]);
      report(pipe, line);
    }
  }
  report(pipe, "\n");
  return 1;
}

int main(int argc, char **argv) {
  if (argc >= 3 && strcmp(argv[1], "run") == 0) {
    HANDLE pipe = report_pipe();
    if (pipe == INVALID_HANDLE_VALUE) {
      fprintf(stderr, "hide-processes run reports on descriptor 3, which it was not given\n");
      return 64;
    }
    return run(pipe);
  }
  if (argc >= 3 && strcmp(argv[1], "tree") == 0) return tree((DWORD)strtoul(argv[2], NULL, 10));
  if (argc >= 3 && strcmp(argv[1], "end") == 0) {
    // `-` names no root.
    const char *root = strcmp(argv[2], "-") == 0 ? "" : argv[2];
    return end(root, argc - 3, argv + 3);
  }
  return 64;
}
