/* Opt-in behavior of the existing Claude/Codex fixture, never a real model. */
static void live_screen(const char *text) {
  const char clear[] = "\033[2J\033[H";
  terminal_write(clear, sizeof clear - 1);
  terminal_write(text, strlen(text));
}

static int live_check(int argc, char **argv) {
  for (int i = 1; i < argc; i++) {
    if (strcmp(argv[i], "--version") == 0) {
      puts("live-check fixture 1");
      return 0;
    }
  }
  raw_terminal();
  const char *scene = getenv("HIDE_LIVE_CHECK_SCENE");
  if (!scene) return 2;
#ifndef _WIN32
  /* Only this opt-in lane gets a private HOME. Exercise the controller's
     real byte recovery, and let e2e identify the exact child/socket to end. */
  const char *home = getenv("HOME");
  if (!home) return 2;
  char location[4096];
  const int codex = run_as_codex(argv[0]);
  const char *configuration = codex ? ".codex/config.toml" : ".claude.json";
  if (snprintf(location, sizeof location, "%s/%s", home, configuration) >= (int)sizeof location) return 2;
  FILE *config = fopen(location, "wb");
  if (!config) return 2;
  fputs(codex ? "model = \"fixture-temporary\"\n" : "{\"fixture\":\"temporarily-mutated\"}\n", config);
  fclose(config);
  if (snprintf(location, sizeof location, "%s/.live-check-child.json", home) >= (int)sizeof location) return 2;
  FILE *identity = fopen(location, "wb");
  if (!identity) return 2;
  const char *socket = getenv("HERDR_SOCKET_PATH");
  fprintf(identity, "{\"pid\":%ld,\"socket\":\"%s\"}\n", (long)getpid(), socket ? socket : "");
  fclose(identity);
  const char *scenario = getenv("LIVE_CHECK_FIXTURE_CASE");
  if (scenario && strcmp(scenario, "hold") == 0) {
    live_screen("LIVE_FIXTURE_HOLD\r\n");
    char ignored;
    while (terminal_read(&ignored, 1) == 1) {}
    return 0;
  }
#endif
  const char *composer = run_as_codex(argv[0])
    ? "\xe2\x80\xba Ask Codex to do anything\r\n" : "\xe2\x9d\xaf \r\n";
  live_screen(strcmp(scene, "startup") == 0 ? "Do you trust this folder?\r\n1. Trust\r\n2. Cancel\r\n" : composer);
  char line[8192];
  size_t used = 0;
  int reached = strcmp(scene, "startup") == 0;
  char byte;
  while (terminal_read(&byte, 1) == 1) {
    if (byte != '\r' && byte != '\n') {
      if (used + 1 >= sizeof line) return 2;
      line[used++] = byte;
      continue;
    }
    line[used] = 0;
    used = 0;
    if (!reached) {
      reached = 1;
      if (strcmp(scene, "working") == 0) live_screen("Working... Esc to interrupt\r\n");
      else if (strcmp(scene, "shell_approval") == 0) live_screen("Do you want to run this command?\r\n  touch probe-1.txt\r\n1. Yes\r\n2. No\r\n");
      else if (strcmp(scene, "file_approval") == 0) live_screen("Apply these changes?\r\n1. Yes\r\n2. No\r\n");
      else if (strcmp(scene, "question") == 0) live_screen("1. One\r\n2. Two\r\nEnter to select - Esc to cancel\r\n");
      else if (strcmp(scene, "plan_approval") == 0) live_screen("Implement this plan?\r\n  1. Yes, implement this plan\r\n  2. No, stay in Plan mode\r\n");
      else if (strcmp(scene, "model_picker") == 0) live_screen("Select model\r\n1. fixture-small\r\n2. fixture-large\r\n");
      else if (strcmp(scene, "resume_picker") == 0) live_screen("Resume session\r\n1. fixture-previous\r\n");
      else if (strcmp(scene, "mcp_approval") == 0) live_screen("Allow MCP live_probe?\r\n1. Yes\r\n2. No\r\n");
      else live_screen(composer);
      continue;
    }
    if (strcmp(scene, "resume_picker") == 0) live_screen("Resumed session: fixture-previous\r\n");
    else if (strcmp(scene, "model_picker") == 0) live_screen("Selected model: fixture-small\r\n");
    else if (strcmp(scene, "startup") == 0) live_screen("Trust saved: fixture-only\r\n");
    else if (strcmp(scene, "plan_approval") == 0) {
      int file = open("probe-3.txt", O_WRONLY | O_CREAT, 0600);
      if (file >= 0) {
#ifdef _WIN32
        _close(file);
#else
        close(file);
#endif
      }
      live_screen("Approval accepted: fixture plan\r\n");
    } else if (strcmp(scene, "rest") == 0) {
      live_screen(composer);
    } else live_screen("Approval accepted: fixture-only\r\n");
  }
  return 0;
}

