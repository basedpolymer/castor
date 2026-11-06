<#
.SYNOPSIS
  Capture the Castor window of a process to a PNG (deterministic screenshots).
.EXAMPLE
  $p = Start-Process target-d\debug\castor.exe -ArgumentList '--page','tools','--theme','light','--size','1280x800' -PassThru
  .\crates\castor-app\tools\shot.ps1 -ProcessId $p.Id -Out target-d\shots\home-light.png -Wait 4
  Stop-Process -Id $p.Id
#>
param(
  [Parameter(Mandatory = $true)][int]$ProcessId,
  [Parameter(Mandatory = $true)][string]$Out,
  [double]$Wait = 0
)
$ErrorActionPreference = 'Stop'
if ($Wait -gt 0) { Start-Sleep -Milliseconds ([int]($Wait * 1000)) }

Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
using System.Collections.Generic;
public static class CastorShot {
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
  public delegate bool EnumProc(IntPtr hWnd, IntPtr lParam);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr lParam);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint pid);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr ctx);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int cmd);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr hWnd, IntPtr hdc, uint flags);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetClassName(IntPtr hWnd, System.Text.StringBuilder s, int n);
  [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr hWnd, int attr, out RECT r, int size);
  public static List<IntPtr> Windows(uint pid) {
    var list = new List<IntPtr>();
    EnumWindows((h, l) => {
      uint p; GetWindowThreadProcessId(h, out p);
      if (p == pid && IsWindowVisible(h)) {
        var sb = new System.Text.StringBuilder(256); GetClassName(h, sb, 256);
        var c = sb.ToString();
        // Skip the console of debug builds.
        if (c != "ConsoleWindowClass" && c != "CASCADIA_HOSTING_WINDOW_CLASS" && c != "PseudoConsoleWindow") list.Add(h);
      }
      return true;
    }, IntPtr.Zero);
    return list;
  }
}
"@
# Physical pixels: per-monitor-aware v2 for this thread (works even if the process is not).
[CastorShot]::SetThreadDpiAwarenessContext([IntPtr](-4)) | Out-Null

$best = $null; $bestArea = 0; $bestRect = $null
# Wait (up to 30 s) for the app window to appear.
for ($i = 0; $i -lt 60 -and ([CastorShot]::Windows([uint32]$ProcessId)).Count -eq 0; $i++) { Start-Sleep -Milliseconds 500 }
if ($Wait -gt 0) { Start-Sleep -Milliseconds 1500 }
foreach ($h in [CastorShot]::Windows([uint32]$ProcessId)) {
  $r = New-Object CastorShot+RECT
  # DWMWA_EXTENDED_FRAME_BOUNDS = 9: the visible frame, without the invisible resize borders.
  [CastorShot]::DwmGetWindowAttribute($h, 9, [ref]$r, [System.Runtime.InteropServices.Marshal]::SizeOf($r)) | Out-Null
  $area = ($r.Right - $r.Left) * ($r.Bottom - $r.Top)
  if ($area -gt $bestArea) { $best = $h; $bestArea = $area; $bestRect = $r }
}
if (-not $best) { throw "No visible window for process $ProcessId" }

$w = $bestRect.Right - $bestRect.Left; $hgt = $bestRect.Bottom - $bestRect.Top
$bmp = New-Object System.Drawing.Bitmap $w, $hgt
$g = [System.Drawing.Graphics]::FromImage($bmp)
# GPUI draws with DirectX: PrintWindow returns black, so bring the window up and copy the screen.
[CastorShot]::ShowWindow($best, 9) | Out-Null
[CastorShot]::SetForegroundWindow($best) | Out-Null
Start-Sleep -Milliseconds 400
$g.CopyFromScreen($bestRect.Left, $bestRect.Top, 0, 0, (New-Object System.Drawing.Size $w, $hgt))
$dir = Split-Path -Parent $Out
if ($dir -and -not (Test-Path $dir)) { New-Item -ItemType Directory -Force $dir | Out-Null }
$bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $bmp.Dispose()
Write-Output "$Out ($w x $hgt)"
