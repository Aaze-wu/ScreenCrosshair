param(
    [string]$Tag = 'snap'
)
$ErrorActionPreference = 'Stop'
$sig = @'
using System;
using System.Runtime.InteropServices;
public class S {
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
}
'@
Add-Type -TypeDefinition $sig
[void][S]::SetProcessDPIAware()
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms

$vs = [System.Windows.Forms.SystemInformation]::VirtualScreen
$bmp = New-Object System.Drawing.Bitmap $vs.Width, $vs.Height
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($vs.Left, $vs.Top, 0, 0, $bmp.Size)
$g.Dispose()

$cx = [int]($bmp.Width / 2)
$cy = [int]($bmp.Height / 2)
$box = 160

# 中心区域绿色像素统计（准星范围）
$hits = 0; $minX = 999999; $maxX = -1; $minY = 999999; $maxY = -1
for ($y = $cy - $box; $y -lt $cy + $box; $y++) {
    for ($x = $cx - $box; $x -lt $cx + $box; $x++) {
        $p = $bmp.GetPixel($x, $y)
        if ($p.G -gt 60 -or $p.R -gt 60 -or $p.B -gt 60) {
            $hits++
            if ($x -lt $minX) { $minX = $x }
            if ($x -gt $maxX) { $maxX = $x }
            if ($y -lt $minY) { $minY = $y }
            if ($y -gt $maxY) { $maxY = $y }
        }
    }
}

$w = 200; $h = 200
$x0 = $cx - [int]($w / 2); $y0 = $cy - [int]($h / 2)
$crop = New-Object System.Drawing.Bitmap $w, $h
$gc = [System.Drawing.Graphics]::FromImage($crop)
$gc.DrawImage($bmp, (New-Object System.Drawing.Rectangle 0, 0, $w, $h), (New-Object System.Drawing.Rectangle $x0, $y0, $w, $h), [System.Drawing.GraphicsUnit]::Pixel)
$gc.Dispose()
$cropOut = Join-Path $env:TEMP "sc_$Tag.png"
$crop.Save($cropOut, [System.Drawing.Imaging.ImageFormat]::Png)
$crop.Dispose()
$bmp.Dispose()

if ($hits -gt 0) {
    Write-Output ("{0}: centerInk={1} bbox=({2},{3})-({4},{5}) span={6}x{7} offsetFromCenter=({8},{9}) -> {10}" -f `
            $Tag, $hits, $minX, $minY, $maxX, $maxY, ($maxX - $minX + 1), ($maxY - $minY + 1), `
        ([int](($minX + $maxX) / 2) - $cx), ([int](($minY + $maxY) / 2) - $cy), $cropOut)
} else {
    Write-Output ("{0}: centerInk=0 (nothing drawn near center) -> {1}" -f $Tag, $cropOut)
}
