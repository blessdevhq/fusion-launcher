; Fusion Launcher NSIS installer hooks.
;
; The libtorrent torrent sidecar (fusion-torrent.exe) is a PyInstaller binary
; that dynamically links the Microsoft Visual C++ runtime. On a clean Windows
; without the VC++ 2015-2022 x64 redistributable, `import libtorrent` fails with
; an ImportError ("DLL load failed") and every download dies with
; "Torrent sidecar exited unexpectedly (code 1)".
;
; This hook installs the redistributable once, silently, when it is missing.
; The installer is bundled as a resource (tauri.sidecar.conf.json ->
; bundle.resources) at $INSTDIR\redist\vc_redist.x64.exe and removed afterwards.
;
; NOTE: vc_redist.x64.exe requires elevation. With a currentUser (non-elevated)
; install this triggers a UAC prompt; if the user declines or has no admin
; rights the runtime stays missing and P2P downloads will not work. Direct HTTP
; downloads and the rest of the app are unaffected.

!macro NSIS_HOOK_POSTINSTALL
  Push $0
  Push $1

  ; The x64 runtime registers in the 64-bit registry view; a 32-bit NSIS
  ; installer must switch views explicitly or it reads WOW6432Node and misses it.
  SetRegView 64
  ReadRegDWORD $1 HKLM "SOFTWARE\Microsoft\VisualStudio\14.0\VC\Runtimes\x64" "Installed"
  SetRegView 32

  StrCmp $1 "1" fl_vcredist_done 0
    IfFileExists "$INSTDIR\redist\vc_redist.x64.exe" 0 fl_vcredist_done
      DetailPrint "Installing Microsoft Visual C++ Redistributable (required for downloads)..."
      ExecWait '"$INSTDIR\redist\vc_redist.x64.exe" /install /quiet /norestart' $0
      DetailPrint "Visual C++ Redistributable installer returned $0."
  fl_vcredist_done:

  ; The redistributable is only needed during install; do not leave it behind.
  Delete "$INSTDIR\redist\vc_redist.x64.exe"
  RMDir "$INSTDIR\redist"

  Pop $1
  Pop $0
!macroend
