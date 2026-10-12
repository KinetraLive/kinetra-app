; NSIS installer hooks (tauri.conf.json bundle.windows.nsis.installerHooks).
!include WinVer.nsh

; Kinetra needs Windows 10 or later (Rust's Windows target and WebView2 do).
!macro NSIS_HOOK_PREINSTALL
  ${IfNot} ${AtLeastWin10}
    MessageBox MB_ICONSTOP "Kinetra needs Windows 10 or later." /SD IDOK
    Abort
  ${EndIf}
!macroend
