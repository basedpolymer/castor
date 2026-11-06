<#
.SYNOPSIS
  Castor for Windows x64: build -> bundle helpers -> stage -> check -> Velopack pack.

.DESCRIPTION
  Produces, in the releases folder (default <TargetDir>\releases\win):
    Castor-win-Setup.exe        per-user installer (no admin), Start menu shortcut
    Castor-win-Portable.zip     portable build
    Castor-<ver>-full.nupkg     full update package (+ -delta.nupkg when a previous release is there)
    releases.win.json           the update feed for this channel
  Everything downloaded is pinned in packaging/pins.env and SHA-256-checked; downloads and
  builds of helpers are cached in <TargetDir>\deps. Nothing is published.

  Installed layout (%LOCALAPPDATA%\Castor\current): castor.exe (the app, which also runs
  `castor cli <args>` and the MCP server `castor mcp`), castor-cli.exe (a ~250 KB console
  launcher for `castor cli`, built from crates/castor-launcher), bin\ (ffmpeg, ffprobe,
  whisper-cli + DLLs), assets\fonts\, LICENSE.txt, THIRD_PARTY.md, licenses\.

  Needs: PowerShell 7, Rust (MSVC), Visual Studio 2022 C++ + Windows SDK, LLVM (libclang for the
  MuPDF bindgen step), CMake + git (whisper.cpp), .NET SDK 8+ with `vpk` (same version as the
  velopack crate: `dotnet tool install -g vpk --version <VPK_VERSION from pins.env>`).

.EXAMPLE
  pwsh packaging/windows/build.ps1                          # everything, version from Cargo.toml
  pwsh packaging/windows/build.ps1 -Steps deps,stage,check  # only some steps
  pwsh packaging/windows/build.ps1 -Version 0.9.1-test -Releases D:\feed -SkipCargo
#>
[CmdletBinding(PositionalBinding = $false)]
param(
  # Package version (SemVer 2). Default: [workspace.package] version of the root Cargo.toml.
  [string]$Version,
  # Steps to run, in this order: build, deps, stage, check, pack.
  # (`pwsh -File build.ps1 -Steps a,b` passes one string "a,b": both forms are accepted.)
  [string[]]$Steps = @('build', 'deps', 'stage', 'check', 'pack'),
  # Cargo target dir (relative to the repo root or absolute).
  [string]$TargetDir = $(if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { 'target-g' }),
  # Cargo profile: `release` (thin LTO, fast to build) or `dist` (fat LTO, smaller/faster, slow).
  [string]$CargoProfile = 'release',
  [string]$Channel = 'win',
  # Velopack output folder; keep the previous releases there so deltas get generated.
  [string]$Releases,
  # Markdown release notes shown in the app's update notification.
  [string]$ReleaseNotes,
  [int]$Jobs = 4,
  # Take the binaries from another folder instead of <TargetDir>\<profile> (e.g. a patched test build).
  [string]$BinDir
)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true
$Steps = @($Steps | ForEach-Object { $_ -split ',' } | ForEach-Object { $_.Trim() } | Where-Object { $_ })
foreach ($s in $Steps) { if ($s -notin 'build', 'deps', 'stage', 'check', 'pack') { throw "unknown step '$s' (build, deps, stage, check, pack)" } }
$Root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
Set-Location $Root
if (-not [IO.Path]::IsPathRooted($TargetDir)) { $TargetDir = Join-Path $Root $TargetDir }
$ProfileDir = if ($CargoProfile -eq 'dev') { 'debug' } else { $CargoProfile }
if (-not $BinDir) { $BinDir = Join-Path $TargetDir $ProfileDir }
if (-not $Releases) { $Releases = Join-Path $TargetDir "releases\$Channel" }
$Deps = Join-Path $TargetDir 'deps'
$Stage = Join-Path $TargetDir 'package\win\stage'

function Step([string]$name) { Write-Host "`n=== $name ===" -ForegroundColor Cyan }

