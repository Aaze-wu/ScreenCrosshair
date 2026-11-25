param(
    [Parameter(Mandatory = $true)][string]$A,
    [Parameter(Mandatory = $true)][string]$B,
    [int]$Threshold = 24
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$ia = [System.Drawing.Bitmap]::FromFile($A)
$ib = [System.Drawing.Bitmap]::FromFile($B)
if ($ia.Width -ne $ib.Width -or $ia.Height -ne $ib.Height) {
    Write-Output "size mismatch: $($ia.Width)x$($ia.Height) vs $($ib.Width)x$($ib.Height)"
    exit 1
}
$changed = 0
$minX = 999999; $maxX = -1; $minY = 999999; $maxY = -1
for ($y = 0; $y -lt $ia.Height; $y++) {
    for ($x = 0; $x -lt $ia.Width; $x++) {
        $pa = $ia.GetPixel($x, $y); $pb = $ib.GetPixel($x, $y)
        $d = [Math]::Abs($pa.R - $pb.R) + [Math]::Abs($pa.G - $pb.G) + [Math]::Abs($pa.B - $pb.B)
        if ($d -gt $Threshold) {
            $changed++
            if ($x -lt $minX) { $minX = $x }
            if ($x -gt $maxX) { $maxX = $x }
            if ($y -lt $minY) { $minY = $y }
            if ($y -gt $maxY) { $maxY = $y }
        }
    }
}
$total = $ia.Width * $ia.Height
$ia.Dispose(); $ib.Dispose()
if ($changed -eq 0) {
    Write-Output "identical (pixels=$total)"
} else {
    Write-Output ("changed={0}/{1} ({2:N1}%) bbox=({3},{4})-({5},{6}) span={7}x{8}" -f `
            $changed, $total, (100.0 * $changed / $total), $minX, $minY, $maxX, $maxY, ($maxX - $minX + 1), ($maxY - $minY + 1))
}
