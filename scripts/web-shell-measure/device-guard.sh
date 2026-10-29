#!/usr/bin/env bash
# Refuses a device measurement that could reach an operator's account, before
# anything dials the device; run.sh and device-herdr.sh source it.
#
# A private sshd still signs in as the account itself, so a `~/` folder
# spelled from any HOME but the private one lands in the account's own: on
# 2026-10-09 a probe with the `~/` defaults uploaded its node into a real
# account's ~/.hide/host-helper. So the private sshd's port
# (MEASURE_DEVICE_PORT) is never 22; the private HOME (MEASURE_DEVICE_HOME)
# lives under /tmp, where no account's own home does; the consent folders are
# absolute and inside it, so none depends on which HOME a session reports;
# and the device's Herdr socket is named under /tmp, so nothing asks for the
# account's default Herdr.
#
# The SSH alias is never read from a caller's configuration: hided's parser
# and OpenSSH read some spellings (`Port=22841`, a tab, `Include`, `Match`)
# differently, so a file this guard accepted could send hided to port 22.
# The guard takes the alias's parts (MEASURE_DEVICE_ALIAS, _HOST, _PORT,
# _USER, _IDENTITY) and writes the one configuration hided reads
# (device_guard_config); this harness's own ssh calls pass the same parts
# with no configuration file. The recorded host key (MEASURE_DEVICE_KNOWN_HOSTS)
# names that host and port only and is the only key file read (no global
# one), so no other sshd's key is trusted.
#
# With the core on the device (MEASURE_SCENARIO=remote-core, PRD
# core-host-node-remote-core B5-B7, B21) nothing is installed there by
# consent; instead a core-role hided runs in the private HOME: its program
# (MEASURE_CORE_PROGRAM) and its state folder (MEASURE_CORE_STATE) are
# absolute paths inside MEASURE_DEVICE_HOME, and its node id
# (MEASURE_CORE_NODE) is a plain name it is started with, never the machine's
# own.

device_guard_refuse() { echo "device guard: $*" >&2; exit 2; }