/* The agent journey's keyless lane (`scripts/agent-journey-check.py --mode ci
   --fixture-bin`): a Claude or Codex stand-in that runs the hook commands
   Hide's kit installed for it, keeps a session file in its agent's own
   format and answers each turn as the journey's fake model does
   (scripts/agent_journey/fake_model.py, `answer`): with the last JOURNEY
   token its prompt or its prompt hook's context carries outside Project
   Memory's blocks, or else with the one in the Memory index its start hook
   gave it. `HIDE_E2E_JOURNEY` names the run's configuration: tab
   separated lines `shell <program>`, `index <folder>`, `log <file>`, where
   each hook's stderr and exit status go, `app_server <script>`, the
   app-server stand-in `codex app-server` runs, and `hook <event> <command>`.
   Never a real model. */
#ifndef _WIN32
#include <poll.h>
#include <signal.h>
#include <sys/stat.h>
#include <sys/wait.h>

#define JOURNEY_HOOKS 64
#define JOURNEY_OUTPUT (64 * 1024)
#define JOURNEY_BOOT_SECONDS 5
/* A hook that has not ended by then is killed and logged as timed out. */
#define JOURNEY_HOOK_SECONDS 30
static char journey_config[1 << 16];
static const char *journey_shell, *journey_index, *journey_log, *journey_app_server;
static const char *journey_events[JOURNEY_HOOKS], *journey_commands[JOURNEY_HOOKS];
static int journey_hook_count, journey_codex;
static char journey_id[64], journey_file[4096], journey_cwd[4096];
/* What the session's start hooks printed: the guidance and its Memory index. */
static char journey_start[JOURNEY_OUTPUT];

static int journey_load(const char *path) {
  FILE *file = fopen(path, "rb");
  if (!file) return 1;
  size_t size = fread(journey_config, 1, sizeof journey_config - 1, file);
  int complete = !ferror(file) && feof(file);
  fclose(file);
  if (!complete) return 1;
  journey_config[size] = 0;
  for (char *line = journey_config; line && *line;) {
    char *end = strchr(line, '\n');
    if (end) *end = 0;
    char *field = strchr(line, '\t');
    if (field) {
      *field++ = 0;
      if (strcmp(line, "shell") == 0) journey_shell = field;
      else if (strcmp(line, "index") == 0) journey_index = field;
      else if (strcmp(line, "log") == 0) journey_log = field;
      else if (strcmp(line, "app_server") == 0) journey_app_server = field;
      else if (strcmp(line, "hook") == 0) {
        char *command = strchr(field, '\t');
        if (!command || journey_hook_count == JOURNEY_HOOKS) return 1;
        *command++ = 0;
        journey_events[journey_hook_count] = field;
        journey_commands[journey_hook_count++] = command;
      }
    }
    line = end ? end + 1 : NULL;
  }
  return !journey_shell || !journey_index || !journey_log;
}

