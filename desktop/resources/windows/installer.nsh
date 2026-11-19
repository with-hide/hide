; hide's additions to electron-builder's NSIS installer (PRD auto-update D-12,
; device-connect-iroh D-38, D-41).
;
; Removing hide from Settings > Apps stops its daemon, which runs from the
; build's copy under ~\.hide rather than this folder, so the installer's own
; stop of every process under $INSTDIR does not reach it. It then takes out
; what a paired PC put in: the login item (a Task Scheduler task), its
; keep-alive marker and the builds under ~\.hide\core (`hide always-on
; remove`), and the firewall rules for same-network machines, which need an
; administrator, so Windows asks through its permission prompt; declined,
; the rules stay and one line says so. A Herdr server an earlier build left
; running and its panes are not stopped: they may hold the operator's
; agents, and docs/INSTALL.md gives the command that ends them. An update
; runs this uninstaller too (`--updated`), and then nothing is stopped or
; removed: the daemon keeps answering until the new app replaces it.
!macro customUnInstall
  ${ifNot} ${isUpdated}
    nsExec::Exec '"$INSTDIR\resources\hide.exe" stop'
    Pop $0
    nsExec::Exec '"$INSTDIR\resources\hide.exe" always-on remove'
    Pop $0
    nsExec::Exec '"$INSTDIR\resources\hide.exe" always-on firewall ask-remove'
    Pop $0
    ${if} $0 != 0
      MessageBox MB_OK|MB_ICONINFORMATION "Hide's firewall rules for same-network connections were left in place. Remove the rule group Hide under Windows Defender Firewall > Advanced settings > Inbound Rules." /SD IDOK
    ${endIf}
  ${endIf}
!macroend
