$ErrorActionPreference = 'Stop'
$sig = @'
using System;
using System.Runtime.InteropServices;
public class P {
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(POINT p);
  [DllImport("user32.dll")] public static extern IntPtr GetAncestor(IntPtr h, uint flags);
  [DllImport("user32.dll")] public static extern int GetClassName(IntPtr h, System.Text.StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern IntPtr GetWindow(IntPtr h, uint cmd);
  public struct POINT { public int X, Y; }
}
'@
Add-Type -TypeDefinition $sig
[void][P]::SetProcessDPIAware()
Add-Type -AssemblyName System.Windows.Forms

$proc = Get-Process screencrosshair -ErrorAction SilentlyContinue
if (-not $proc) { Write-Output 'process not running'; exit 1 }
$targetPid = $proc.Id

$vs = [System.Windows.Forms.SystemInformation]::VirtualScreen
$points = @(
    @{ n = 'screen-center'; x = [int]($vs.Width / 2); y = [int]($vs.Height / 2) },
    @{ n = 'top-left';      x = 12; y = 12 },
    @{ n = 'bottom-right';  x = ($vs.Width - 12); y = ($vs.Height - 12) }
)

foreach ($pt in $points) {
    $p = New-Object P+POINT
    $p.X = $pt.x; $p.Y = $pt.y
    $h = [P]::WindowFromPoint($p)
    $root = [P]::GetAncestor($h, 2)   # GA_ROOT
    $cls = New-Object System.Text.StringBuilder 256
    if ($root -ne [IntPtr]::Zero) { [void][P]::GetClassName($root, $cls, 256) }
    $wpid = 0
    if ($root -ne [IntPtr]::Zero) { [void][P]::GetWindowThreadProcessId($root, [ref]$wpid) }
    $owner = if ($wpid -eq $targetPid) { 'OVERLAY(click-blocked!)' } else { "other(pid=$wpid)" }
    Write-Output ("{0,-14} ({1},{2}) -> hwnd=0x{3:X} class='{4}' {5}" -f $pt.n, $pt.x, $pt.y, $root.ToInt64(), $cls.ToString(), $owner)
}