/* `in` as the inside of a JSON string; 0 when it does not fit. */
static size_t journey_escape(char *out, size_t capacity, const char *in) {
  size_t used = 0;
  for (const unsigned char *at = (const unsigned char *)in; *at; at++) {
    char piece[8];
    int size;
    if (*at == '"' || *at == '\\') size = snprintf(piece, sizeof piece, "\\%c", *at);
    else if (*at < 32) size = snprintf(piece, sizeof piece, "\\u%04x", *at);
    else { piece[0] = (char)*at; piece[1] = 0; size = 1; }
    if (used + (size_t)size >= capacity) return 0;
    memcpy(out + used, piece, (size_t)size);
    used += (size_t)size;
  }
  out[used] = 0;
  return used;
}

static void journey_time(char *out, size_t size, const char *format) {
  time_t now = time(NULL);
  struct tm parts;
  gmtime_r(&now, &parts);
  strftime(out, size, format, &parts);
}

static int journey_folders(char *path) {
  for (char *at = path + 1; *at; at++) {
    if (*at != '/') continue;
    *at = 0;
    int made = mkdir(path, 0700) == 0 || errno == EEXIST;
    *at = '/';
    if (!made) return 1;
  }
  return 0;
}

/* Runs every command installed for `event` with `payload` on its stdin, as
   the agent runs its hooks, and keeps what they printed. */
static long journey_elapsed_ms(const struct timespec *since) {
  struct timespec now;
  clock_gettime(CLOCK_MONOTONIC, &now);
  return (now.tv_sec - since->tv_sec) * 1000L + (now.tv_nsec - since->tv_nsec) / 1000000L;
}

static void journey_hooks(const char *event, const char *payload, char *output, size_t capacity) {
  size_t used = 0;
  output[0] = 0;
  for (int i = 0; i < journey_hook_count; i++) {
    if (strcmp(journey_events[i], event) != 0) continue;
    int in[2], out[2];
    if (pipe(in) != 0) continue;
    if (pipe(out) != 0) { close(in[0]); close(in[1]); continue; }
    FILE *log = fopen(journey_log, "ab");
    if (log) { fprintf(log, "%s hook %d\n", event, i); fflush(log); }
    pid_t child = fork();
    if (child == 0) {
      signal(SIGPIPE, SIG_DFL);
      dup2(in[0], 0);
      dup2(out[1], 1);
      if (log) { dup2(fileno(log), 2); close(fileno(log)); }
      close(in[0]); close(in[1]); close(out[0]); close(out[1]);
      execl(journey_shell, journey_shell, "-c", journey_commands[i], (char *)NULL);
      _exit(127);
    }
    close(in[0]);
    close(out[1]);
    if (child < 0) {
      close(in[1]);
      close(out[0]);
      if (log) { fprintf(log, "%s hook %d fork failed\n", event, i); fclose(log); }
      continue;
    }
    /* The payload is written while the output is read, so a hook that
       answers before it reads its input never blocks on a full pipe. */
    fcntl(in[1], F_SETFL, fcntl(in[1], F_GETFL) | O_NONBLOCK);
    const char *at = payload;
    size_t left = strlen(payload);
    int writing = in[1], reading = out[0], timed_out = 0, failed = 0;
    struct timespec started;
    clock_gettime(CLOCK_MONOTONIC, &started);
    if (!left) { close(writing); writing = -1; }
    char sink[4096];
    while (reading >= 0) {
      long remaining = JOURNEY_HOOK_SECONDS * 1000L - journey_elapsed_ms(&started);
      if (remaining <= 0) { timed_out = 1; break; }
      struct pollfd fds[2] = {{reading, POLLIN, 0}, {writing, POLLOUT, 0}};
      int ready = poll(fds, writing >= 0 ? 2 : 1, (int)remaining);
      if (ready < 0 && errno == EINTR) continue;
      if (ready <= 0) { timed_out = ready == 0; failed = ready < 0; break; }
      if (writing >= 0 && fds[1].revents) {
        ssize_t wrote = -1;
        errno = EPIPE; /* an error or hang-up without room to write */
        if (fds[1].revents & POLLOUT) wrote = write(writing, at, left);
        if (wrote > 0) { at += wrote; left -= (size_t)wrote; }
        else if (wrote < 0 && (errno == EAGAIN || errno == EINTR)) { /* poll again */ }
        else left = 0; /* the hook closed its input */
        if (!left) { close(writing); writing = -1; }
      }
      if (fds[0].revents) {
        int room = used + 1 < capacity;
        ssize_t got = read(reading, room ? output + used : sink, room ? capacity - 1 - used : sizeof sink);
        if (got > 0) { if (room) used += (size_t)got; }
        else if (got < 0 && (errno == EAGAIN || errno == EINTR)) { /* poll again */ }
        else { close(reading); reading = -1; }
      }
    }
    if (writing >= 0) close(writing);
    if (reading >= 0) close(reading);
    output[used] = 0;
    if (timed_out || failed) kill(child, SIGKILL);
    int status = 0;
    while (waitpid(child, &status, 0) < 0 && errno == EINTR) {}
    if (log) {
      if (timed_out) fprintf(log, "%s hook %d timed out after %d s\n", event, i, JOURNEY_HOOK_SECONDS);
      else if (failed) fprintf(log, "%s hook %d poll failed, killed\n", event, i);
      else fprintf(log, "%s hook %d exit %d\n", event, i, WIFEXITED(status) ? WEXITSTATUS(status) : -1);
      fclose(log);
    }
  }
}

