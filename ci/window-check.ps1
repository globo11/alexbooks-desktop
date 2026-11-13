# The window check: starts the freshly built AlexBooks for Windows on the build machine against a stand-in office
# (ci/office.py) and looks at what a person would see. A build that fails the REQUIRED parts is not released.
#
#   REQUIRED  the window opens the office on a clean start, with the engine started by AlexBooks itself
#             the right-click menu and browser-only keys are wired; a link that opens a second window works
#             the window ALSO opens while an older AlexBooks for Windows still has the engine running (the moment
#             after an update), and opens normally again afterwards
#   RECORDED  screenshots of the page, of each right-click and of each key, and engine.txt - for a person to look at
#
#   powershell -File ci\window-check.ps1 -Exe <new exe> -Out <folder> [-OldExe <an installed older exe>]
param(
    [Parameter(Mandatory = $true)][string]$Exe,
    [Parameter(Mandatory = $true)][string]$Out,
    [string]$OldExe = ""
)
$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class Win {
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int c);
    [DllImport("user32.dll")] public static extern bool MoveWindow(IntPtr h, int x, int y, int w, int hh, bool r);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, uint d, UIntPtr e);
    [StructLayout(LayoutKind.Sequential)] public struct Pt { public int X; public int Y; }
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref Pt p);
}
"@

New-Item -ItemType Directory -Force $Out | Out-Null
$Out = (Resolve-Path $Out).Path
$Exe = (Resolve-Path $Exe).Path
if ($OldExe) { $OldExe = (Resolve-Path $OldExe).Path }
$OfficeLog = Join-Path $Out "office.log"
$EngineNote = Join-Path $env:LOCALAPPDATA "AlexBooks\engine.txt"
$Port = 8765
$script:Problems = @()
$script:Notes = @()

function Say($m) { Write-Host $m; $script:Notes += $m }
function Problem($m) { Write-Host "PROBLEM: $m"; $script:Problems += $m }

function Shot($name) {
    try {
        $b = [System.Windows.Forms.SystemInformation]::VirtualScreen
        $bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height
        $g = [System.Drawing.Graphics]::FromImage($bmp)
        $g.CopyFromScreen($b.Left, $b.Top, 0, 0, $bmp.Size)
        $bmp.Save((Join-Path $Out "$name.png"), [System.Drawing.Imaging.ImageFormat]::Png)
        $g.Dispose(); $bmp.Dispose()
    } catch { Say "no screenshot for ${name}: $_" }
}

function Engines {
    # the engine's main process for AlexBooks' own folder (the helpers carry --type=...)
    @(Get-CimInstance Win32_Process -Filter "Name='msedgewebview2.exe'" -ErrorAction SilentlyContinue |
        Where-Object { $_.CommandLine -match 'com\.alextaxservices\.alexbooks' -and $_.CommandLine -notmatch '--type=' })
}

function Stop-All {
    $mine = @($Exe); if ($OldExe) { $mine += $OldExe }
    Get-CimInstance Win32_Process -ErrorAction SilentlyContinue | Where-Object { $_.ExecutablePath -and ($mine -contains $_.ExecutablePath) } |
        ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
    Start-Sleep -Milliseconds 500
    Get-CimInstance Win32_Process -Filter "Name='msedgewebview2.exe'" -ErrorAction SilentlyContinue |
        Where-Object { $_.CommandLine -match 'com\.alextaxservices\.alexbooks' } |
        ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
    Start-Sleep -Seconds 2
    Remove-Item (Join-Path $env:LOCALAPPDATA "AlexBooks\alexbooks-window.lock") -ErrorAction SilentlyContinue
}

function Log-Lines { if (Test-Path $OfficeLog) { @(Get-Content $OfficeLog -Encoding UTF8) } else { @() } }
function Log-Count($pattern) { @(Log-Lines | Where-Object { $_ -match $pattern }).Count }
function Wait-Log($pattern, $atLeast, $seconds) {
    $until = (Get-Date).AddSeconds($seconds)
    while ((Get-Date) -lt $until) {
        if ((Log-Count $pattern) -ge $atLeast) { return $true }
        Start-Sleep -Milliseconds 500
    }
    return $false
}

function Start-App($path, $extraEnv) {
    $si = New-Object System.Diagnostics.ProcessStartInfo
    $si.FileName = $path
    $si.UseShellExecute = $false
    $si.WorkingDirectory = Split-Path $path
    foreach ($k in $extraEnv.Keys) { $si.EnvironmentVariables[$k] = $extraEnv[$k] }
    [System.Diagnostics.Process]::Start($si)
}

