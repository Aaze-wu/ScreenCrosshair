$ErrorActionPreference = 'Stop'

$sig = @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public class W {
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr p);
  public delegate bool EnumProc(IntPtr h, IntPtr p);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern int GetWindowLong(IntPtr h, int i);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  public struct RECT { public int L, T, R, B; }
}
'@
Add-Type -TypeDefinition $sig

$proc = Get-Process screencrosshair -ErrorAction SilentlyContinue
if (-not $proc) { Write-Output 'process not running'; exit 1 }
$targetPid = $proc.Id
Write-Output "pid=$targetPid"

$rows = New-Object System.Collections.ArrayList
$cb = [W+EnumProc] {
    param($h, $p)
    $q = 0
    [void][W]::GetWindowThreadProcessId($h, [ref]$q)
    if ($q -eq $targetPid) {
        $sb = New-Object System.Text.StringBuilder 256
        [void][W]::GetClassName($h, $sb, 256)
        $r = New-Object W+RECT
        [void][W]::GetWindowRect($h, [ref]$r)
        $ex = [W]::GetWindowLong($h, -20)
        $st = [W]::GetWindowLong($h, -16)
        $vis = [W]::IsWindowVisible($h)
        [void]$rows.Add(("cls={0} vis={1} rect=({2},{3})-({4},{5}) size={6}x{7} ex=0x{8:X} style=0x{9:X}" -f `
                    $sb.ToString(), $vis, $r.L, $r.T, $r.R, $r.B, ($r.R - $r.L), ($r.B - $r.T), $ex, $st))
    }
    return $true
}
[void][W]::EnumWindows($cb, [IntPtr]::Zero)
$rows | ForEach-Object { Write-Output $_ }

# 屏幕几何（物理像素）
[void][W]::SetProcessDPIAware()
Add-Type -AssemblyName System.Windows.Forms
$vs = [System.Windows.Forms.SystemInformation]::VirtualScreen
Write-Output ("VirtualScreen=({0},{1}) {2}x{3}" -f $vs.Left, $vs.Top, $vs.Width, $vs.Height)
foreach ($s in [System.Windows.Forms.Screen]::AllScreens) {
    Write-Output ("Screen {0} primary={1} bounds={2}" -f $s.DeviceName, $s.Primary, $s.Bounds)
}

# 中心区域像素采样
Add-Type -AssemblyName System.Drawing
$bmp = New-Object System.Drawing.Bitmap $vs.Width, $vs.Height
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($vs.Left, $vs.Top, 0, 0, $bmp.Size)
$g.Dispose()
$out = Join-Path $env:TEMP 'sc_shot2.png'
$bmp.Save($out, [System.Drawing.Imaging.ImageFormat]::Png)
Write-Output "saved $out"

$cx = [int]($bmp.Width / 2)
$cy = [int]($bmp.Height / 2)
$hits = 0
$minX = 999999; $maxX = -1; $minY = 999999; $maxY = -1
for ($y = 0; $y -lt $bmp.Height; $y++) {
    for ($x = 0; $x -lt $bmp.Width; $x++) {
        $p = $bmp.GetPixel($x, $y)
        if ($p.G -gt 150 -and $p.R -lt 120 -and $p.B -lt 120) {
            $hits++
            if ($x -lt $minX) { $minX = $x }
            if ($x -gt $maxX) { $maxX = $x }
            if ($y -lt $minY) { $minY = $y }
            if ($y -gt $maxY) { $maxY = $y }
        }
    }
}
$bmp.Dispose()
Write-Output "screenshot=$($vs.Width)x$($vs.Height) center=($cx,$cy) greenHits=$hits bbox=($minX,$minY)-($maxX,$maxY)"

# 中心 200x200 裁剪
$img = [System.Drawing.Image]::FromFile($out)
$w = 200; $h = 200
$x0 = [int](($img.Width - $w) / 2); $y0 = [int](($img.Height - $h) / 2)
$crop = New-Object System.Drawing.Bitmap $w, $h
$gc = [System.Drawing.Graphics]::FromImage($crop)
$gc.DrawImage($img, (New-Object System.Drawing.Rectangle 0, 0, $w, $h), (New-Object System.Drawing.Rectangle $x0, $y0, $w, $h), [System.Drawing.GraphicsUnit]::Pixel)
$gc.Dispose(); $img.Dispose()
$cropOut = Join-Path $env:TEMP 'sc_center2.png'
$crop.Save($cropOut, [System.Drawing.Imaging.ImageFormat]::Png)
$crop.Dispose()
Write-Output "cropped $cropOut"
