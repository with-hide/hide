#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <errno.h>
#ifdef _WIN32
#include <windows.h>
#include <io.h>
#include <sys/stat.h>
#include <process.h>
#define read _read
#define write _write
#define open _open
typedef int fixture_count_t;
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
#else
#include <termios.h>
#include <time.h>
#include <unistd.h>
typedef ssize_t fixture_count_t;
#define terminal_read(bytes, size) read(0, bytes, size)
#define terminal_write(bytes, size) write(1, bytes, size)
static void raw_terminal(void) {
  struct termios tio;
  if (tcgetattr(0, &tio) == 0) {
    tio.c_lflag &= ~(ICANON | ECHO | IEXTEN);
    tio.c_cc[VMIN] = 1;
    tio.c_cc[VTIME] = 0;
    tcsetattr(0, TCSANOW, &tio);
  }
}
#endif
static char prompt[1 << 20];
/* The schema a request asks its answer to fit: `--json-schema`, or, for a
   plain-JSON request, the schema its `--system-prompt` carries. */
static const char *request_schema(int argc, char **argv, int *plain) {
  const char *schema = NULL, *system = NULL;
  for (int i = 1; i + 1 < argc; i++) {
    if (strcmp(argv[i], "--json-schema") == 0) schema = argv[i + 1];
    if (strcmp(argv[i], "--system-prompt") == 0) system = argv[i + 1];
  }
  *plain = !schema && system;
  return schema ? schema : system;
}
/* A provider answers in the shape its schema asks for: a turn start's
   schema has no `end`, so the label's `end` is left out of that answer. */
static int fit_schema(char *label, int argc, char **argv) {
  int plain;
  const char *schema = request_schema(argc, argv, &plain);
  if (!schema) return 1;
  if (strstr(schema, "\"end\"")) {
    /* Recorded native-format fixtures retain their old label marker. This
       controlled provider gives that unrelated reader journey no work judgment
       under the current END schema, without rewriting the recorded blobs. */
    if (strstr(schema, "\"work\"") && !strstr(label, ",\"work\":")) {
      const char *work = ",\"work\":{\"task\":{\"kind\":\"none\"},\"missed\":[],\"resolved\":[],\"output_task_id\":null}}";
      size_t length = strlen(label);
      size_t capacity = sizeof prompt - (size_t)(label - prompt);
      if (!length || label[length - 1] != '}' || length - 1 + strlen(work) >= capacity) return 0;
      memcpy(label + length - 1, work, strlen(work) + 1);
    }
    return 1;
  }
  /* The optional work judgment is the last field of the fixture marker.
     START asks for neither work nor end. */
  char *work = strstr(label, ",\"work\":");
  if (work) { work[0] = '}'; work[1] = 0; }
  char *field = strstr(label, ",\"end\":\"");
  char *close = field ? strchr(field + strlen(",\"end\":\""), '"') : NULL;
  if (close) memmove(field, close + 1, strlen(close + 1) + 1);
  return 1;
}
/* A schema request answers in `structured_output`; a plain-JSON request
   answers the same object as the text of `result`. */
