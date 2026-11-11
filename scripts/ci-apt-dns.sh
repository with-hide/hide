#!/usr/bin/env bash
# Ubuntu package DNS on disposable CI runners (with-hide/hide-private#871).
# Healthy defaults are left alone. The two real package hosts are checked
# within bounded lookups; a failed repair fails the caller, never the tests.
set -euo pipefail

export HIDE_CI_APT_DNS_SERVER=1.1.1.1
export HIDE_CI_APT_DNS_HOSTS='archive.ubuntu.com security.ubuntu.com azure.archive.ubuntu.com'

case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*)
    powershell.exe -NoProfile -Command '
      $ErrorActionPreference = "Stop"
      $hosts = $env:HIDE_CI_APT_DNS_HOSTS.Split(" ")
      function Test-DefaultDns {
        try {
          foreach ($name in $hosts) {
            Resolve-DnsName $name -Type A -DnsOnly -QuickTimeout | Out-Host
          }
          return $true
        } catch {
          Write-Host $_
          return $false
        }
      }
      if (Test-DefaultDns) { Write-Host "Ubuntu package DNS is ready; keeping the resolver"; exit 0 }
      Get-DnsClientServerAddress -AddressFamily IPv4 | Out-Host
      if ($env:RUNNER_ENVIRONMENT -ne "github-hosted") {
        throw "DNS repair is restricted to disposable GitHub-hosted runners"
      }
      $server = $env:HIDE_CI_APT_DNS_SERVER
      foreach ($name in $hosts) {
        Resolve-DnsName $name -Type A -Server $server -DnsOnly -QuickTimeout | Out-Host
      }
      $interfaces = @(Get-NetRoute -DestinationPrefix "0.0.0.0/0" | Select-Object -ExpandProperty InterfaceIndex -Unique)
      if ($interfaces.Count -eq 0) { throw "the runner has no default IPv4 route" }
      foreach ($index in $interfaces) {
        Set-DnsClientServerAddress -InterfaceIndex $index -ServerAddresses $server
      }
      Clear-DnsClientCache
      if (!(Test-DefaultDns)) { throw "Ubuntu package DNS still fails after selecting $server" }
      Write-Host "Ubuntu package DNS repaired with $server"
    '
    ;;
  Linux)
    probe() {
      local name
      for name in $HIDE_CI_APT_DNS_HOSTS; do
        timeout --kill-after=1 5 getent ahostsv4 "$name" || return 1
      done
    }
    diagnostics() {
      cat /etc/resolv.conf >&2
      if [[ -d /run/systemd/system ]]; then
        timeout --kill-after=1 5 resolvectl status >&2 || true
      fi
    }
    if probe; then
      echo 'Ubuntu package DNS is ready; keeping the resolver'
      exit 0
    fi
    diagnostics
    [[ ${RUNNER_ENVIRONMENT:-} == github-hosted ]] || {
      echo 'DNS repair is restricted to disposable GitHub-hosted runners' >&2
      exit 1
    }
    root=()
    (( EUID == 0 )) || root=(sudo)
    if [[ -d /run/systemd/system ]]; then
      # Ask the active owner rather than overwrite its generated stub file.
      links=$(ip -o -4 route show default | awk '{for (i=1; i<NF; i++) if ($i == "dev") print $(i+1)}' | sort -u)
      [[ -n $links ]] || { echo 'the runner has no default IPv4 route' >&2; exit 1; }
      for link in $links; do
        timeout --kill-after=1 10 "${root[@]}" resolvectl dns "$link" "$HIDE_CI_APT_DNS_SERVER"
        timeout --kill-after=1 10 "${root[@]}" resolvectl domain "$link" '~.'
      done
      timeout --kill-after=1 10 "${root[@]}" resolvectl flush-caches
    else
      # The bare Ubuntu package container has no systemd or sudo. Its own
      # resolver file is writable and dies with that container.
      printf 'nameserver %s\n' "$HIDE_CI_APT_DNS_SERVER" | "${root[@]}" tee /etc/resolv.conf >/dev/null
    fi
    if ! probe; then
      diagnostics
      echo "Ubuntu package DNS still fails after selecting $HIDE_CI_APT_DNS_SERVER" >&2
      exit 1
    fi
    echo "Ubuntu package DNS repaired with $HIDE_CI_APT_DNS_SERVER"
    ;;
  *) echo 'Ubuntu package DNS is supported only on the Linux and Windows CI runners' >&2; exit 2 ;;
esac
