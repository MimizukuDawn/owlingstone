# Starts a built Owlingstone.exe twice, as CI does before every release:
#   1. `--version` must print the version and exit 0;
#   2. the overlay (OWLINGSTONE_SMOKE=1 makes it close by itself after a few seconds) must open and exit 0.
# Usage: ./tools/smoke.ps1 target\release\Owlingstone.exe
param([Parameter(Mandatory)][string]$Exe)
$ErrorActionPreference = "Stop"
$out = Join-Path ([IO.Path]::GetTempPath()) "owlingstone-version.txt"
$p = Start-Process $Exe -ArgumentList "--version" -PassThru -Wait -NoNewWindow -RedirectStandardOutput $out
if ($p.ExitCode -ne 0) { throw "--version exited with code $($p.ExitCode)" }
$v = (Get-Content $out -Raw).Trim()
if ($v -notmatch '^Owlingstone \d+\.\d+\.\d+') { throw "--version printed '$v'" }
$env:OWLINGSTONE_SMOKE = "1"
$p = Start-Process $Exe -PassThru
if (-not $p.WaitForExit(120000)) { $p.Kill(); throw "the overlay did not close by itself" }
if ($p.ExitCode -ne 0) {
  $log = Join-Path $env:OWLINGSTONE_HOME "error.log"
  if (Test-Path $log) { Get-Content $log }
  throw "the overlay exited with code $($p.ExitCode)"
}
"$v started and closed fine; size: {0:N1} MB" -f ((Get-Item $Exe).Length / 1MB)