static void emit_answer(const char *json, int plain) {
  if (!plain) {
    printf("{\"type\":\"result\",\"is_error\":false,\"structured_output\":%s}\n", json);
    return;
  }
  printf("{\"type\":\"result\",\"is_error\":false,\"result\":\"");
  for (const char *at = json; *at; at++) {
    if (*at == '"' || *at == '\\') printf("\\%c", *at);
    else if ((unsigned char)*at < 32) printf("\\u%04x", (unsigned char)*at);
    else putchar(*at);
  }
  puts("\"}");
}
static int provider(int argc, char **argv) {
  if (argc > 2 && strcmp(argv[1], "auth") == 0 && strcmp(argv[2], "status") == 0) {
    puts("{\"loggedIn\":true}");
    return 0;
  }
  size_t len = 0; fixture_count_t n;
  while (len < sizeof prompt - 1 && (n = read(0, prompt + len, sizeof prompt - 1 - len)) > 0) len += (size_t)n;
  prompt[len] = 0;
  char *label = NULL;
  for (char *at = prompt; (at = strstr(at, "HIDE_E2E_LABEL ")) != NULL; at++) label = at;
  /* A draft may quote the label marker in its original assistant context.
     Its title/body schema selects the explicit draft answer instead. */
  int plain;
  const char *requested = request_schema(argc, argv, &plain);
  if (requested && strstr(requested, "\"title\"") && strstr(requested, "\"body\"")) label = NULL;
  if (!label) {
    /* A judgment's input is JSON, where a label cannot stand unescaped: a spec
       that drives one names a file whose one-line answer every such call gets. */
    const char *answer = getenv("HIDE_E2E_PROVIDER_ANSWER");
    FILE *file = answer ? fopen(answer, "rb") : NULL;
    if (file) {
      static char fixed[1 << 16];
      size_t got = fread(fixed, 1, sizeof fixed - 1, file);
      fclose(file);
      fixed[got] = 0;
      char *line = strchr(fixed, '\n');
      if (line) *line = 0;
      if (got > 0) {
        emit_answer(fixed, plain);
        return 0;
      }
    }
    puts("{\"type\":\"result\",\"is_error\":true,\"subtype\":\"error_during_execution\"}");
    return 1;
  }
  char *delay = NULL;
  for (char *at = prompt; (at = strstr(at, "HIDE_E2E_DELAY_MS ")) != NULL && at < label; at++) delay = at;
  if (delay) {
    long ms = atol(delay + strlen("HIDE_E2E_DELAY_MS "));
#ifdef _WIN32
    Sleep((DWORD)ms);
#else
    struct timespec wait = { ms / 1000, (ms % 1000) * 1000000L };
    nanosleep(&wait, NULL);
#endif
  }
  label += strlen("HIDE_E2E_LABEL ");
  char *end = strchr(label, '\n');
  if (end) *end = 0;
  if (!fit_schema(label, argc, argv)) {
    puts("{\"type\":\"result\",\"is_error\":true,\"subtype\":\"error_during_execution\"}");
    return 1;
  }
  const char *calls = getenv("HIDE_E2E_PROVIDER_LOG");
  FILE *log = calls ? fopen(calls, "a") : NULL;
  if (log) { fprintf(log, "%s\n", label); fclose(log); }
  emit_answer(label, plain);
  return 0;
}
// The model list Hide AI reads: one `initialize` control request on stdin, answered with
// the models and no turn (the same shape `hide-ai/tests/fixtures/fake-claude.py` answers).
static int models(void) {
  char sink[4096];
  while (read(0, sink, sizeof sink) > 0) {}
  puts("{\"type\":\"control_response\",\"response\":{\"subtype\":\"success\",\"request_id\":\"hide-models\","
       "\"response\":{\"models\":[{\"value\":\"default\",\"resolvedModel\":\"claude-opus-5-5\"},"
       "{\"value\":\"opus\",\"resolvedModel\":\"claude-opus-5-5\"},"
       "{\"value\":\"sonnet\",\"resolvedModel\":\"claude-sonnet-5-5\"}]}}}");
  return 0;
}
// Specs copy this shim as `codex`. The pinned Herdr's `agent start --kind
// codex` succeeds only once the screen shows Codex's startup composer, so the
// copy draws that line.
static int run_as_codex(const char *argv0) {
  const char *base = argv0;
  for (const char *at = argv0; *at; at++) {
    if (*at == '/' || *at == '\\') base = at + 1;
  }
  return strncmp(base, "codex", 5) == 0;
}

#include "live-check.h"

