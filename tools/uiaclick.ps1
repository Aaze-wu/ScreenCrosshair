# 用 UI 自动化在指定窗口里按文本点一个控件。
#
# 为什么用 UIA 而不是注入键盘/鼠标事件：本程序走的是 Win32 消息循环，
# 注入的 Alt+F4 之类需要目标窗口是前台窗口，而合成消息不满足
# 「进程刚收到用户输入」的条件，SetForegroundWindow 会被系统拒绝。
# UIA 直接调控件的 Invoke，不依赖焦点，脚本化最可靠。
#
# 用法：
#   .\uiaclick.ps1 -Class 'Window Class' -Name '关闭窗口'
param(
    [Parameter(Mandatory = $true)][string]$Class,
    [Parameter(Mandatory = $true)][string]$Name
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes

$root = [System.Windows.Automation.AutomationElement]::RootElement
$clsCond = New-Object System.Windows.Automation.PropertyCondition(
    [System.Windows.Automation.AutomationElement]::ClassNameProperty, $Class)
$win = $root.FindFirst([System.Windows.Automation.TreeScope]::Children, $clsCond)
if (-not $win) {
    Write-Output "FAIL: window not found: class='$Class'"
    exit 1
}

$nameCond = New-Object System.Windows.Automation.PropertyCondition(
    [System.Windows.Automation.AutomationElement]::NameProperty, $Name)
$el = $win.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $nameCond)
if (-not $el) {
    Write-Output "FAIL: element not found: '$Name'"
    exit 1
}

$pattern = $null
if ($el.TryGetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern, [ref]$pattern)) {
    $pattern.Invoke()
    Write-Output "invoked '$Name'"
} elseif ($el.TryGetCurrentPattern([System.Windows.Automation.TogglePattern]::Pattern, [ref]$pattern)) {
    $pattern.Toggle()
    Write-Output "toggled '$Name'"
} else {
    Write-Output "FAIL: '$Name' has no Invoke/Toggle pattern"
    exit 1
}