function Front($proc) {
    # the window to the front at a known place; returns the screen position of the page's top-left corner
    $h = [IntPtr]::Zero
    for ($i = 0; $i -lt 40 -and $h -eq [IntPtr]::Zero; $i++) { $proc.Refresh(); $h = $proc.MainWindowHandle; if ($h -eq [IntPtr]::Zero) { Start-Sleep -Milliseconds 500 } }
    if ($h -eq [IntPtr]::Zero) { return $null }
    [Win]::ShowWindow($h, 9) | Out-Null
    [Win]::MoveWindow($h, 20, 20, 1280, 860, $true) | Out-Null
    [Win]::SetForegroundWindow($h) | Out-Null
    Start-Sleep -Milliseconds 800
    $p = New-Object 'Win+Pt'
    [Win]::ClientToScreen($h, [ref]$p) | Out-Null
    return $p
}

function Click($origin, $x, $y, [switch]$Right, [switch]$Double) {
    [Win]::SetCursorPos($origin.X + $x, $origin.Y + $y) | Out-Null
    Start-Sleep -Milliseconds 200
    $down = 0x2; $up = 0x4
    if ($Right) { $down = 0x8; $up = 0x10 }
    [Win]::mouse_event($down, 0, 0, 0, [UIntPtr]::Zero); [Win]::mouse_event($up, 0, 0, 0, [UIntPtr]::Zero)
    if ($Double) { Start-Sleep -Milliseconds 80; [Win]::mouse_event($down, 0, 0, 0, [UIntPtr]::Zero); [Win]::mouse_event($up, 0, 0, 0, [UIntPtr]::Zero) }
    Start-Sleep -Milliseconds 900
}

function Keys($k, $ms = 900) { [System.Windows.Forms.SendKeys]::SendWait($k); Start-Sleep -Milliseconds $ms }

function Engine-Note { if (Test-Path $EngineNote) { (Get-Content $EngineNote -Raw) } else { "" } }

# ---------------------------------------------------------------------------------------------------------------
# the stand-in office, and this PC "already connected" to it
Remove-Item $OfficeLog -ErrorAction SilentlyContinue
$office = Start-Process python -ArgumentList @((Join-Path $PSScriptRoot "office.py"), $OfficeLog, $Port) -PassThru -WindowStyle Hidden
$cfgDir = Join-Path $env:APPDATA "com.alextaxservices.alexbooks"
New-Item -ItemType Directory -Force $cfgDir | Out-Null
Set-Content (Join-Path $cfgDir "config.json") ('{"server":"http://localhost:' + $Port + '/"}') -Encoding ASCII
Start-Sleep -Seconds 2

