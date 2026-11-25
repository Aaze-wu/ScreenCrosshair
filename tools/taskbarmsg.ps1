<#
.SYNOPSIS
    模拟「资源管理器重启」，给 ScreenCrosshair 的消息窗口广播 TaskbarCreated。

.DESCRIPTION
    真实场景是 explorer.exe 崩溃/重启后，任务栏把托盘区清空，并在启动完成时
    向所有顶层窗口广播一条 `RegisterWindowMessageW("TaskbarCreated")`。
    程序必须收到它并重新 Shell_NotifyIconW(NIM_ADD)，否则托盘图标永久消失。

    这个脚本不重启资源管理器（那会把桌面和所有资源管理器窗口一起关掉），
    而是直接用同一个消息号给目标窗口 PostMessage，验证回调链路是否通畅。
    代价是只验证「我们这一侧」，验证不到「系统确实会广播」——后者是 Windows 行为。

.NOTES
    查窗口句柄用 EnumWindows 按 PID + 类名匹配，不用 FindWindow：
    实测 FindWindowW 对这两个自绘窗口类恒返回 0（系统类 Shell_TrayWnd 正常），
    而 EnumWindows 能正常枚举到，所以跟 probe.ps1 保持一致。

.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\taskbarmsg.ps1
#>
param(
    [string]$ClassName = 'ScreenCrosshairMsg'
)

$ErrorActionPreference = 'Stop'

Add-Type @'
using System;
using System.Runtime.InteropServices;
using System.Text;

public static class ScreenCrosshairTaskbarMsg
{
    public delegate bool EnumProc(IntPtr h, IntPtr p);

    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern uint RegisterWindowMessage(string lpString);

    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr p);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
}
'@

$proc = Get-Process screencrosshair -ErrorAction SilentlyContinue
if (-not $proc) { Write-Output 'FAIL 程序没在跑'; exit 1 }
$targetPid = $proc.Id
Write-Output "pid = $targetPid"

$script:found = [IntPtr]::Zero
$cb = [ScreenCrosshairTaskbarMsg+EnumProc] {
    param($h, $p)
    $q = 0
    [void][ScreenCrosshairTaskbarMsg]::GetWindowThreadProcessId($h, [ref]$q)
    if ($q -eq $targetPid) {
        $sb = New-Object System.Text.StringBuilder 256
        [void][ScreenCrosshairTaskbarMsg]::GetClassName($h, $sb, 256)
        if ($sb.ToString() -eq $ClassName) { $script:found = $h; return $false }
    }
    return $true
}
[void][ScreenCrosshairTaskbarMsg]::EnumWindows($cb, [IntPtr]::Zero)

$hwnd = $script:found
if ($hwnd -eq [IntPtr]::Zero) { Write-Output "FAIL 找不到窗口类 $ClassName"; exit 1 }
Write-Output "hwnd = $hwnd"

$msg = [ScreenCrosshairTaskbarMsg]::RegisterWindowMessage('TaskbarCreated')
Write-Output "taskbarCreated msg = $msg"

$null = [ScreenCrosshairTaskbarMsg]::PostMessage($hwnd, $msg, [IntPtr]::Zero, [IntPtr]::Zero)
Write-Output 'posted'

Start-Sleep -Milliseconds 900
$p = Get-Process screencrosshair -ErrorAction SilentlyContinue
if (-not $p) { Write-Output 'FAIL 进程已退出（重挂过程中崩溃）'; exit 1 }
Write-Output "process alive, handles = $($p.HandleCount)"

# 重挂后图标必须还在。粗检：UIA 里还能不能枚举到我们的托盘按钮。
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
$A = [System.Windows.Automation.AutomationElement]
$btnCond = New-Object System.Windows.Automation.PropertyCondition(
    $A::ControlTypeProperty, [System.Windows.Automation.ControlType]::Button)
$hits = 0
foreach ($cls in @('Shell_TrayWnd', 'TopLevelWindowForOverflowXamlIsland', 'NotifyIconOverflowWindow')) {
    $cond = New-Object System.Windows.Automation.PropertyCondition($A::ClassNameProperty, $cls)
    $win = $A::RootElement.FindFirst([System.Windows.Automation.TreeScope]::Children, $cond)
    if (-not $win) { continue }
    foreach ($b in $win.FindAll([System.Windows.Automation.TreeScope]::Descendants, $btnCond)) {
        if ($b.Current.Name -like '*ScreenCrosshair*') { $hits++ }
    }
}
Write-Output "tray button hits = $hits"
if ($hits -lt 1) {
    Write-Output 'NOTE 可见托盘区与未展开的溢出面板都查过了；图标可能还在溢出面板块里（未展开时 UIA 枚举不到），属正常'
}