# ---------------------------------------------------------------- pins + version
$Pins = @{}
foreach ($line in Get-Content (Join-Path $Root 'packaging\pins.env')) {
  if ($line -match '^\s*([A-Z0-9_]+)=(.*)$') { $Pins[$Matches[1]] = $Matches[2].Trim() }
}
if (-not $Version) {
  $toml = Get-Content -Raw (Join-Path $Root 'Cargo.toml')
  if ($toml -notmatch '(?ms)^\[workspace\.package\].*?^version\s*=\s*"([^"]+)"') { throw 'workspace version not found in Cargo.toml' }
  $Version = $Matches[1]
}
Write-Host "Castor $Version  profile=$CargoProfile  channel=$Channel  target=$TargetDir"

# ---------------------------------------------------------------- helpers
function Get-Verified([string]$Url, [string]$Sha256, [string]$Dest) {
  if ((Test-Path $Dest) -and ((Get-FileHash -Algorithm SHA256 $Dest).Hash -eq $Sha256.ToUpper())) {
    Write-Host "cached: $(Split-Path -Leaf $Dest)"; return
  }
  New-Item -ItemType Directory -Force (Split-Path $Dest) | Out-Null
  Write-Host "download: $Url"
  $part = "$Dest.part"
  Invoke-WebRequest -Uri $Url -OutFile $part -UseBasicParsing
  $got = (Get-FileHash -Algorithm SHA256 $part).Hash
  if ($got -ne $Sha256.ToUpper()) {
    Remove-Item $part -Force
    throw "SHA-256 mismatch for $Url`n expected $Sha256`n got      $got"
  }
  Move-Item -Force $part $Dest
}

function Find-CMake {
  $vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
  if (Test-Path $vswhere) {
    $vs = & $vswhere -latest -products * -property installationPath
    $c = Join-Path $vs 'Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe'
    if (Test-Path $c) { return $c }
  }
  $c = Get-Command cmake -ErrorAction SilentlyContinue
  if ($c) { return $c.Source }
  throw 'CMake not found (install the VS "C++ CMake tools" component or Kitware CMake).'
}

function Find-Dumpbin {
  $vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
  if (-not (Test-Path $vswhere)) { return $null }
  $vs = & $vswhere -latest -products * -property installationPath
  Get-ChildItem (Join-Path $vs 'VC\Tools\MSVC') -Directory | Sort-Object Name -Descending |
    ForEach-Object { Join-Path $_.FullName 'bin\Hostx64\x64\dumpbin.exe' } | Where-Object { Test-Path $_ } | Select-Object -First 1
}

# ---------------------------------------------------------------- 1. cargo build
if ($Steps -contains 'build') {
  Step "build ($CargoProfile)"
  # MuPDF bindgen work-around (same as crates/engine-pdf/env.sh).
  $hdr = (Join-Path $Root 'crates\engine-pdf\bindgen\max_align_t.h') -replace '\\', '/'
  $env:BINDGEN_EXTRA_CLANG_ARGS = "-D__CLANG_MAX_ALIGN_T_DEFINED -include $hdr"
  if (-not $env:LIBCLANG_PATH -and (Test-Path 'C:\Program Files\LLVM\bin')) { $env:LIBCLANG_PATH = 'C:/Program Files/LLVM/bin' }
  # No MSVC whole-program optimisation in MuPDF's C objects (fast links; see env.sh).
  $env:_CL_ = '-GL-'
  $env:CARGO_TARGET_DIR = $TargetDir
  $sw = [Diagnostics.Stopwatch]::StartNew()
  # One binary carries everything: `castor` (GUI), `castor cli …`, `castor mcp`. The launcher
  # (~250 KB, console subsystem) is shipped as castor-cli.exe and runs `castor cli <args>`.
  cargo build --locked --profile $CargoProfile -j $Jobs -p castor-app -p castor-launcher
  Write-Host ("cargo build: {0:n0} s" -f $sw.Elapsed.TotalSeconds)
}

