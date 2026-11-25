# 用 UI 自动化定位托盘菜单项并点击，验证「菜单 -> 命令 -> 生效」整条链路。
#
# 为什么不用键盘：TrackPopupMenu 要靠 SetForegroundWindow 才收得到键盘，
# 而合成投递 WM_TRAY 时进程并不满足「刚收到输入」的条件，SetForegroundWindow
# 会被系统拒绝 —— 这是注入场景的固有限制，不是程序的问题。真实右键托盘时
# 进程刚收到输入，这条路径是正常的。鼠标则不然：弹出菜单会 SetCapture，
# 点击必然送达菜单，所以用鼠标点。
param(
    [string]$ItemName = '开机自启',
    [switch]$NoClick
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, System.Windows.Forms

if (-not ('TrayProbe' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class TrayProbe {
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern IntPtr FindWindowW(string cls, string title);
    [DllImport("user32.dll")]
    public static extern bool PostMessageW(IntPtr h, uint msg, IntPtr wp, IntPtr lp);
    [DllImport("user32.dll")]
    public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")]
    public static extern void mouse_event(uint f, int dx, int dy, uint data, UIntPtr extra);
    [DllImport("user32.dll")]
    public static extern bool SetProcessDPIAware();
}
'@
}
[void][TrayProbe]::SetProcessDPIAware()

$WM_TRAY = 0x8001
$WM_RBUTTONUP = 0x0205
$RUN = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'

function Show-Autostart([string]$tag) {
    $v = (Get-ItemProperty -Path $RUN -Name 'ScreenCrosshair' -ErrorAction SilentlyContinue).ScreenCrosshair
    if ($v) { Write-Output "$tag ON  -> $v" } else { Write-Output "$tag OFF" }
}

$hwnd = [TrayProbe]::FindWindowW('ScreenCrosshairMsg', [NullString]::Value)
if ($hwnd -eq [IntPtr]::Zero) { Write-Output 'FAIL: msg window not found'; exit 1 }

function Open-Menu {
    $b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
    [void][TrayProbe]::SetCursorPos([int]($b.Width / 3), [int]($b.Height / 3))
    Start-Sleep -Milliseconds 120
    [void][TrayProbe]::PostMessageW($hwnd, $WM_TRAY, [IntPtr]1, [IntPtr]$WM_RBUTTONUP)
    Start-Sleep -Milliseconds 700
}

function Find-MenuItem([string]$name) {
    $root = [System.Windows.Automation.AutomationElement]::RootElement
    $cond = New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
        [System.Windows.Automation.ControlType]::MenuItem)
    $all = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $cond)
    foreach ($e in $all) {
        if ($e.Current.Name -eq $name) { return $e }
    }
    return $null
}

Show-Autostart 'before:      '

Open-Menu
$item = Find-MenuItem $ItemName
if (-not $item) {
    Write-Output "FAIL: menu item not found: $ItemName"
    exit 1
}
$r = $item.Current.BoundingRectangle
Write-Output ("found '{0}' at {1},{2} {3}x{4}" -f $ItemName, [int]$r.X, [int]$r.Y, [int]$r.Width, [int]$r.Height)

if ($NoClick) { Write-Output 'dry run, not clicking'; exit 0 }

$cx = [int]($r.X + $r.Width / 2)
$cy = [int]($r.Y + $r.Height / 2)
[void][TrayProbe]::SetCursorPos($cx, $cy)
Start-Sleep -Milliseconds 120
[TrayProbe]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)   # LEFTDOWN
Start-Sleep -Milliseconds 60
[TrayProbe]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)   # LEFTUP
Start-Sleep -Milliseconds 900
Show-Autostart 'after-click-1:'

# 再点一次切回原状。
Open-Menu
$item = Find-MenuItem $ItemName
if ($item) {
    $r = $item.Current.BoundingRectangle
    [void][TrayProbe]::SetCursorPos([int]($r.X + $r.Width / 2), [int]($r.Y + $r.Height / 2))
    Start-Sleep -Milliseconds 120
    [TrayProbe]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 60
    [TrayProbe]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 900
}
Show-Autostart 'after-click-2:'
Write-Output 'done'