static void journey_payload(char *out, size_t size, const char *event, const char *extra) {
  char cwd[8192], file[8192];
  journey_escape(cwd, sizeof cwd, journey_cwd);
  journey_escape(file, sizeof file, journey_file);
  snprintf(out, size, "{\"session_id\":\"%s\",\"transcript_path\":\"%s\",\"cwd\":\"%s\",\"hook_event_name\":\"%s\","
           "\"model\":\"journey-fixture\",\"permission_mode\":\"default\"%s}", journey_id, file, cwd, event, extra);
}

static int journey_record(const char *line) {
  FILE *file = fopen(journey_file, "ab");
  if (!file) return 1;
  int written = fputs(line, file) >= 0 && fputc('\n', file) != EOF;
  return fclose(file) != 0 || !written;
}

static int journey_turns;

/* Codex's turn lifecycle (`task_started` in the default mode, then
   `task_complete`), which Hide reads to know a turn waits for no one, and
   the token count Codex writes for each model response. */
static int journey_lifecycle(const char *kind) {
  if (!journey_codex) return 0;
  static char line[1024];
  char stamp[32];
  journey_time(stamp, sizeof stamp, "%Y-%m-%dT%H:%M:%SZ");
  snprintf(line, sizeof line, "{\"type\":\"event_msg\",\"timestamp\":\"%s\",\"payload\":{\"type\":\"%s\","
           "\"turn_id\":\"journey-turn-%d\"%s}}", stamp, kind, journey_turns,
           strcmp(kind, "task_started") == 0 ? ",\"collaboration_mode_kind\":\"default\"" : "");
  return journey_record(line);
}

static int journey_message(const char *role, const char *text) {
  static char escaped[1 << 17], line[1 << 18];
  char stamp[32];
  if (!journey_escape(escaped, sizeof escaped, text) && *text) return 1;
  journey_time(stamp, sizeof stamp, "%Y-%m-%dT%H:%M:%SZ");
  if (journey_codex)
    snprintf(line, sizeof line, "{\"type\":\"response_item\",\"timestamp\":\"%s\",\"payload\":{\"type\":\"message\","
             "\"role\":\"%s\",\"content\":[{\"type\":\"%s\",\"text\":\"%s\"}]}}",
             stamp, role, strcmp(role, "user") == 0 ? "input_text" : "output_text", escaped);
  else if (strcmp(role, "user") == 0)
    snprintf(line, sizeof line, "{\"type\":\"user\",\"timestamp\":\"%s\",\"sessionId\":\"%s\",\"origin\":{\"kind\":\"human\"},"
             "\"promptSource\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"%s\"}}", stamp, journey_id, escaped);
  else
    snprintf(line, sizeof line, "{\"type\":\"assistant\",\"timestamp\":\"%s\",\"sessionId\":\"%s\",\"message\":{\"id\":\"msg_journey_%d\","
             "\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"%s\"}]}}", stamp, journey_id, journey_turns, escaped);
  return journey_record(line);
}

