<#
.SYNOPSIS
    一次性把准星染色成纯绿、截图、再全屏定位纯绿像素。

.DESCRIPTION
    排查「准星到底画在哪里 / 有没有画」时最干净的办法：换成一个桌面上绝不会
    出现的颜色（纯绿 #00FF00），这样 findcross 的结果不会被界面颜色污染。
    跑完记得用 -Restore 把颜色改回去（默认改成 #FF3B30）。
#>
param(
    [string]$ProbeColor = '#00FF00',
    [string]$RestoreColor = '',
    [int]$WaitMs = 1400
)
$ErrorActionPreference = 'Stop'
$tools = Split-Path -Parent $MyInvocation.MyCommand.Path
$cfg = Join-Path $env:APPDATA 'ScreenCrosshair\config.json'

& (Join-Path $tools 'setcfg.ps1') -Color $ProbeColor | Out-Null
Start-Sleep -Milliseconds $WaitMs

$rgb = [Convert]::ToInt32($ProbeColor.Substring(1), 16)
& (Join-Path $tools 'findcross.ps1') -Red (($rgb -shr 16) -band 0xFF) -Green (($rgb -shr 8) -band 0xFF) -Blue ($rgb -band 0xFF) -Tol 10 -Tag probe

if ($RestoreColor) {
    & (Join-Path $tools 'setcfg.ps1') -Color $RestoreColor | Out-Null
    Write-Output "restored to $RestoreColor"
}
