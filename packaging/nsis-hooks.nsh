; Hook installer NSIS untuk Dekat.
;
; Dipakai Tauri lewat `bundle.windows.nsis.installerHooks`.
;
; Dua masalah nyata yang ditangani di sini:
;  1. Install lama bernama "LAN Chat" punya ProductName sendiri, jadi uninstaller
;     baru tidak menyentuh shortcut-nya. Akibatnya Start Menu punya dua entri.
;  2. Windows menyimpan ikon per-path. Karena exe diganti di path yang sama
;     (%LOCALAPPDATA%\Dekat\dekat-desktop.exe), Explorer bisa tetap menampilkan
;     ikon lama ("LC") sampai cache ikon shell disegarkan.

!macro DekatHapusShortcutLama
  Delete "$SMPROGRAMS\LAN Chat.lnk"
  RMDir /r "$SMPROGRAMS\LAN Chat"
  Delete "$DESKTOP\LAN Chat.lnk"
  Delete "$SMPROGRAMS\Dekat.lnk"
  RMDir /r "$SMPROGRAMS\Dekat"
!macroend

!macro DekatSegarkanIkon
  ; -show memaksa shell memuat ulang cache ikon tanpa perlu reboot.
  nsExec::Exec '"$SYSDIR\ie4uinit.exe" -show'
!macroend

!macro NSIS_HOOK_PREINSTALL
  !insertmacro DekatHapusShortcutLama
!macroend

!macro NSIS_HOOK_POSTINSTALL
  !insertmacro DekatSegarkanIkon
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  !insertmacro DekatSegarkanIkon
!macroend