# ---------------------------------------------------------------- 2. helpers (ffmpeg, whisper-cli)
$FfDir = Join-Path $Deps "ffmpeg-$($Pins.FFMPEG_WIN_VERSION)"
$WhSrc = Join-Path $Deps "whisper.cpp-$($Pins.WHISPER_TAG)"
$WhOut = Join-Path $WhSrc 'build\bin\Release'
if ($Steps -contains 'deps') {
  Step 'deps: FFmpeg (BtbN GPL shared)'
  $zip = Join-Path $Deps ("dl\" + (Split-Path -Leaf $Pins.FFMPEG_WIN_URL))
  Get-Verified $Pins.FFMPEG_WIN_URL $Pins.FFMPEG_WIN_SHA256 $zip
  if (-not (Test-Path (Join-Path $FfDir 'bin\ffmpeg.exe'))) {
    # Unpack into a fresh, empty folder (untrusted archive), then flatten the single top folder.
    $tmp = "$FfDir.unpack"
    Remove-Item -Recurse -Force $tmp, $FfDir -ErrorAction SilentlyContinue
    Expand-Archive -Path $zip -DestinationPath $tmp
    $top = Get-ChildItem $tmp -Directory | Select-Object -First 1
    Move-Item $top.FullName $FfDir
    Remove-Item -Recurse -Force $tmp
  }

  Step "deps: whisper.cpp $($Pins.WHISPER_TAG) (whisper-cli, CPU, runtime AVX dispatch)"
  if (-not (Test-Path (Join-Path $WhOut 'whisper-cli.exe'))) {
    if (-not (Test-Path (Join-Path $WhSrc '.git'))) {
      git -c advice.detachedHead=false clone --depth 1 --branch $Pins.WHISPER_TAG $Pins.WHISPER_REPO $WhSrc
    }
    $head = (git -C $WhSrc rev-parse HEAD).Trim()
    if ($head -ne $Pins.WHISPER_COMMIT) { throw "whisper.cpp $($Pins.WHISPER_TAG) is $head, expected $($Pins.WHISPER_COMMIT)" }
    $cmake = Find-CMake
    # Shared ggml so the CPU backend variants (SSE4.2 … AVX-512) are picked at run time;
    # static CRT and no OpenMP (vcomp140.dll) so whisper needs no VC++ redistributable.
    & $cmake -S $WhSrc -B (Join-Path $WhSrc 'build') -G 'Visual Studio 17 2022' -A x64 `
      -DCMAKE_POLICY_DEFAULT_CMP0091=NEW -DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded `
      -DBUILD_SHARED_LIBS=ON -DGGML_NATIVE=OFF -DGGML_BACKEND_DL=ON -DGGML_CPU_ALL_VARIANTS=ON -DGGML_OPENMP=OFF `
      -DWHISPER_BUILD_TESTS=OFF -DWHISPER_BUILD_SERVER=OFF -DWHISPER_SDL2=OFF -DWHISPER_CURL=OFF `
      -DWHISPER_BUILD_EXAMPLES=ON
    & $cmake --build (Join-Path $WhSrc 'build') --config Release -j $Jobs
  }
}

# ---------------------------------------------------------------- 3. stage the app folder
if ($Steps -contains 'stage') {
  Step "stage -> $Stage"
  Remove-Item -Recurse -Force $Stage -ErrorAction SilentlyContinue
  $bin = Join-Path $Stage 'bin'
  $lic = Join-Path $Stage 'licenses'
  $fonts = Join-Path $Stage 'assets\fonts'
  New-Item -ItemType Directory -Force $bin, $lic, $fonts | Out-Null
  # castor.exe = app + CLI + MCP server; castor-cli.exe = the tiny launcher (renamed).
  foreach ($pair in @(@('castor.exe', 'castor.exe'), @('castor-cli-launcher.exe', 'castor-cli.exe'))) {
    $p = Join-Path $BinDir $pair[0]
    if (-not (Test-Path $p)) { throw "missing $p (run the build step)" }
    Copy-Item $p (Join-Path $Stage $pair[1])
  }
  # FFmpeg: ffmpeg + ffprobe and the DLLs they share (no ffplay, headers, import libs, docs).
  Copy-Item (Join-Path $FfDir 'bin\ffmpeg.exe'), (Join-Path $FfDir 'bin\ffprobe.exe') $bin
  Copy-Item (Join-Path $FfDir 'bin\*.dll') $bin
  Copy-Item (Join-Path $FfDir 'LICENSE.txt') (Join-Path $lic 'FFmpeg-LICENSE.txt')
  # whisper-cli + ggml backends.
  Copy-Item (Join-Path $WhOut 'whisper-cli.exe') $bin
  Copy-Item (Join-Path $WhOut 'whisper.dll'), (Join-Path $WhOut 'ggml*.dll') $bin
  Copy-Item (Join-Path $WhSrc 'LICENSE') (Join-Path $lic 'whisper.cpp-LICENSE.txt')
  # Fonts loaded at run time by tools-image (resources_dir()/assets/fonts) + their licence.
  Copy-Item (Join-Path $Root 'assets\fonts\*') $fonts
  # Licences: GPL text, third-party notices (shown from Settings > About).
  Copy-Item (Join-Path $Root 'LICENSE') (Join-Path $Stage 'LICENSE.txt')
  if (Test-Path (Join-Path $Root 'THIRD_PARTY.md')) { Copy-Item (Join-Path $Root 'THIRD_PARTY.md') $Stage }
  else { Write-Warning 'THIRD_PARTY.md missing: run packaging/third-party/generate.sh' }

  $size = (Get-ChildItem -Recurse -File $Stage | Measure-Object Length -Sum).Sum
  Write-Host ("staged: {0:n1} MB" -f ($size / 1MB))
  Get-ChildItem -File $Stage | ForEach-Object { '{0,-20} {1,8:n1} MB' -f $_.Name, ($_.Length / 1MB) } | Write-Host
}

# ---------------------------------------------------------------- 4. checks on the staged folder
if ($Steps -contains 'check') {
  Step 'check (minimal PATH, isolated CASTOR_HOME)'
  $oldPath = $env:PATH; $oldHome = $env:CASTOR_HOME
  $work = Join-Path $TargetDir 'package\win\check'
  Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
  New-Item -ItemType Directory -Force $work | Out-Null
  try {
    # Only Windows itself on PATH: anything the package needs must come from the package.
    $env:PATH = "$env:SystemRoot\System32;$env:SystemRoot;$env:SystemRoot\System32\Wbem"
    $env:CASTOR_HOME = Join-Path $work 'home'
    $cli = Join-Path $Stage 'castor-cli.exe'
    $app = Join-Path $Stage 'castor.exe'
    # castor-cli.exe is the launcher: it must find castor.exe next to it and pass stdio through.
    $n = (& $cli list | Measure-Object -Line).Lines
    Write-Host "castor-cli list (launcher): $n lines"
    if ($n -lt 100) { throw "castor-cli list printed only $n lines" }
    $n2 = (& $app cli list | Measure-Object -Line).Lines
    Write-Host "castor cli list (direct, piped): $n2 lines"
    if ($n2 -ne $n) { throw "castor cli list: $n2 lines, the launcher printed $n" }
    # MCP server over stdio: initialize + tools/list, then EOF ends the server.
    $mcpIn = @(
      '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"castor-package-check","version":"1"}}}',
      '{"jsonrpc":"2.0","method":"notifications/initialized"}',
      '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}'
    )
    # Started the way MCP clients do (CreateProcess with pipes): PowerShell does not pipe stdin
    # into GUI-subsystem programs, so `$mcpIn | & $app mcp` would test nothing.
    $psi = [Diagnostics.ProcessStartInfo]::new($app, 'mcp')
    $psi.UseShellExecute = $false; $psi.RedirectStandardInput = $true; $psi.RedirectStandardOutput = $true; $psi.RedirectStandardError = $true
    $mp = [Diagnostics.Process]::Start($psi)
    $errTask = $mp.StandardError.ReadToEndAsync()
    $mcpIn | ForEach-Object { $mp.StandardInput.WriteLine($_) }
    $mp.StandardInput.Close()
    $mcpOut = $mp.StandardOutput.ReadToEnd() -split "`r?`n" | Where-Object { $_ }
    if (-not $mp.WaitForExit(30000)) { $mp.Kill(); throw 'castor mcp did not exit at end of input' }
    $null = $errTask.Result
    $init = $mcpOut | ForEach-Object { try { $_ | ConvertFrom-Json } catch { } } | Where-Object { $_.id -eq 1 }
    $list = $mcpOut | ForEach-Object { try { $_ | ConvertFrom-Json } catch { } } | Where-Object { $_.id -eq 2 }
    if (-not $init.result.serverInfo) { throw "castor mcp: no initialize result. Output:`n$($mcpOut -join "`n")" }
    Write-Host ("castor mcp: {0} {1}, protocol {2}, {3} MCP tools" -f $init.result.serverInfo.name, $init.result.serverInfo.version, $init.result.protocolVersion, @($list.result.tools).Count)
    & (Join-Path $Stage 'bin\ffmpeg.exe') -hide_banner -loglevel error -f lavfi -i 'sine=frequency=440:duration=2' -y (Join-Path $work 'tone.wav')
    & (Join-Path $Stage 'bin\ffprobe.exe') -hide_banner -loglevel error -show_entries format=duration -of csv=p=0 (Join-Path $work 'tone.wav')
    & (Join-Path $Stage 'bin\whisper-cli.exe') --help *> (Join-Path $work 'whisper-help.txt')
    Write-Host "whisper-cli --help: $((Get-Content (Join-Path $work 'whisper-help.txt') | Measure-Object -Line).Lines) lines"
    # Real tool runs through the bundled helpers / linked engines.
    & $cli run audio.convert (Join-Path $work 'tone.wav') --set format=mp3 --out (Join-Path $work 'out')
    & $cli run files.checksum (Join-Path $work 'tone.wav') --out (Join-Path $work 'out')
    $pdf = Get-ChildItem (Join-Path $Root 'tests\fixtures') -Recurse -Filter *.pdf -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($pdf) { & $cli run pdf.compress $pdf.FullName --out (Join-Path $work 'out') }
    Get-ChildItem (Join-Path $work 'out') | ForEach-Object { '  out: {0} ({1:n0} B)' -f $_.Name, $_.Length } | Write-Host
    # GUI start-up: time until the main window exists (isolated profile = its own instance).
    foreach ($i in 1..3) {
      $sw = [Diagnostics.Stopwatch]::StartNew()
      $p = Start-Process $app -PassThru
      while (-not $p.HasExited -and $p.MainWindowHandle -eq 0 -and $sw.Elapsed.TotalSeconds -lt 30) { Start-Sleep -Milliseconds 20; $p.Refresh() }
      $t = $sw.Elapsed.TotalSeconds
      if ($p.HasExited) { throw "castor.exe exited during start-up (code $($p.ExitCode)); see $work\home" }
      if ($p.MainWindowHandle -eq 0) { Write-Warning 'castor.exe: no main window after 30 s' }
      else { Write-Host ("start-up {0}: main window after {1:n2} s" -f $i, $t) }
      Start-Sleep 2
      $p | Stop-Process -Force; $p.WaitForExit(10000) | Out-Null
    }
  } finally { $env:PATH = $oldPath; $env:CASTOR_HOME = $oldHome }

  $dumpbin = Find-Dumpbin
  if ($dumpbin) {
    $system = '^(KERNEL32|USER32|GDI32|ADVAPI32|SHELL32|OLE32|OLEAUT32|COMDLG32|COMCTL32|SHLWAPI|WS2_32|CRYPT32|BCRYPT|bcryptprimitives|NTDLL|ntdll|SECUR32|IPHLPAPI|USERENV|WINMM|DWMAPI|UXTHEME|IMM32|VERSION|SETUPAPI|POWRPROF|PROPSYS|D3D11|D3D12|DXGI|D3DCOMPILER_47|dcomp|DWrite|d2d1|WINDOWSCODECS|MSIMG32|WININET|WINHTTP|NCRYPT|Normaliz|RPCRT4|CFGMGR32|HID|MF|MFPlat|MFReadWrite|AVRT|PSAPI|DBGHELP|WTSAPI32|WINSPOOL\.DRV|COMBASE|api-ms-win-.*|ext-ms-.*|CRYPTBASE|MSVCRT|ucrtbase|OPENGL32|VCRUNTIME140|VCRUNTIME140_1|MSVCP140|SHCORE|WINTRUST|NETAPI32|MPR|DNSAPI|CREDUI|MSCMS|HHCTRL\.OCX|WLDAP32|DXCORE|TaskSchd|urlmon|ntdll|USP10|AVICAP32|UIAutomationCore|icuuc|gdiplus|DirectML|MSVCP140_1)\.dll$'
    foreach ($f in (Get-ChildItem $Stage -Recurse -Include *.exe, *.dll)) {
      $deps = & $dumpbin /nologo /dependents $f.FullName | Where-Object { $_ -match '^\s+\S+\.(dll|DLL|ocx|drv)\s*$' } | ForEach-Object { $_.Trim() }
      $local = Get-ChildItem (Split-Path $f.FullName) -File | ForEach-Object Name
      $missing = $deps | Where-Object { ($_ -notmatch $system) -and ($local -notcontains $_) }
      $crt = $deps | Where-Object { $_ -match '^(VCRUNTIME140|VCRUNTIME140_1|MSVCP140|MSVCP140_1)\.dll$' }
      if ($missing) { Write-Warning "$($f.Name): not found next to it and not a system DLL: $($missing -join ', ')" }
      if ($crt -and $f.Directory.FullName -eq $Stage) { Write-Host "$($f.Name) needs the VC++ runtime: $($crt -join ', ')" }
      if ($deps -match '(?i)onnxruntime') { Write-Warning "$($f.Name) imports an ONNX Runtime DLL (should be static)" }
      # pyke's static ONNX Runtime is the DirectML build: DirectML.dll comes from System32
      # (Windows 10 2004+ / 11), like icuuc.dll (Windows 10 1903+).
      if ($deps -match '(?i)^DirectML\.dll$') { Write-Host "$($f.Name) imports System32\DirectML.dll (Windows 10 2004 or later)" }
    }
  } else { Write-Warning 'dumpbin not found: DLL import check skipped' }
}

# ---------------------------------------------------------------- 5. Velopack
if ($Steps -contains 'pack') {
  Step "vpk pack $Version -> $Releases"
  $vpkVer = (& vpk --help 2>&1 | Select-String 'Velopack CLI (\S+)').Matches.Groups[1].Value
  if ($vpkVer -ne $Pins.VPK_VERSION) { Write-Warning "vpk is $vpkVer, pins.env says $($Pins.VPK_VERSION)" }
  New-Item -ItemType Directory -Force $Releases | Out-Null
  $vpkArgs = @(
    'pack', '--packId', $Pins.APP_ID, '--packVersion', $Version, '--packDir', $Stage,
    '--mainExe', 'castor.exe', '--packTitle', $Pins.APP_TITLE, '--packAuthors', $Pins.APP_AUTHORS,
    '--icon', (Join-Path $Root 'assets\brand\castor.ico'),
    '--splashImage', (Join-Path $Root 'assets\brand\png\castor-icon-256.png'),
    '--runtime', 'win-x64', '--channel', $Channel, '--outputDir', $Releases,
    '--shortcuts', 'StartMenuRoot',
    # The app needs the VC++ 2015-2022 runtime (Rust MSVC binaries, MuPDF); Setup installs it if missing.
    '--framework', 'vcredist143-x64',
    '--skip-updates'
  )
  if ($ReleaseNotes) { $vpkArgs += @('--releaseNotes', $ReleaseNotes) }
  & vpk @vpkArgs
  Get-ChildItem $Releases -File | Sort-Object LastWriteTime | ForEach-Object { '{0,-40} {1,8:n1} MB' -f $_.Name, ($_.Length / 1MB) } | Write-Host
}
