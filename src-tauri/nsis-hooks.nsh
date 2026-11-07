; AlexBooks for Windows - installer hooks (1.4.0)
;
; One uninstall for everything. On a PC that is also the office server (C:\AlexBooks), uninstalling
; "AlexBooks" from Windows > Installed apps first runs the server's own uninstaller, elevated (Windows asks
; once): it stops the server, removes the startup task, the icons and the certificate, and deletes the app
; folder. The office's data folder (books, backups, keys) is kept; the log on the Public desktop says where.

!macro NSIS_HOOK_PREUNINSTALL
  IfFileExists "C:\AlexBooks\app\install\uninstall.ps1" 0 done_server
    ExecShellWait "runas" "powershell.exe" '-NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File "C:\AlexBooks\app\install\uninstall.ps1" -Yes' SW_HIDE
  done_server:
!macroend
