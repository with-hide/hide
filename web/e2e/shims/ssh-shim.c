// A stand-in for the system `ssh` where a spec asks Hide where a Host leads
// (`ssh -G`), so the answer is the test's and never the runner's timing.
//
// Hide reads an alias with `ssh -F <config> -G -- <alias>` (hide-node's
// `ssh::hosts`), and the answer is the same `keyword value` lines OpenSSH
// prints. Reading the account's own config would make the spec depend on the
// system's `ssh` finishing inside the three-second deadline the product gives
// it, which a loaded runner has missed (with-hide/hide-private#842); the
// parser and that deadline keep their own Rust tests.
//
// Only a `-G` answer is served. The spec's Host entry says what its address
// is, so this reads the `HostName` and `User` it was given rather than
// inventing them. A config with no such line is the alias the spec always
// writes, whose address is `<alias>.invalid`.
//
// Any other argument is refused: this stands in for the listing and nothing
// else, so a spec that dials a device is told its stand-in does not connect.

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define MAX_PATH_LENGTH 4096
#define MAX_ALIAS_BYTES 255

/** A value with the line end words read on Windows keep: `\r` is not part of it. */
static void trim(char *value) {
  size_t length = strlen(value);
  while (length > 0 && (value[length - 1] == '\r' || value[length - 1] == ' ' || value[length - 1] == '\t')) value[--length] = '\0';
}

/** The last argument is the alias: `ssh ... -G -- <alias>`. */
static int listing_requested(int argc, char **argv) {
  int general = 0;
  for (int at = 1; at < argc; at++) {
    if (strcmp(argv[at], "-G") == 0) general = 1;
  }
  return general;
}

/** The config file a `-F <path>` argument names, or NULL for none. */
static const char *config_file(int argc, char **argv) {
  for (int at = 1; at + 1 < argc; at++) {
    if (strcmp(argv[at], "-F") == 0) return argv[at + 1];
  }
  return NULL;
}

/** The config's `HostName` and `User` for `alias`, if the file names them. */
static void config_answer(const char *path, const char *alias, char *hostname, char *user) {
  FILE *file = fopen(path, "r");
  if (!file) return;
  char line[MAX_PATH_LENGTH];
  int matches = 0;
  while (fgets(line, sizeof line, file)) {
    char keyword[64] = {0};
    char value[MAX_PATH_LENGTH] = {0};
    if (sscanf(line, " %63s %4095[^\n]", keyword, value) != 2) continue;
    if (strcmp(keyword, "Host") == 0) {
      // A `Host` line starts a new block; the alias must be named on it.
      matches = 0;
      char *rest = value;
      char *token = strtok(rest, " \t\r\n");
      while (token) {
        if (strcmp(token, alias) == 0) matches = 1;
        token = strtok(NULL, " \t\r\n");
      }
      continue;
    }
    if (!matches) continue;
    if (strcmp(keyword, "HostName") == 0) {
      trim(value);
      snprintf(hostname, MAX_ALIAS_BYTES, "%s", value);
    } else if (strcmp(keyword, "User") == 0) {
      trim(value);
      snprintf(user, MAX_ALIAS_BYTES, "%s", value);
    }
  }
  fclose(file);
}

int main(int argc, char **argv) {
  if (!listing_requested(argc, argv)) {
    fputs("ssh-shim: this stand-in answers `ssh -G` only\n", stderr);
    return 1;
  }
  const char *alias = argc > 0 ? argv[argc - 1] : "";
  char hostname[MAX_ALIAS_BYTES] = {0};
  char user[MAX_ALIAS_BYTES] = {0};
  const char *config = config_file(argc, argv);
  if (config) config_answer(config, alias, hostname, user);
  if (hostname[0] == '\0') snprintf(hostname, sizeof hostname, "%s.invalid", alias);
  if (user[0] == '\0') snprintf(user, sizeof user, "%s", "e2e");
  // The lines Hide reads, in the shape OpenSSH prints: lowercase keywords.
  printf("host %s\n", alias);
  printf("user %s\n", user);
  printf("hostname %s\n", hostname);
  printf("port %s\n", "22");
  return 0;
}