// A Factory question fixture needs the existing owned-command runner's native
// session report and sentinel command. Provider/auth/model requests above keep
// using this shim; only an explicitly configured interactive pane hands off.
static int factory_question_runner(void) {
  const char *root = getenv("HIDE_E2E_ROOT");
  if (!root) return 0;
  char filename[4096], runner[4096];
  if (snprintf(filename, sizeof filename, "%s/factory-question-runner.config", root) >= (int)sizeof filename) return 1;
  FILE *file = fopen(filename, "rb");
  if (!file) return errno == ENOENT ? 0 : 1;
  size_t count = fread(runner, 1, sizeof runner - 1, file);
  int complete = !ferror(file) && feof(file);
  fclose(file);
  runner[count] = 0;
  if (!complete || !count || strchr(runner, '\n') || strchr(runner, '\r')) return 1;
#ifdef _WIN32
  WCHAR executable[4096];
  if (!MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, runner, -1, executable, 4096)) return 1;
  const WCHAR *arguments[] = { executable, NULL };
  _wexecv(executable, arguments);
#else
  char *arguments[] = { runner, NULL };
  execv(runner, arguments);
#endif
  fprintf(stderr, "fixture question runner failed: %s\n", strerror(errno));
  return 1;
}

static int copy_seed(const char *seed, const char *destination) {
  FILE *file = fopen(seed, "rb");
  if (!file) return 1;
  static char records[65536];
  size_t size = fread(records, 1, sizeof records, file);
  int complete = !ferror(file) && feof(file);
  fclose(file);
  if (!complete || !size) return 1;
  file = fopen(destination, "wb");
  if (!file) return 1;
  complete = fwrite(records, 1, size, file) == size;
  return fclose(file) != 0 || !complete;
}

/* The native-file fixture writes its own session, rather than having
   the spec plant an already-readable transcript. The private root supplies
   a bounded seed and destination; resumed launches preserve the same file.
   Grok's session is a folder: its summary seed lands beside the conversation.
   Without that configuration the binary is a plain stand-in, as other
   specs (the Grok and Cursor hook spec) launch it. */
static int native_session(int argc, char **argv) {
  const char *base = argv[0];
  for (const char *at = argv[0]; *at; at++) if (*at == '/' || *at == '\\') base = at + 1;
  const char *kind, *resume_flag;
  if (strcmp(base, "pi") == 0 || strcmp(base, "pi.exe") == 0) { kind = "pi"; resume_flag = "--session"; }
  else if (strcmp(base, "omp") == 0 || strcmp(base, "omp.exe") == 0) { kind = "omp"; resume_flag = "--resume"; }
  else if (strcmp(base, "grok") == 0 || strcmp(base, "grok.exe") == 0) { kind = "grok"; resume_flag = "--resume"; }
  else if (strcmp(base, "cursor-agent") == 0 || strcmp(base, "cursor-agent.exe") == 0) { kind = "cursor"; resume_flag = "--resume"; }
  else return 0;
  const char *root = getenv("HIDE_E2E_ROOT");
  if (!root) return 0;
  char config[4096], destination[4096], seed[4096], launches[4096];
  if (snprintf(config, sizeof config, "%s/%s-session-path.config", root, kind) >= (int)sizeof config ||
      snprintf(seed, sizeof seed, "%s/%s-session-seed.%s", root, kind, strcmp(kind, "cursor") == 0 ? "db" : "jsonl") >= (int)sizeof seed ||
      snprintf(launches, sizeof launches, "%s/%s-launches.jsonl", root, kind) >= (int)sizeof launches) return 1;
  FILE *file = fopen(config, "rb");
  if (!file) return errno == ENOENT ? 0 : 1;
  size_t size = fread(destination, 1, sizeof destination - 1, file);
  int complete = !ferror(file) && feof(file);
  fclose(file);
  destination[size] = 0;
  if (!complete || !size || strchr(destination, '\n') || strchr(destination, '\r')) return 1;
  int resume = argc == 3 && strcmp(argv[1], resume_flag) == 0;
  if (resume) {
    file = fopen(destination, "rb");
    if (!file) return 1;
    fclose(file);
  } else {
    if (copy_seed(seed, destination)) return 1;
    if (strcmp(kind, "grok") == 0 || strcmp(kind, "cursor") == 0) {
      char summary_seed[4096], summary[4096];
      const char *folder_end = strrchr(destination, '/'), *native_end = strrchr(destination, '\\');
      if (native_end && (!folder_end || native_end > folder_end)) folder_end = native_end;
      if (!folder_end) return 1;
      const char *sidecar = strcmp(kind, "cursor") == 0 ? "meta" : "summary";
      if (snprintf(summary_seed, sizeof summary_seed, "%s/%s-%s-seed.json", root, kind, sidecar) >= (int)sizeof summary_seed ||
          snprintf(summary, sizeof summary, "%.*s/%s.json", (int)(folder_end - destination), destination, sidecar) >= (int)sizeof summary) return 1;
      if (copy_seed(summary_seed, summary)) return 1;
    }
  }
  file = fopen(launches, "ab");
  if (!file) return 1;
  fputc('[', file);
  for (int i = 1; i < argc; i++) {
    if (i > 1) fputc(',', file);
    fputc('"', file);
    for (const unsigned char *at = (const unsigned char *)argv[i]; *at; at++) {
      if (*at == '"' || *at == '\\') fputc('\\', file);
      if (*at < 32) fprintf(file, "\\u%04x", *at); else fputc(*at, file);
    }
    fputc('"', file);
  }
  fputs("]\n", file);
  if (fclose(file) != 0) return 1;
  printf("%s fixture ready\n", kind);
  return 0;
}

