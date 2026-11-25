<#
.SYNOPSIS
    全屏扫描「准星颜色」的像素，报告质心 / 包围盒 / 与屏幕几何中心的偏差。

.DESCRIPTION
    比 snap.ps1 更能定位问题：snap 只看中心 320x320 的「亮像素」，被桌面内容
    干扰后无法区分「准星在正中」与「准星根本没画」。这里按颜色筛（默认红系），
    并在整块虚拟屏幕上找，所以能发现「画了但位置不对」。

.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\findcross.ps1
    powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\findcross.ps1 -Red 0 -Green 255 -Blue 0
#>
param(
    [int]$Red = 255,
    [int]$Green = 59,
    [int]$Blue = 48,
    [int]$Tol = 90,
    [string]$Tag = 'find'
)
$ErrorActionPreference = 'Stop'

$sig = @'
using System;
using System.Runtime.InteropServices;
public class ScreenCrosshairFind {
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
}
'@
Add-Type -TypeDefinition $sig
[void][ScreenCrosshairFind]::SetProcessDPIAware()
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms

$vs = [System.Windows.Forms.SystemInformation]::VirtualScreen
$bmp = New-Object System.Drawing.Bitmap $vs.Width, $vs.Height
$gr = [System.Drawing.Graphics]::FromImage($bmp)
$gr.CopyFromScreen($vs.Left, $vs.Top, 0, 0, $bmp.Size)
$gr.Dispose()

# 锁定位图数据后逐像素读，GetPixel 在 400 万像素上要几十秒。
$rect = New-Object System.Drawing.Rectangle 0, 0, $bmp.Width, $bmp.Height
$data = $bmp.LockBits($rect, [System.Drawing.Imaging.ImageLockMode]::ReadOnly,
    [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
$stride = $data.Stride
$bytes = New-Object byte[] ($stride * $bmp.Height)
[System.Runtime.InteropServices.Marshal]::Copy($data.Scan0, $bytes, 0, $bytes.Length)
$bmp.UnlockBits($data)

$hits = 0; $sumX = 0; $sumY = 0
$minX = [int]::MaxValue; $maxX = -1; $minY = [int]::MaxValue; $maxY = -1
for ($y = 0; $y -lt $bmp.Height; $y++) {
    $row = $y * $stride
    for ($x = 0; $x -lt $bmp.Width; $x++) {
        $i = $row + $x * 4
        $pb = $bytes[$i]; $pg = $bytes[$i + 1]; $pr = $bytes[$i + 2]
        if ([Math]::Abs($pr - $Red) -le $Tol -and [Math]::Abs($pg - $Green) -le $Tol -and [Math]::Abs($pb - $Blue) -le $Tol) {
            $hits++; $sumX += $x; $sumY += $y
            if ($x -lt $minX) { $minX = $x }
            if ($x -gt $maxX) { $maxX = $x }
            if ($y -lt $minY) { $minY = $y }
            if ($y -gt $maxY) { $maxY = $y }
        }
    }
}
$bmp.Dispose()

$cx = [int]($vs.Width / 2); $cy = [int]($vs.Height / 2)
if ($hits -eq 0) {
    Write-Output ("{0}: no pixel matches rgb({1},{2},{3})±{4} on {5}x{6}" -f $Tag, $Red, $Green, $Blue, $Tol, $vs.Width, $vs.Height)
    exit 2
}
$gx = [int]($sumX / $hits); $gy = [int]($sumY / $hits)
Write-Output ("{0}: hits={1} centroid=({2},{3}) bbox=({4},{5})-({6},{7}) span={8}x{9} center=({10},{11}) offset=({12},{13})" -f `
        $Tag, $hits, $gx, $gy, $minX, $minY, $maxX, $maxY, ($maxX - $minX + 1), ($maxY - $minY + 1), `
    $cx, $cy, ($gx - $cx), ($gy - $cy))
