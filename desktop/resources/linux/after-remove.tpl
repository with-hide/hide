#!/bin/bash
# hide's deb and rpm after-remove script, from electron-builder's own template
# with the alternative naming the CLI that after-install.tpl registered.

if type update-alternatives >/dev/null 2>&1; then
    update-alternatives --remove '${executable}' '/opt/${sanitizedProductName}/resources/${executable}'
else
    rm -f '/usr/bin/${executable}'
fi

APPARMOR_PROFILE_DEST='/etc/apparmor.d/${executable}'
if [ -f "$APPARMOR_PROFILE_DEST" ]; then
  if apparmor_status --enabled > /dev/null 2>&1; then
    if ! { [ -x '/usr/bin/ischroot' ] && /usr/bin/ischroot; } && hash apparmor_parser 2>/dev/null; then
      apparmor_parser --remove "$APPARMOR_PROFILE_DEST" || true
    fi
  fi
  rm -f "$APPARMOR_PROFILE_DEST"
fi

# A real removal (deb: remove or purge; rpm: the last copy erased), not an
# upgrade, also takes out what a paired machine put in for each account
# (PRD device-connect-iroh D-41): the core's systemd user unit, which runs
# the core from ~/.hide/core rather than /opt, its keep-alive marker in the
# default state folder, and those builds. The account's own manager stops
# the unit; the files go as that account (runuser), so root follows no path
# inside a home, which its owner could point anywhere. Only local accounts
# (/etc/passwd) are read, so no directory-service home is mounted; an
# account from elsewhere takes its own out with `hide always-on remove`. A
# step that fails leaves the rest to go on.
case "$1" in
  remove|purge|0) ;;
  *) exit 0 ;;
esac
while IFS=: read -r user _ _ _ _ home _; do
  [ -n "$user" ] && [ -n "$home" ] || continue
  systemctl --user -M "$user@" stop 'dev.withhide.core*.service' >/dev/null 2>&1 || true
  runuser -u "$user" -- /bin/sh -c '
    units="$1/.config/systemd/user"
    for unit in "$units"/dev.withhide.core*.service; do
      [ -f "$unit" ] || continue
      rm -f "$unit" "$units/default.target.wants/${unit##*/}"
      rm -f "$1/.hide/state/keep-alive"
      rm -rf "$1/.hide/core"
    done
  ' sh "$home" >/dev/null 2>&1 || true
  systemctl --user -M "$user@" daemon-reload >/dev/null 2>&1 || true
done < /etc/passwd