/* The last JOURNEY-<hex> token in `text`, the answer a token prompt asks for. */
static int journey_token(const char *text, char *token, size_t size) {
  const char *found = NULL;
  for (const char *at = text; (at = strstr(at, "JOURNEY-")) != NULL; at++) found = at;
  if (!found) return 0;
  size_t length = strlen("JOURNEY-");
  while (found[length] && strchr("0123456789ABCDEF", found[length])) length++;
  if (length == strlen("JOURNEY-") || length >= size) return 0;
  memcpy(token, found, length);
  token[length] = 0;
  return 1;
}

/* Removes Project Memory's blocks (`<hide-memory-index ...>` and
   `<hide-memory-context ...>`) from `text`, as the fake model reads a turn:
   a card's token there is not the turn's question. */
static void journey_strip_memory(char *text) {
  static const char *const names[] = {"index", "context"};
  for (int i = 0; i < 2; i++) {
    char open[32], close[32];
    snprintf(open, sizeof open, "<hide-memory-%s", names[i]);
    snprintf(close, sizeof close, "</hide-memory-%s>", names[i]);
    char *start;
    while ((start = strstr(text, open)) != NULL) {
      char *end = strstr(start, close);
      if (!end) { *start = 0; break; }
      end += strlen(close);
      memmove(start, end, strlen(end) + 1);
    }
  }
}

/* The last JOURNEY token inside the last Memory index block of `text`. */
static int journey_index_token(const char *text, char *token, size_t size) {
  static const char open[] = "<hide-memory-index", close[] = "</hide-memory-index>";
  static char block[JOURNEY_OUTPUT];
  const char *start = NULL;
  for (const char *at = text; (at = strstr(at, open)) != NULL; at++) start = at;
  if (!start) return 0;
  const char *end = strstr(start, close);
  size_t length = end ? (size_t)(end - start) : strlen(start);
  if (length >= sizeof block) length = sizeof block - 1;
  memcpy(block, start, length);
  block[length] = 0;
  return journey_token(block, token, size);
}

static void journey_screen(const char *text) {
  live_screen(text);
}

/* Herdr reads both agents' state from the terminal title first, as each CLI
   sets it: a braille spinner while a turn runs, Claude's `✳` and Codex's
   plain title at rest. */
static const char *journey_composer(void) {
  return journey_codex ? "\033]0;codex\007\xe2\x80\xba Ask Codex to do anything\r\n"
                       : "\033]0;\xe2\x9c\xb3 Claude Code\007\r\nClaude Code fixture\r\n\xe2\x9d\xaf \r\n";
}

static int journey_turn(const char *prompt) {
  static char payload[1 << 18], escaped[1 << 17], extra[1 << 17], output[JOURNEY_OUTPUT];
  if (!journey_escape(escaped, sizeof escaped, prompt) && *prompt) return 1;
  journey_screen(journey_codex ? "\033]0;\xe2\xa0\x8b codex\007\xe2\x80\xa2 Working (0s \xe2\x80\xa2 esc to interrupt)\r\n"
                               : "\033]0;\xe2\xa0\x8b Working\007\xe2\x9c\xbb Working\xe2\x80\xa6 (esc to interrupt)\r\n");
  snprintf(extra, sizeof extra, ",\"prompt\":\"%s\"", escaped);
  journey_payload(payload, sizeof payload, "UserPromptSubmit", extra);
  journey_hooks("UserPromptSubmit", payload, output, sizeof output);
  journey_turns++;
  if (journey_lifecycle("task_started") || journey_message("user", prompt)) return 1;
  /* The state reader sees the turn under way before it ends. */
  struct timespec working = {3, 0};
  nanosleep(&working, NULL);
  char token[64];
  journey_strip_memory(output);
  const char *answer = journey_token(output, token, sizeof token) || journey_token(prompt, token, sizeof token)
    || journey_index_token(journey_start, token, sizeof token) ? token : "no token in this turn";
  if (journey_message("assistant", answer)) return 1;
  if (journey_codex && journey_record("{\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\"}}")) return 1;
  if (journey_lifecycle("task_complete")) return 1;
  char screen[256];
  snprintf(screen, sizeof screen, "\xe2\x80\xa2 %s\r\n%s", answer, journey_composer());
  journey_screen(screen);
  journey_escape(escaped, sizeof escaped, answer);
  snprintf(extra, sizeof extra, ",\"stop_hook_active\":false,\"last_assistant_message\":\"%s\"", escaped);
  journey_payload(payload, sizeof payload, "Stop", extra);
  journey_hooks("Stop", payload, output, sizeof output);
  return 0;
}

