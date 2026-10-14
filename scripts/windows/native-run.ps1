# The Windows hardware run, on the machine itself, from an ssh session while a user is logged in
# at the console (the agent runs in that session). Everything lives under -Root; -Cleanup removes
# it, the agent and the test install again.
#
#   native-run.ps1 -Root C:\nd-native            gates and pacing from the prebuilt bin\ folder,
#                                                 then the packaging leg when src\ is there
#   native-run.ps1 -Root C:\nd-native -Cleanup
#
# bin\ holds hello.exe, pacing.exe, ndshell.exe and real_resize_gate.exe; results go to out\.
param([string]$Root = 'C:\nd-native', [switch]$Cleanup, [int]$Runs = 3)
$ErrorActionPreference = 'Continue'
$ProgressPreference = 'SilentlyContinue'
$bin = Join-Path $Root 'bin'
$out = Join-Path $Root 'out'
$agent = Join-Path $env:LOCALAPPDATA 'nd\agent\ndshell.exe'
$demo = Join-Path $env:LOCALAPPDATA 'dev.formalsnake.ndgatem10win'

if ($Cleanup) {
    if (Test-Path "$demo\Update.exe") { & "$demo\Update.exe" --uninstall --silent 2>&1 | Out-Null }
    if (Test-Path $agent) { & $agent agent uninstall 2>&1 | Out-Null }
    Start-Sleep 2
    foreach ($dir in @($Root, (Join-Path $env:LOCALAPPDATA 'nd'), $demo)) {
        if (Test-Path $dir) { Remove-Item -Recurse -Force $dir }
    }
    'cleaned up'
    return
}

New-Item -ItemType Directory -Force $out | Out-Null
function Log($text) { $text | Tee-Object -FilePath (Join-Path $out 'run.log') -Append }
function Agent($json) { & $agent agent call $json | ConvertFrom-Json }

Log "== agent"
& "$bin\ndshell.exe" agent install 2>&1 | ForEach-Object { Log "$_" }
Start-Sleep 3
$ping = Agent '{"cmd":"ping"}'
Log ($ping | ConvertTo-Json -Compress -Depth 5)

Log "== resize gate x$Runs"
$env:ND_REAL = '1'
$env:ND_HELLO_BIN = "$bin\hello.exe"
foreach ($i in 1..$Runs) {
    $env:ND_GATE_OUT = Join-Path $out "gate$i"
    & "$bin\real_resize_gate.exe" 2>&1 | ForEach-Object { Log "$_" }
}

Log "== pacing"
$monitors = @($ping.monitors)
$launch = Agent ('{"cmd":"launch","exe":"' + ("$bin\pacing.exe" -replace '\\', '\\') + '","env":{"ND_PACING":"1"}}')
$a = Agent '{"cmd":"window","title":"pacing-a","timeout_ms":15000}'
$b = Agent '{"cmd":"window","title":"pacing-b","timeout_ms":15000}'
function Place($w, $m) {
    $r = $m.rect
    Agent ('{"cmd":"set_frame","hwnd":' + $w.hwnd + ',"rect":[' + ($r[0] + 100) + ',' + ($r[1] + 100) + ',480,280]}') | Out-Null
}
function Report($label) {
    Start-Sleep 8
    $lines = (Agent ('{"cmd":"output","pid":' + $launch.pid + '}')).lines | Where-Object { $_ -like 'nd-pacing:*' }
    Log "-- $label"
    $lines | Select-Object -Last 4 | ForEach-Object { Log $_ }
}
if ($monitors.Count -ge 2) {
    Place $a $monitors[0]; Place $b $monitors[1]; Report 'a on monitor 0, b on monitor 1'
    Place $a $monitors[1]; Place $b $monitors[0]; Report 'after moving: a on monitor 1, b on monitor 0'
} else {
    Report 'one monitor'
}
Agent ('{"cmd":"kill","pid":' + $launch.pid + '}') | Out-Null

if (Test-Path (Join-Path $Root 'src')) {
    Log "== package windows + signed delta update"
    Push-Location (Join-Path $Root 'src')
    $env:RUSTUP_TOOLCHAIN = '1.95.0-x86_64-pc-windows-msvc'
    $env:CARGO_TARGET_DIR = Join-Path $Root 'target'
    $env:CARGO_BUILD_BUILD_DIR = Join-Path $Root 'target\build'
    $env:Path = "$env:USERPROFILE\.cargo\bin;$env:USERPROFILE\.nd-tools\zig-x86_64-windows-0.16.0;$env:Path"
    cargo test --config "build.rustc-wrapper=''" -p nd-cli --test gate_m10 real_package_windows -- --nocapture 2>&1 |
        ForEach-Object { Log "$_" }
    Pop-Location
}
Log "== done"
