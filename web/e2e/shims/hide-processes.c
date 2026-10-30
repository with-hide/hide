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
//       job had in all, or `survivors <processes> <id> ...` after five
//       seconds, or `unstarted <reason>` when the program did not start.
//       Exits with the program's status when it ended by itself, 1 when it
//       was ended, and 64-71 when it did not start. A run that is killed
//       takes the job with it.
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

static DWORD job_active(HANDLE job, DWORD *total) {
  JOBOBJECT_BASIC_ACCOUNTING_INFORMATION accounting;
  if (!QueryInformationJobObject(job, JobObjectBasicAccountingInformation, &accounting, sizeof accounting, NULL)) return (DWORD)-1;
  *total = accounting.TotalProcesses;
  return accounting.ActiveProcesses;
}

static int run(HANDLE pipe) {
  int argc = 0;
  wchar_t **argv = CommandLineToArgvW(GetCommandLineW(), &argc);
  if (!argv || argc < 3) {
    report(pipe, "unstarted no program to run\n");
    return 64;
  }
  static wchar_t program[32768];
  if (!find_program(argv[2], program, 32768)) {
    report(pipe, "unstarted the program to run is not found\n");
    return 65;
  }
  // The program's own arguments, as this one was given them.
  const wchar_t *arguments = after_argument(after_argument(after_argument(GetCommandLineW())));
  size_t capacity = wcslen(program) + wcslen(arguments) + 3;
  wchar_t *command = malloc(capacity * sizeof *command);
  if (!command || _snwprintf(command, capacity, L"\"%ls\"%ls", program, arguments) < 0) return 70;
  // Killed with this program when it is killed; a program that crashes ends
  // instead of waiting on an error report.
  HANDLE job = CreateJobObjectW(NULL, NULL);
  JOBOBJECT_EXTENDED_LIMIT_INFORMATION limits = {0};
  limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION;
  HANDLE port = CreateIoCompletionPort(INVALID_HANDLE_VALUE, NULL, 0, 1);
  JOBOBJECT_ASSOCIATE_COMPLETION_PORT notify = {job, port};
  if (!job || !port || !SetInformationJobObject(job, JobObjectExtendedLimitInformation, &limits, sizeof limits) ||
      !SetInformationJobObject(job, JobObjectAssociateCompletionPortInformation, &notify, sizeof notify)) {
    report(pipe, "unstarted no job object\n");
    return 70;
  }
  // The program inherits its output and an empty input and nothing else, so
  // nothing it starts holds this program's input or report.
  SECURITY_ATTRIBUTES inherit = {sizeof inherit, NULL, TRUE};
  HANDLE nothing = CreateFileW(L"NUL", GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE, &inherit, OPEN_EXISTING, 0, NULL);
  HANDLE out = GetStdHandle(STD_OUTPUT_HANDLE);
  HANDLE err = GetStdHandle(STD_ERROR_HANDLE);
  HANDLE inherited[3] = {nothing, out, err};
  DWORD inherited_count = err == out ? 2 : 3;
  for (DWORD i = 1; i < inherited_count; i += 1) SetHandleInformation(inherited[i], HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT);
  SIZE_T attributes_size = 0;
  InitializeProcThreadAttributeList(NULL, 1, 0, &attributes_size);
  STARTUPINFOEXW startup = {0};
  startup.StartupInfo.cb = sizeof startup;
  startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
  startup.StartupInfo.hStdInput = nothing;
  startup.StartupInfo.hStdOutput = out;
  startup.StartupInfo.hStdError = err;
  startup.lpAttributeList = malloc(attributes_size);
  if (nothing == INVALID_HANDLE_VALUE || !startup.lpAttributeList || !InitializeProcThreadAttributeList(startup.lpAttributeList, 1, 0, &attributes_size) ||
      !UpdateProcThreadAttribute(startup.lpAttributeList, 0, PROC_THREAD_ATTRIBUTE_HANDLE_LIST, inherited, inherited_count * sizeof(HANDLE), NULL, NULL)) {
    report(pipe, "unstarted the program's handles cannot be set\n");
    return 70;
  }
  // Suspended until it is in the job, so it starts nothing outside it.
  PROCESS_INFORMATION started;
  if (!CreateProcessW(program, command, NULL, NULL, TRUE, CREATE_SUSPENDED | EXTENDED_STARTUPINFO_PRESENT, NULL, NULL, &startup.StartupInfo, &started)) {
    char line[96];
    snprintf(line, sizeof line, "unstarted the program could not start (error %lu)\n", GetLastError());
    report(pipe, line);
    return 71;
  }
  if (!AssignProcessToJobObject(job, started.hProcess)) {
    char line[96];
    snprintf(line, sizeof line, "unstarted the program could not join its job (error %lu)\n", GetLastError());
    // Still suspended: it ran nothing, and ends here.
    TerminateProcess(started.hProcess, 1);
    if (WaitForSingleObject(started.hProcess, END_MS) != WAIT_OBJECT_0) snprintf(line, sizeof line, "survivors 1 %lu\n", started.dwProcessId);
    report(pipe, line);
    return 71;
  }
  ResumeThread(started.hThread);
  CloseHandle(started.hThread);
  HANDLE ended = CreateEventW(NULL, TRUE, FALSE, NULL);
  HANDLE watcher = ended ? CreateThread(NULL, 0, caller_end, ended, 0, NULL) : NULL;
  HANDLE waits[2] = {started.hProcess, ended};
  int by_itself = WaitForMultipleObjects(watcher ? 2 : 1, waits, FALSE, INFINITE) == WAIT_OBJECT_0;
  DWORD status = 1;
  if (by_itself && !GetExitCodeProcess(started.hProcess, &status)) status = 1;
  // Ended again while any is left: a process that was starting one as the
  // job ended can still add it.
  DWORD deadline = GetTickCount() + END_MS;
  DWORD total = 0;
  DWORD active;
  for (;;) {
    TerminateJobObject(job, 1);
    active = job_active(job, &total);
    if (active == 0 || (LONG)(GetTickCount() - deadline) >= 0) break;
    // Woken by the job's last process ending; a message the system did not
    // deliver costs one slice, after which the count is read again.
    DWORD message, left = deadline - GetTickCount();
    ULONG_PTR key;
    LPOVERLAPPED overlapped;
    GetQueuedCompletionStatus(port, &message, &key, &overlapped, left < 100 ? left : 100);
  }
  char line[64];
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
