# 查托盘区里到底有没有我们的图标（普通区 + 溢出区都查）。
#
# 背景：Windows 11 会把新出现的托盘图标默认塞进「溢出」浮出面板，从任务栏
# 表面看就像「图标没加上」。这里用 UIA 枚举两处的按钮，一次区分：
#   - 两处都找不到  => NIM_ADD 没生效（真 bug）
#   - 只在溢出区找到 => 图标加上了，只是被系统收进溢出面板
param(
    [string]$Match = 'ScreenCrosshair',
    [switch]$List,             # 顺便列出托盘区所有按钮名，便于对照
    [switch]$Open,             # 先点开「显示隐藏的图标」，把溢出面板也枚举一遍
    [switch]$KeepOpen          # 枚举完不关面板（方便截图）
)

$ErrorActionPreference = 'Stop'

Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes

$A = [System.Windows.Automation.AutomationElement]
$Desc = [System.Windows.Automation.TreeScope]::Descendants
$Children = [System.Windows.Automation.TreeScope]::Children

$btnCond = New-Object System.Windows.Automation.PropertyCondition(
    $A::ControlTypeProperty, [System.Windows.Automation.ControlType]::Button)

function Scan-Area([System.Windows.Automation.AutomationElement]$root, [string]$label) {
    if ($null -eq $root) {
        Write-Output "$label : NOT FOUND (窗口不存在)"
        return
    }
    $hits = $root.FindAll($Desc, $btnCond)
    $found = 0
    foreach ($e in $hits) {
        $name = $e.Current.Name
        if ($List) {
            $r0 = $e.Current.BoundingRectangle
            Write-Output ("  [{0},{1}] name='{2}'" -f [int]$r0.X, [int]$r0.Y, $name)
        }
        if ($name -and $name -like "*$Match*") {
            $r = $e.Current.BoundingRectangle
            Write-Output ("{0}: HIT name='{1}' rect=({2},{3})-({4},{5})" -f `
                    $label, $name, [int]$r.X, [int]$r.Y, [int]$r.Right, [int]$r.Bottom)
            $found++
        }
    }
    Write-Output ("{0}: buttons={1} hits={2}" -f $label, $hits.Count, $found)
}

$root = $A::RootElement

function Find-TopLevel([string]$class) {
    $c = New-Object System.Windows.Automation.PropertyCondition($A::ClassNameProperty, $class)
    return $root.FindFirst($Children, $c)
}

Scan-Area (Find-TopLevel 'Shell_TrayWnd') 'tray'

if ($Open) {
    # 溢出面板是个按需创建的浮出窗口：没点开时 UIA 里根本不存在，
    # 所以「托盘区看不到」并不能区分「在溢出里」和「没加上」。
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class TrayIconMouse {
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint f, int dx, int dy, uint d, UIntPtr e);
    [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
}
'@
    [void][TrayIconMouse]::SetProcessDPIAware()

    $chev = $null
    $trayRoot = Find-TopLevel 'Shell_TrayWnd'
    if ($trayRoot) {
        foreach ($e in $trayRoot.FindAll($Desc, $btnCond)) {
            if ($e.Current.Name -like '*显示隐藏*') { $chev = $e; break }
        }
    }
    if (-not $chev) {
        Write-Output 'overflow : 找不到「显示隐藏的图标」按钮'
    } else {
        $r = $chev.Current.BoundingRectangle
        $cx = [int]($r.X + $r.Width / 2)
        $cy = [int]($r.Y + $r.Height / 2)
        [void][TrayIconMouse]::SetCursorPos($cx, $cy)
        Start-Sleep -Milliseconds 80
        [TrayIconMouse]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)   # LEFTDOWN
        [TrayIconMouse]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)   # LEFTUP
        Start-Sleep -Milliseconds 700
        # Win10 叫 NotifyIconOverflowWindow，Win11 是 XAML island，
        # 类名形如 TopLevelWindowForOverflowXamlIsland ⇒ 按子串匹配，两个都盖。
        $tops = $root.FindAll($Children, [System.Windows.Automation.Condition]::TrueCondition)
        $anyOverflow = $false
        foreach ($t in $tops) {
            $cls = $t.Current.ClassName
            if ($cls -and $cls -match 'Overflow') {
                $anyOverflow = $true
                Scan-Area $t "overflow($cls)"
            }
        }
        if (-not $anyOverflow) { Write-Output 'overflow : 浮出窗口没出现（点开失败）' }
        if (-not $KeepOpen) {
            [TrayIconMouse]::keybd_event(0x1B, 0, 0, [UIntPtr]::Zero)    # ESC 关掉浮出面板
            [TrayIconMouse]::keybd_event(0x1B, 0, 2, [UIntPtr]::Zero)
        }
    }
} else {
    Scan-Area (Find-TopLevel 'NotifyIconOverflowWindow') 'overflow'
    Scan-Area (Find-TopLevel 'TopLevelWindowForOverflowXamlIsland') 'overflow(xaml)'
}