static int journey_new_id(void) {
  unsigned char bytes[16];
  FILE *random = fopen("/dev/urandom", "rb");
  if (!random) return 1;
  int read_all = fread(bytes, 1, sizeof bytes, random) == sizeof bytes;
  fclose(random);
  if (!read_all) return 1;
  bytes[6] = (unsigned char)((bytes[6] & 0x0f) | 0x40);
  bytes[8] = (unsigned char)((bytes[8] & 0x3f) | 0x80);
  snprintf(journey_id, sizeof journey_id,
           "%02x%02x%02x%02x-%02x%02x-%02x%02x-%02x%02x-%02x%02x%02x%02x%02x%02x",
           bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
           bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]);
  return 0;
}

/* Where a session's file is, kept by id for its resume. */
static int journey_index_path(char *out, size_t size) {
  return snprintf(out, size, "%s/%s", journey_index, journey_id) >= (int)size;
}

static int journey_open_session(int resumed) {
  char index[4096];
  if (journey_index_path(index, sizeof index)) return 1;
  if (resumed) {
    FILE *file = fopen(index, "rb");
    if (!file) return 1;
    size_t size = fread(journey_file, 1, sizeof journey_file - 1, file);
    fclose(file);
    journey_file[size] = 0;
    return !size;
  }
  const char *home = getenv("HOME");
  if (!home) return 1;
  if (journey_codex) {
    const char *codex_home = getenv("CODEX_HOME");
    char day[16], stamp[32];
    journey_time(day, sizeof day, "%Y/%m/%d");
    journey_time(stamp, sizeof stamp, "%Y-%m-%dT%H-%M-%S");
    int size = codex_home
      ? snprintf(journey_file, sizeof journey_file, "%s/sessions/%s/rollout-%s-%s.jsonl", codex_home, day, stamp, journey_id)
      : snprintf(journey_file, sizeof journey_file, "%s/.codex/sessions/%s/rollout-%s-%s.jsonl", home, day, stamp, journey_id);
    if (size >= (int)sizeof journey_file) return 1;
  } else {
    char folder[4096];
    snprintf(folder, sizeof folder, "%s", journey_cwd);
    for (char *at = folder; *at; at++) if (*at == '/') *at = '-';
    if (snprintf(journey_file, sizeof journey_file, "%s/.claude/projects/%s/%s.jsonl", home, folder, journey_id)
        >= (int)sizeof journey_file) return 1;
  }
  if (journey_folders(journey_file) || journey_folders(index)) return 1;
  FILE *file = fopen(index, "wb");
  if (!file) return 1;
  int written = fputs(journey_file, file) >= 0;
  if (fclose(file) != 0 || !written) return 1;
  if (journey_codex) {
    static char cwd[8192], line[16384];
    char stamp[32];
    journey_escape(cwd, sizeof cwd, journey_cwd);
    journey_time(stamp, sizeof stamp, "%Y-%m-%dT%H:%M:%SZ");
    snprintf(line, sizeof line, "{\"type\":\"session_meta\",\"timestamp\":\"%s\",\"payload\":{\"id\":\"%s\",\"cwd\":\"%s\"}}",
             stamp, journey_id, cwd);
    return journey_record(line);
  }
  return 0;
}

