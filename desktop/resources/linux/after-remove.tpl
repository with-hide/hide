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
# the core from ~/.hide/core rather than /opt, stopped and removed, its
# keep-alive marker in the default state folder, and those builds. A step
# that fails leaves the rest to go on; the unit file goes either way, so
# no later login starts the core.
case "$1" in
  remove|purge|0) ;;
  *) exit 0 ;;
esac
getent passwd | while IFS=: read -r user _ _ _ _ home _; do
  units="$home/.config/systemd/user"
  for unit in "$units"/dev.withhide.core*.service; do
    [ -f "$unit" ] || continue
    name=$(basename "$unit")
    systemctl --user -M "$user@" disable --now "$name" >/dev/null 2>&1 || true
    rm -f "$unit" "$units/default.target.wants/$name"
    systemctl --user -M "$user@" daemon-reload >/dev/null 2>&1 || true
    rm -f "$home/.hide/state/keep-alive"
    rm -rf "$home/.hide/core"
  done
done
