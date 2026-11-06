<#
.SYNOPSIS
  Local end-to-end test of Castor's Velopack update flow, without publishing anything.

.DESCRIPTION
  1. packs version A and installs it from a local feed folder (per-user, %LOCALAPPDATA%\Castor)
  2. packs version B into the same folder (full + delta package)
  3. starts the installed app with CASTOR_UPDATE_FEED=<feed> and an isolated CASTOR_HOME whose
     settings enable "check" and "install automatically"; waits for the download of B
  4. -Mode quit:    closes the window -> the app applies B on quit -> relaunches, checks B runs
     -Mode restart: leaves the app running so "Restart to update" can be clicked in
                    Settings > Updates; then run the script again with -Verify
  5. -Uninstall removes the test installation.

  The app must honour CASTOR_UPDATE_FEED (crates/castor-app/src/updates.rs). Until it does in
  the main tree, -PatchedBuild builds castor.exe from a copy of the sources under
  <TargetDir>\update-test\src with that one change (the repository is not modified).

.EXAMPLE
  pwsh packaging/windows/update-test.ps1 -PatchedBuild -Mode quit
  pwsh packaging/windows/update-test.ps1 -Verify
  pwsh packaging/windows/update-test.ps1 -Uninstall
#>
[CmdletBinding()]
param(
  [string]$TargetDir = $(if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { 'target-g' }),
  [string]$VersionA = '0.9.0-test',
  [string]$VersionB = '0.9.1-test',
  [ValidateSet('quit', 'restart')][string]$Mode = 'quit',
  [switch]$PatchedBuild,
  [switch]$Verify,
  [switch]$Uninstall,
  [int]$TimeoutSec = 300
)
$ErrorActionPreference = 'Stop'
$Root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
Set-Location $Root
if (-not [IO.Path]::IsPathRooted($TargetDir)) { $TargetDir = Join-Path $Root $TargetDir }
$Work = Join-Path $TargetDir 'update-test'
$Feed = Join-Path $Work 'feed'
$TestHome = Join-Path $Work 'home'
$Install = Join-Path $env:LOCALAPPDATA 'Castor'
$Build = Join-Path $PSScriptRoot 'build.ps1'

function Installed-Version {
  $f = Join-Path $Install 'current\sq.version'
  if (-not (Test-Path $f)) { return $null }
  ([xml](Get-Content -Raw $f)).package.metadata.version
}
function Castor-Processes { Get-Process castor -ErrorAction SilentlyContinue | Where-Object { $_.Path -like "$Install\*" } }

if ($Uninstall) {
  Castor-Processes | Stop-Process -Force
  $upd = Join-Path $Install 'Update.exe'
  # Setup.exe / Update.exe are GUI programs: PowerShell would not wait for them with `&`.
  if (Test-Path $upd) { Start-Process $upd -ArgumentList '--uninstall', '--silent' -Wait }
  Write-Host "installed version now: $(Installed-Version)"
  return
}

if ($Verify) {
  $v = Installed-Version
  $p = Castor-Processes
  Write-Host "installed version: $v"
  Write-Host ("running: " + (($p | ForEach-Object { "$($_.Id) $($_.Path)" }) -join '; '))
  if ($v -ne $VersionB) { throw "expected $VersionB, found $v" }
  Write-Host "OK: Castor runs $VersionB" -ForegroundColor Green
  return
}

# ------------------------------------------------------------------ patched castor.exe (optional)
$BinDir = $null
if ($PatchedBuild) {
  $src = Join-Path $Work 'src'
  Write-Host "=== patched copy of the sources -> $src"
  Remove-Item -Recurse -Force $src -ErrorAction SilentlyContinue
  New-Item -ItemType Directory -Force $src | Out-Null
  foreach ($item in 'Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', 'crates', 'assets', 'locales', 'LICENSE') {
    Copy-Item -Recurse (Join-Path $Root $item) $src
  }
  $upd = Join-Path $src 'crates\castor-app\src\updates.rs'
  $code = Get-Content -Raw $upd
  $old = 'UpdateManager::new(GithubSource::new(REPO, None, false), None, None)'
  $new = 'UpdateManager::new(velopack::sources::AutoSource::new(std::env::var("CASTOR_UPDATE_FEED").ok().filter(|s| !s.trim().is_empty()).as_deref().unwrap_or(REPO)), None, None)'
  if ($code.Contains('CASTOR_UPDATE_FEED')) { Write-Host 'updates.rs already honours CASTOR_UPDATE_FEED' }
  elseif ($code.Contains($old)) { $code.Replace($old, $new) | Set-Content -NoNewline $upd }
  else { throw 'updates.rs changed: cannot apply the CASTOR_UPDATE_FEED patch' }
  $hdr = (Join-Path $src 'crates\engine-pdf\bindgen\max_align_t.h') -replace '\\', '/'
  $env:BINDGEN_EXTRA_CLANG_ARGS = "-D__CLANG_MAX_ALIGN_T_DEFINED -include $hdr"
  if (-not $env:LIBCLANG_PATH) { $env:LIBCLANG_PATH = 'C:/Program Files/LLVM/bin' }
  $env:CARGO_TARGET_DIR = $TargetDir
  cargo build --release -j 4 --manifest-path (Join-Path $src 'Cargo.toml') -p castor-app
  if ($LASTEXITCODE) { throw 'patched build failed' }
  $BinDir = Join-Path $Work 'bin'
  New-Item -ItemType Directory -Force $BinDir | Out-Null
  Copy-Item (Join-Path $TargetDir 'release\castor.exe') $BinDir
  # The castor-cli launcher is unchanged: take it from the regular build.
  Copy-Item (Join-Path $TargetDir 'release\castor-cli-launcher.exe') $BinDir
}
$binArgs = @{}
if ($BinDir) { $binArgs['BinDir'] = $BinDir }

