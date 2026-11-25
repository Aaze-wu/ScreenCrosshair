param(
    [string]$ClassPart = 'Window Class',
    [int]$Width = 0,
    [int]$Height = 0
)
$ErrorActionPreference = 'Stop'
$sig = @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public class F {
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr p);
  public delegate bool EnumProc(IntPtr h, IntPtr p);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
  [DllImport("user32.dll")] public static extern bool BringWindowToTop(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  public struct RECT { public int L, T, R, B; }
}
'@
Add-Type -TypeDefinition $sig
[void][F]::SetProcessDPIAware()

$proc = Get-Process screencrosshair -ErrorAction SilentlyContinue
if (-not $proc) { Write-Output 'process not running'; exit 1 }
$targetPid = $proc.Id

$found = [IntPtr]::Zero
$rect = New-Object F+RECT
$cb = [F+EnumProc] {
    param($h, $p)
    $q = 0
    [void][F]::GetWindowThreadProcessId($h, [ref]$q)
    if ($q -eq $targetPid) {
        $sb = New-Object System.Text.StringBuilder 256
        [void][F]::GetClassName($h, $sb, 256)
        $name = $sb.ToString()
        if ($name -like "*$ClassPart*") {
            $r = New-Object F+RECT
            [void][F]::GetWindowRect($h, [ref]$r)
            $w = $r.R - $r.L; $ht = $r.B - $r.T
            if ($Width -gt 0 -and ($w -ne $Width -or $ht -ne $Height)) { return $true }
            $script:found = $h
            $script:rect = $r
            return $false
        }
    }
    return $true
}
[void][F]::EnumWindows($cb, [IntPtr]::Zero)

if ($found -eq [IntPtr]::Zero) { Write-Output "no window matching '$ClassPart'"; exit 1 }

# HWND_TOPMOST = -1, SWP_NOMOVE|SWP_NOSIZE|SWP_SHOWWINDOW
[void][F]::ShowWindow($found, 9)   # SW_RESTORE
[void][F]::SetWindowPos($found, [IntPtr](-1), 0, 0, 0, 0, 0x0001 -bor 0x0002 -bor 0x0040)
[void][F]::BringWindowToTop($found)
[void][F]::SetForegroundWindow($found)
Start-Sleep -Milliseconds 400
Write-Output ("focused hwnd=0x{0:X} rect=({1},{2})-({3},{4}) size={5}x{6}" -f $found.ToInt64(), $rect.L, $rect.T, $rect.R, $rect.B, ($rect.R - $rect.L), ($rect.B - $rect.T))