# An absolute path strictly inside a folder, with no empty, `.` or `..` part.
device_guard_inside() {
  local path=$1 folder=$2 rest part
  [[ "$path" != *[[:cntrl:]]* && "$path" == "$folder"/?* ]] || return 1
  rest=${path#"$folder"/}
  IFS=/ read -r -a parts <<< "$rest"
  [[ "$rest" != */ ]] || return 1
  for part in "${parts[@]}"; do
    [[ -n "$part" && "$part" != . && "$part" != .. ]] || return 1
  done
}

device_guard_under_tmp() {
  device_guard_inside "$1" /tmp || device_guard_inside "$1" /private/tmp
}

[[ -z "${MEASURE_DEVICE_SSH_CONFIG:-}" ]] || device_guard_refuse "MEASURE_DEVICE_SSH_CONFIG is not read; name the alias's parts in MEASURE_DEVICE_HOST, MEASURE_DEVICE_USER and MEASURE_DEVICE_IDENTITY"
device_guard_core=false
[[ "${MEASURE_SCENARIO:-}" != remote-core ]] || device_guard_core=true
device_guard_names=(MEASURE_DEVICE_ALIAS MEASURE_DEVICE_HOST MEASURE_DEVICE_PORT MEASURE_DEVICE_USER MEASURE_DEVICE_IDENTITY MEASURE_DEVICE_KNOWN_HOSTS MEASURE_DEVICE_HOME MEASURE_DEVICE_SOCKET)
if $device_guard_core; then
  device_guard_names+=(MEASURE_CORE_PROGRAM MEASURE_CORE_STATE MEASURE_CORE_NODE)
else
  device_guard_names+=(MEASURE_DEVICE_HELPER_ROOT MEASURE_DEVICE_CLI_DIR)
fi
for name in "${device_guard_names[@]}"; do
  [[ -n "${!name:-}" ]] || device_guard_refuse "the ${MEASURE_SCENARIO:-device} scenario needs $name"
done
# Names that cannot read as an option or split a configuration line.
device_guard_plain=(MEASURE_DEVICE_ALIAS MEASURE_DEVICE_USER)
! $device_guard_core || device_guard_plain+=(MEASURE_CORE_NODE)
for name in "${device_guard_plain[@]}"; do
  [[ "${!name}" =~ ^[A-Za-z0-9][A-Za-z0-9._-]*$ ]] || device_guard_refuse "$name ${!name} is not a plain name"
done
[[ "$MEASURE_DEVICE_HOST" =~ ^[A-Za-z0-9][A-Za-z0-9.:-]*$ ]] || device_guard_refuse "MEASURE_DEVICE_HOST $MEASURE_DEVICE_HOST is not a plain host name or address"
[[ "$MEASURE_DEVICE_PORT" =~ ^[0-9]{1,5}$ ]] && (( 10#$MEASURE_DEVICE_PORT >= 1 && 10#$MEASURE_DEVICE_PORT <= 65535 )) || device_guard_refuse "MEASURE_DEVICE_PORT $MEASURE_DEVICE_PORT is not a port"
(( 10#$MEASURE_DEVICE_PORT != 22 )) || device_guard_refuse "MEASURE_DEVICE_PORT is 22, an account's own sshd"
device_guard_port=$((10#$MEASURE_DEVICE_PORT))
[[ "$MEASURE_DEVICE_IDENTITY" == /* && "$MEASURE_DEVICE_IDENTITY" != *[[:space:]]* && -f "$MEASURE_DEVICE_IDENTITY" ]] || device_guard_refuse "MEASURE_DEVICE_IDENTITY $MEASURE_DEVICE_IDENTITY is not an absolute key file path without spaces"
device_guard_under_tmp "$MEASURE_DEVICE_HOME" || device_guard_refuse "MEASURE_DEVICE_HOME $MEASURE_DEVICE_HOME is not a private path under /tmp"
if $device_guard_core; then
  device_guard_inside_names=(MEASURE_CORE_PROGRAM MEASURE_CORE_STATE)
else
  device_guard_inside_names=(MEASURE_DEVICE_HELPER_ROOT MEASURE_DEVICE_CLI_DIR)
fi
for name in "${device_guard_inside_names[@]}"; do
  device_guard_inside "${!name}" "$MEASURE_DEVICE_HOME" || device_guard_refuse "$name ${!name} is not an absolute path inside MEASURE_DEVICE_HOME"
done
device_guard_under_tmp "$MEASURE_DEVICE_SOCKET" || device_guard_refuse "MEASURE_DEVICE_SOCKET $MEASURE_DEVICE_SOCKET is not a private path under /tmp"
# Every recorded key is for the private sshd's host and port, written out
# (a hashed name cannot be read, a marker line widens trust).
# ssh splits a known-hosts path at spaces into several files.
[[ "$MEASURE_DEVICE_KNOWN_HOSTS" == /* && "$MEASURE_DEVICE_KNOWN_HOSTS" != *[[:space:]]* ]] || device_guard_refuse "MEASURE_DEVICE_KNOWN_HOSTS $MEASURE_DEVICE_KNOWN_HOSTS is not an absolute path without spaces"
[[ -f "$MEASURE_DEVICE_KNOWN_HOSTS" && -r "$MEASURE_DEVICE_KNOWN_HOSTS" ]] || device_guard_refuse "MEASURE_DEVICE_KNOWN_HOSTS $MEASURE_DEVICE_KNOWN_HOSTS is not a readable file"
device_guard_keys=0
while IFS= read -r device_guard_line || [[ -n "$device_guard_line" ]]; do
  [[ -n "${device_guard_line//[[:space:]]/}" && "$device_guard_line" != \#* ]] || continue
  [[ "$device_guard_line" == "[$MEASURE_DEVICE_HOST]:$device_guard_port "* ]] || device_guard_refuse "MEASURE_DEVICE_KNOWN_HOSTS holds a key for another host or port than [$MEASURE_DEVICE_HOST]:$device_guard_port"
  device_guard_keys=$((device_guard_keys + 1))
done < "$MEASURE_DEVICE_KNOWN_HOSTS"
(( device_guard_keys > 0 )) || device_guard_refuse "MEASURE_DEVICE_KNOWN_HOSTS records no key for [$MEASURE_DEVICE_HOST]:$device_guard_port"

# The alias as hided reads it: one setting per line, one space after its
# name, nothing its parser and OpenSSH could read apart.
device_guard_config() {
  printf 'Host %s\n  HostName %s\n  Port %s\n  User %s\n  IdentityFile %s\n  IdentityAgent none\n' \
    "$MEASURE_DEVICE_ALIAS" "$MEASURE_DEVICE_HOST" "$device_guard_port" "$MEASURE_DEVICE_USER" "$MEASURE_DEVICE_IDENTITY"
}

# The same alias for this harness's own ssh, with no configuration file.
device_guard_ssh=(
  -F /dev/null
  -o "HostName=$MEASURE_DEVICE_HOST"
  -o "Port=$device_guard_port"
  -o "User=$MEASURE_DEVICE_USER"
  -o "IdentityFile=$MEASURE_DEVICE_IDENTITY"
  -o IdentitiesOnly=yes
  -o IdentityAgent=none
  -o "UserKnownHostsFile=$MEASURE_DEVICE_KNOWN_HOSTS"
  -o GlobalKnownHostsFile=/dev/null
  -o StrictHostKeyChecking=yes
  -o BatchMode=yes
)
