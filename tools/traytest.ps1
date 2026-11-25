# 托盘菜单端到端自测：弹出菜单 -> 键盘导航 -> 选中「开机自启」-> 校验注册表 -> 再切回。
#
# 走的是真实链路：Shell_NotifyIcon 会把 WM_TRAY(wParam=图标 id, lParam=鼠标消息)
# 送到消息窗口。这里直接找到该窗口并投递同样的消息，等价于右键托盘图标，
# 但不必在任务栏托盘区去点那一个像素。
#
# 用 PostMessage 而不是 SendMessage：TrackPopupMenu 是模态阻塞的，
# SendMessage 会一直卡到菜单关闭，脚本就没法继续发按键了。
param(
    [int]$TogglesBefore = 10,   # 「开机自启」前面有几项（分隔线不计）
    [switch]$NoEnter            # 只弹菜单不选择，方便截图
)

$ErrorActionPreference = 'Stop'

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
    public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
    [DllImport("user32.dll")]
    public static extern bool SetProcessDPIAware();
}
'@
}
[void][TrayProbe]::SetProcessDPIAware()

function Send-Vk([byte]$vk) {
    [TrayProbe]::keybd_event($vk, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 40
    [TrayProbe]::keybd_event($vk, 0, 2, [UIntPtr]::Zero)
}

$VK_DOWN = 0x28
$VK_RETURN = 0x0D

$WM_TRAY = 0x8001
$WM_RBUTTONUP = 0x0205
$RUN = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'

# 程序刚启动时窗口还没建出来，轮询等一下。
$hwnd = [IntPtr]::Zero
for ($i = 0; $i -lt 40; $i++) {
    $hwnd = [TrayProbe]::FindWindowW('ScreenCrosshairMsg', [NullString]::Value)
    if ($hwnd -ne [IntPtr]::Zero) { break }
    Start-Sleep -Milliseconds 100
}
if ($hwnd -eq [IntPtr]::Zero) {
    Write-Output 'FAIL: msg window not found'
    exit 1
}
Write-Output "msg hwnd=0x$('{0:X}' -f $hwnd.ToInt64())"

function Show-Autostart([string]$tag) {
    $v = (Get-ItemProperty -Path $RUN -Name 'ScreenCrosshair' -ErrorAction SilentlyContinue).ScreenCrosshair
    if ($v) { Write-Output "$tag ON  -> $v" } else { Write-Output "$tag OFF" }
}

Show-Autostart 'before:'

# 光标放到屏幕中间偏上，菜单才不会被屏幕边缘挤变形。
Add-Type -AssemblyName System.Windows.Forms
$b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
[void][TrayProbe]::SetCursorPos([int]($b.Width / 3), [int]($b.Height / 3))
Start-Sleep -Milliseconds 150

[void][TrayProbe]::PostMessageW($hwnd, $WM_TRAY, [IntPtr]1, [IntPtr]$WM_RBUTTONUP)
Start-Sleep -Milliseconds 600

if ($NoEnter) {
    Write-Output 'menu posted (left open on purpose)'
    exit 0
}

for ($i = 0; $i -lt $TogglesBefore; $i++) { Send-Vk $VK_DOWN }
Start-Sleep -Milliseconds 150
Send-Vk $VK_RETURN
Start-Sleep -Milliseconds 800
Show-Autostart 'after-toggle-1:'

# 再走一遍切回原状，别给这台机器留下自启项。
[void][TrayProbe]::PostMessageW($hwnd, $WM_TRAY, [IntPtr]1, [IntPtr]$WM_RBUTTONUP)
Start-Sleep -Milliseconds 600
for ($i = 0; $i -lt $TogglesBefore; $i++) { Send-Vk $VK_DOWN }
Start-Sleep -Milliseconds 150
Send-Vk $VK_RETURN
Start-Sleep -Milliseconds 800
Show-Autostart 'after-toggle-2:'

Write-Output 'done'