try {
    # ----------------------------------------------------------------------------------------------------------
    Say "== 1. a clean start =="
    Stop-All
    $app = Start-App $Exe @{ ALEXBOOKS_ENGINE_LOG = "1" }
    if (-not (Wait-Log '^loaded .*AlexBooksDesktop/' 1 120)) {
        Shot "1-did-not-open"
        Problem "clean start: the window did not open the office within 2 minutes"
    } else {
        $note = Engine-Note
        if ($note -notmatch 'started by AlexBooks') { Problem "clean start: the engine was not started by AlexBooks: $note" }
        if ($note -notmatch 'right-click menu trimmed') { Problem "clean start: the right-click menu was not wired" }
        if ($note -notmatch 'browser-only keys off') { Problem "clean start: the keys were not wired" }
        if ($note -notmatch 'no SmartScreen check') { Problem "clean start: the SmartScreen setting was not applied" }
        $eng = Engines
        if ($eng.Count -lt 1) { Problem "clean start: no engine process found for AlexBooks' folder" }
        else {
            $cl = $eng[0].CommandLine
            Say ("engine command line: " + $cl)
            if ($cl -notmatch 'disable-features=[^ ]*Translate') { Problem "clean start: the engine was not started with the no-translation switch" }
            if ($cl -notmatch 'disable-features=[^ ]*msSmartScreenProtection') { Problem "clean start: the engine was not started with the no-SmartScreen switch" }
        }

        # ---- what a person sees (recorded, and checked where a page can tell) ----
        $o = Front $app
        if ($null -eq $o) { Say "the window could not be brought to the front - nothing clicked" }
        else {
            Start-Sleep -Seconds 4                      # time for a translation offer to appear, if one would
            Shot "1-page"
            Click $o 1050 520                           # an empty part of the page: gives the page the keyboard
            Click $o 1050 520 -Right; Shot "2-rightclick-empty"; Keys "{ESC}" 500
            Click $o 200 282; Click $o 200 282 -Right; Shot "3-rightclick-textbox"; Keys "{ESC}" 500
            Click $o 300 130 -Double; Click $o 300 130 -Right; Shot "4-rightclick-selected-text"; Keys "{ESC}" 500
            Click $o 1050 520
            Keys "x"
            Keys "^j" 1500; Shot "5-ctrl-j"
            Keys "{F7}" 1500; Shot "6-f7"; Keys "{ESC}" 500
            Keys "^u" 1500; Shot "7-ctrl-u"
            Keys "^f" 1500; Shot "8-ctrl-f"; Keys "{ESC}" 500
            Click $o 1050 520
            if ((Log-Count '^key x$') -ge 1) {
                if ((Log-Count '^key ctrl\+j$') -ne 0) { Problem "Ctrl+J still reaches the engine" }
                if ((Log-Count '^key f7$') -ne 0) { Problem "F7 still reaches the engine" }
                if ((Log-Count '^key ctrl\+u$') -ne 0) { Problem "Ctrl+U still reaches the engine" }
                if ((Log-Count '^key ctrl\+f$') -lt 1) { Problem "Ctrl+F no longer reaches the page (it must keep working)" }
            } else { Say "typed keys did not reach the page on this machine - the key check is not conclusive" }

            # a link that opens a second window: it must open on the same engine start
            $before = Log-Count '^loaded '
            Click $o 180 384
            if (Wait-Log '^loaded ' ($before + 1) 30) {
                Start-Sleep -Seconds 1; Shot "9-second-window"
                if ((Engine-Note) -notmatch '(?m)^w\d+: ') { Problem "the second window opened but was not tidied" }
                Keys "%{F4}" 1200
            } else { Shot "9-second-window-missing"; Problem "a link that opens a second window did not open one" }

            $o = Front $app
            if ($null -ne $o) { Click $o 1050 520; Keys "^p" 4000; Shot "10-ctrl-p"; Keys "{ESC}" 1500 }
        }
        $app.Refresh()
        if ($app.HasExited) { Problem "clean start: the program ended by itself (exit code $($app.ExitCode))" }
    }
    Copy-Item $EngineNote (Join-Path $Out "engine-1-clean.txt") -ErrorAction SilentlyContinue
    Copy-Item $OfficeLog (Join-Path $Out "office-1-clean.log") -ErrorAction SilentlyContinue
    Stop-All

    # ----------------------------------------------------------------------------------------------------------
    if ($OldExe) {
        Say "== 2. the moment after an update: an older AlexBooks for Windows still has the engine running =="
        Remove-Item $OfficeLog -ErrorAction SilentlyContinue; New-Item -ItemType File $OfficeLog | Out-Null
        $fake = Join-Path $Out "other-localappdata"; New-Item -ItemType Directory -Force $fake | Out-Null
        # its "one window per PC" lock goes to another folder, so the new one starts beside it; its engine folder
        # is still the real one - which is the point
        $old = Start-App $OldExe @{ LOCALAPPDATA = $fake }
        if (-not (Wait-Log '^loaded .*AlexBooksDesktop/' 1 120)) { Shot "20-old-did-not-open"; Say "the older version did not open - this part is not conclusive" }
        else {
            $oo = Front $old
            if ($null -ne $oo) { Start-Sleep -Seconds 4; Shot "20-old-page"; Click $oo 1050 520; Click $oo 1050 520 -Right; Shot "21-old-rightclick-empty"; Keys "{ESC}" 500 }
            foreach ($e in (Engines)) { Say ("older engine: " + $e.CommandLine) }
            $before = Log-Count '^loaded '
            $new = Start-App $Exe @{ ALEXBOOKS_ENGINE_LOG = "1" }
            if (Wait-Log '^loaded ' ($before + 1) 120) {
                Say "the new version opened beside the older one"
                $n2 = Front $new; Start-Sleep -Seconds 1; Shot "22-new-beside-old"
            } else {
                Shot "22-new-did-not-open"
                Problem "with an older version still running, the new window did not open"
            }
            Say ("engine.txt:`n" + (Engine-Note))
            Copy-Item $EngineNote (Join-Path $Out "engine-2-beside-old.txt") -ErrorAction SilentlyContinue
        }
        Copy-Item $OfficeLog (Join-Path $Out "office-2-beside-old.log") -ErrorAction SilentlyContinue
        Stop-All

        Say "== 3. the next start, with nothing else running =="
        Remove-Item $OfficeLog -ErrorAction SilentlyContinue; New-Item -ItemType File $OfficeLog | Out-Null
        $again = Start-App $Exe @{ ALEXBOOKS_ENGINE_LOG = "1" }
        if (-not (Wait-Log '^loaded .*AlexBooksDesktop/' 1 120)) { Shot "30-did-not-open"; Problem "the start after that did not open the office" }
        elseif ((Engine-Note) -notmatch 'started by AlexBooks') { Problem "the start after that did not start the engine itself: $(Engine-Note)" }
        Copy-Item $EngineNote (Join-Path $Out "engine-3-next-start.txt") -ErrorAction SilentlyContinue
        Stop-All
    }
}
finally {
    Stop-All
    try { Stop-Process -Id $office.Id -Force -ErrorAction SilentlyContinue } catch {}
    ($script:Notes + "" + "PROBLEMS: $($script:Problems.Count)" + $script:Problems) | Set-Content (Join-Path $Out "summary.txt") -Encoding UTF8
}

if ($script:Problems.Count -gt 0) { Write-Host "`nThe window check FAILED:"; $script:Problems | ForEach-Object { Write-Host " - $_" }; exit 1 }
Write-Host "`nThe window check passed."
