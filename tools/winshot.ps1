# 按窗口类名（和可选标题）截图（DPI 感知）。给「验证某个窗口的布局有没有被裁掉」用。
#
# 参数：
#   -Class    窗口类名（ASCII，最可靠；中文标题从命令行传容易被控制台代码页搞坏）
#   -Title    窗口标题（精确匹配，可选）
#   -Tag      输出文件名后缀（默认 winshot），写到 $env:TEMP\sc_<Tag>.png
param(
    [Parameter(Mandatory = $true)][string]$Class,
    [string]$Title,
    [string]$Tag = 'winshot'
)

$ErrorActionPreference = 'Stop'

if (-not ('WinShot' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class WinShot {
    public delegate bool EnumProc(IntPtr h, IntPtr p);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr p);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    public struct RECT { public int L, T, R, B; }
}
'@
}

# 必须先声明 DPI 感知，否则 GetWindowRect 与 CopyFromScreen 都是缩放后的坐标，
# 截出来的图会小一圈并且错位（实测在 150% 缩放屏上差 1.5 倍）。
[void][WinShot]::SetProcessDPIAware()

# 用 EnumWindows 自己找，不用 FindWindowW：后者的「未指定条件」是空串而不是 NULL 时
# 永远匹配不到（踩过，症状是类名明明存在却报 not found）。另外这里只挑可见窗口，
# 免得撞上 winit 那些同名但隐藏的辅助窗口。
$script:found = [IntPtr]::Zero
$script:foundTitle = ''
$cb = [WinShot+EnumProc] {
    param($h, $p)
    if (-not [WinShot]::IsWindowVisible($h)) { return $true }
    $cls = New-Object System.Text.StringBuilder 256
    [void][WinShot]::GetClassNameW($h, $cls, 256)
    if ($cls.ToString() -ne $Class) { return $true }
    $txt = New-Object System.Text.StringBuilder 512
    [void][WinShot]::GetWindowTextW($h, $txt, 512)
    if ($Title -and $txt.ToString() -ne $Title) { return $true }
    $script:found = $h
    $script:foundTitle = $txt.ToString()
    return $false
}
[void][WinShot]::EnumWindows($cb, [IntPtr]::Zero)

if ($script:found -eq [IntPtr]::Zero) {
    Write-Output "FAIL: window not found: class='$Class' title='$Title'"
    exit 1
}

Write-Output ("hwnd=0x{0:X} class={1} title={2}" -f $script:found.ToInt64(), $Class, $script:foundTitle)

$r = New-Object WinShot+RECT
[void][WinShot]::GetWindowRect($script:found, [ref]$r)
$w = $r.R - $r.L
$h = $r.B - $r.T
Write-Output ("rect=({0},{1}) {2}x{3}" -f $r.L, $r.T, $w, $h)

Add-Type -AssemblyName System.Drawing
$bmp = New-Object System.Drawing.Bitmap $w, $h
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($r.L, $r.T, 0, 0, $bmp.Size)
$g.Dispose()

$out = Join-Path $env:TEMP ("sc_{0}.png" -f $Tag)
$bmp.Save($out, [System.Drawing.Imaging.ImageFormat]::Png)
Write-Output "saved $out"