static int has_option(int argc, char **argv, const char *name) {
  for (int i = 1; i < argc; i++) if (strcmp(argv[i], name) == 0) return 1;
  return 0;
}

int main(int argc, char **argv) {
#ifdef _WIN32
  _setmode(0, _O_BINARY);
  _setmode(1, _O_BINARY);
#endif
  if (getenv("HIDE_E2E_LIVE_CHECK")) return live_check(argc, argv);
  for (int i = 1; i < argc; i++) {
    if (strcmp(argv[i], "--json-schema") == 0 || (i == 1 && strcmp(argv[i], "auth") == 0)) return provider(argc, argv);
    if (i == 1 && strcmp(argv[i], "-p") == 0 && has_option(argc, argv, "--system-prompt")) return provider(argc, argv);
    if (strcmp(argv[i], "--input-format") == 0) return models();
  }
  if (getenv("HIDE_E2E_JOURNEY")) return journey(argc, argv);
  if (native_session(argc, argv) != 0) return 1;
  if (factory_question_runner() != 0) return 1;
  const char *log_path = getenv("HIDE_E2E_INPUT_LOG");
  int flags = O_WRONLY | O_CREAT | O_APPEND;
#ifdef _WIN32
  flags |= _O_BINARY;
#endif
  int log = log_path ? open(log_path, flags, 0644) : -1;
  raw_terminal();
  if (run_as_codex(argv[0])) {
    static const char composer[] = "\xe2\x80\xba Ask Codex to do anything\r\n";
    if (terminal_write(composer, sizeof composer - 1) != (fixture_count_t)(sizeof composer - 1)) return 1;
  }
#ifdef _WIN32
  // Herdr's encoded PowerShell launch has no visible agent name.
  // Announce from the initialized interactive process before reading input.
  static const char ready[] = "claude fixture ready\r\n";
  if (terminal_write(ready, sizeof ready - 1) != (fixture_count_t)(sizeof ready - 1)) return 1;
  char b[16384];
#else
  char b[4096];
#endif
  fixture_count_t n;
  while ((n = terminal_read(b, sizeof b)) > 0) {
    if (log >= 0 && write(log, b, (size_t)n) < 0) return 1;
#ifdef _WIN32
    // A Unix tty turns Enter's CR into LF on input and LF into CRLF on
    // output, so the line ends. A console delivers CR alone: echo it as the
    // line end too, or the next line is drawn over this one.
    static char echo[sizeof b * 2];
    size_t out = 0;
    for (fixture_count_t i = 0; i < n; i++) {
      echo[out++] = b[i];
      if (b[i] == '\r' && (i + 1 == n || b[i + 1] != '\n')) echo[out++] = '\n';
    }
    if (terminal_write(echo, out) < 0) return 1;
#else
    if (terminal_write(b, (size_t)n) < 0) return 1;
#endif
  }
#ifdef _WIN32
  if (n < 0) return 1;
#endif
  return 0;
}