static int journey(int argc, char **argv) {
  journey_codex = run_as_codex(argv[0]);
  /* A hook that exits without reading its input must not end the stand-in. */
  signal(SIGPIPE, SIG_IGN);
  for (int i = 1; i < argc; i++) {
    if (strcmp(argv[i], "--version") == 0) { puts("journey fixture 1"); return 0; }
    /* No print mode: Hide AI's own calls get a refusal, not a session. */
    if (strcmp(argv[i], "-p") == 0 || strcmp(argv[i], "--print") == 0 || strcmp(argv[i], "exec") == 0) {
      fputs("journey fixture: no print mode\n", stderr);
      return 2;
    }
  }
  /* The kit reads Codex's features; the stand-in is a Codex with no shared
     daemon, which lists no daemon_auto_start row. */
  if (journey_codex && argc == 3 && strcmp(argv[1], "features") == 0 && strcmp(argv[2], "list") == 0) return 0;
  if (journey_codex && argc > 1 && strcmp(argv[1], "features") == 0) return 2;
  if (journey_load(getenv("HIDE_E2E_JOURNEY"))) { fputs("journey fixture: configuration unreadable\n", stderr); return 2; }
  /* Hide records Codex's folder and hook trust through `codex app-server`;
     the stand-in of hide-agent-hooks's tests answers it. */
  if (journey_codex && argc == 2 && strcmp(argv[1], "app-server") == 0) {
    if (!journey_app_server) return 2;
    execlp("python3", "python3", journey_app_server, "app-server", (char *)NULL);
    return 2;
  }
  if (!getcwd(journey_cwd, sizeof journey_cwd)) return 2;
  /* The product's resume arguments: Claude `--resume <id>`, Codex `resume <id>`. */
  const char *resume = NULL, *prompt = NULL;
  for (int i = 1; i + 1 < argc; i++) {
    if (strcmp(argv[i], journey_codex ? "resume" : "--resume") == 0) resume = argv[i + 1];
  }
  for (int i = 1; i < argc; i++) if (strcmp(argv[i], "--") == 0 && i + 1 < argc) prompt = argv[argc - 1];
  if (resume) {
    if (strlen(resume) >= sizeof journey_id) return 2;
    snprintf(journey_id, sizeof journey_id, "%s", resume);
  } else if (journey_new_id()) return 2;
  if (journey_open_session(resume != NULL)) { fputs("journey fixture: session file unavailable\n", stderr); return 2; }
  raw_terminal();
  journey_screen(journey_composer());
  /* A real CLI takes seconds to start before it runs its start hooks and
     reads its first prompt (Claude Code and Codex took 2 to 4 here). A start
     hook run at once asks Hide before it has connected the pane and gets no
     Memory (`pane_not_connected`), and a turn that ended before Hide learned
     the session would never read working on its row. */
  struct timespec boot = {JOURNEY_BOOT_SECONDS, 0};
  nanosleep(&boot, NULL);
  static char payload[16384];
  journey_payload(payload, sizeof payload, "SessionStart", resume ? ",\"source\":\"resume\"" : ",\"source\":\"startup\"");
  journey_hooks("SessionStart", payload, journey_start, sizeof journey_start);
  if (prompt && !resume && journey_turn(prompt)) return 2;
  static char line[1 << 16];
  size_t used = 0;
  int escape = 0;
  char byte;
  while (terminal_read(&byte, 1) == 1) {
    /* Terminal control sequences (bracketed paste) are not prompt text. */
    if (escape == 1) { escape = byte == '[' ? 2 : 0; continue; }
    if (escape == 2) { if (byte >= 0x40 && byte <= 0x7e) escape = 0; continue; }
    if (byte == 0x1b) { escape = 1; continue; }
    if (byte != '\r' && byte != '\n') {
      if (used + 1 >= sizeof line) return 2;
      line[used++] = byte;
      continue;
    }
    line[used] = 0;
    used = 0;
    if (*line && journey_turn(line)) return 2;
  }
  return 0;
}
#else
static int journey(int argc, char **argv) {
  (void)argc; (void)argv;
  fputs("journey fixture: not built for Windows yet\n", stderr);
  return 2;
}
#endif