# ------------------------------------------------------------------ A: pack + install
Write-Host "=== pack $VersionA and install it"
Castor-Processes | Stop-Process -Force
Remove-Item -Recurse -Force $Feed, $TestHome -ErrorAction SilentlyContinue
& $Build -Steps stage, pack -Version $VersionA -Releases $Feed -TargetDir $TargetDir @binArgs
$setup = Get-ChildItem $Feed -Filter '*-Setup.exe' | Select-Object -First 1
$sw = [Diagnostics.Stopwatch]::StartNew()
$sp = Start-Process $setup.FullName -ArgumentList '--silent' -Wait -PassThru
Write-Host ("setup exit {0} after {1:n1} s; installed version: {2}" -f $sp.ExitCode, $sw.Elapsed.TotalSeconds, (Installed-Version))
Castor-Processes | Stop-Process -Force   # Setup starts the app after installing
if ((Installed-Version) -ne $VersionA) { throw "install of $VersionA failed" }

# ------------------------------------------------------------------ B: pack (delta)
Write-Host "=== pack $VersionB into the same feed"
$notes = Join-Path $Work 'notes.md'
"## $VersionB`n`n- Local update test." | Set-Content $notes
& $Build -Steps pack -Version $VersionB -Releases $Feed -TargetDir $TargetDir -ReleaseNotes $notes
Get-ChildItem $Feed | ForEach-Object { '  {0,-40} {1,8:n1} MB' -f $_.Name, ($_.Length / 1MB) } | Write-Host

# ------------------------------------------------------------------ run A against the local feed
Write-Host "=== start $VersionA with CASTOR_UPDATE_FEED=$Feed"
New-Item -ItemType Directory -Force (Join-Path $TestHome 'config') | Out-Null
"check_updates = true`nauto_install_updates = true`nkeep_running_in_tray = false`nglobal_hotkey = `"`"`n" | Set-Content (Join-Path $TestHome 'config\settings.toml')
$env:CASTOR_HOME = $TestHome
$env:CASTOR_UPDATE_FEED = $Feed
$env:CASTOR_LOG = 'info,velopack=debug'
$app = Start-Process (Join-Path $Install 'current\castor.exe') -PassThru
$pkgs = Join-Path $Install 'packages'
$deadline = (Get-Date).AddSeconds($TimeoutSec)
# Velopack downloads the delta, rebuilds the full package as *.partial, then renames it: only
# the full .nupkg means "downloaded" (quitting earlier just abandons the download).
$full = "*-$VersionB-full.nupkg"
while (-not (Get-ChildItem $pkgs -Filter $full -ErrorAction SilentlyContinue) -and (Get-Date) -lt $deadline) { Start-Sleep 1 }
$got = Get-ChildItem $pkgs -Filter $full -ErrorAction SilentlyContinue
if (-not $got) { throw "no $VersionB package downloaded within $TimeoutSec s (see $TestHome\data\logs\castor.log)" }
Write-Host "downloaded: $($got.Name -join ', ')"
Start-Sleep 3

if ($Mode -eq 'restart') {
  Write-Host "Now click Settings > Updates > 'Restart to update' in the Castor window (PID $($app.Id)),"
  Write-Host "then run: pwsh packaging/windows/update-test.ps1 -Verify"
  return
}

# ------------------------------------------------------------------ quit -> apply -> relaunch
Write-Host "=== close the window: the update is applied on quit"
$null = $app.CloseMainWindow()
$app.WaitForExit(30000) | Out-Null
$deadline = (Get-Date).AddSeconds(60)
while ((Installed-Version) -ne $VersionB -and (Get-Date) -lt $deadline) { Start-Sleep 1 }
Write-Host "installed version after quit: $(Installed-Version)"
# Update.exe is still finishing (hooks, clean-up) right after the version flips; a launch during
# that window exits at once. Wait for it like a user would.
$deadline = (Get-Date).AddSeconds(60)
while ((Get-Process Update -ErrorAction SilentlyContinue | Where-Object { $_.Path -like "$Install\*" }) -and (Get-Date) -lt $deadline) { Start-Sleep 1 }
$app2 = Start-Process (Join-Path $Install 'current\castor.exe') -PassThru
Start-Sleep 5
Write-Host "relaunched: PID $($app2.Id), alive=$(-not $app2.HasExited)"
if ($app2.HasExited) { throw "the updated app exited at launch (code $($app2.ExitCode))" }
& $PSCommandPath -Verify -VersionB $VersionB -TargetDir $TargetDir
$null = $app2.CloseMainWindow()
