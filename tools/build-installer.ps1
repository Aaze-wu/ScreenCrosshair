# 一步出安装包：cargo build --release + ISCC 编译 .iss
#
# 为什么要有这个脚本：`ScreenCrosshair.iss` 的版本号是从 exe 的 VERSIONINFO
# 读出来的，所以「先 build 再打包」的顺序是硬要求。忘了 build 时 ISCC 会直接
# 报错退出，而这个脚本把这层依赖串起来，顺便把「别忘 build」这类事忘掉。
#
# 用法：
#   powershell -ExecutionPolicy Bypass -File tools\build-installer.ps1
#   powershell ... -SkipBuild      # 只重打包，不重新编译
#   powershell ... -Iscc <路径>    # 指定 ISCC.exe
#
# 产物：dist\ScreenCrosshair-<版本>-setup.exe

[CmdletBinding()]
param(
    [switch]$SkipBuild,
    [string]$Iscc
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

# ---- 找 ISCC.exe ----
# Inno Setup 的安装路径没有标准位置（6 装在 Program Files (x86)，7 可能装在
# 任何盘），所以按「已知候选 → 注册表 → PATH」的顺序找，都没有才报错。
if (-not $Iscc) {
    $candidates = @(
        'C:\Program Files (x86)\Inno Setup 7\ISCC.exe',
        'C:\Program Files\Inno Setup 7\ISCC.exe',
        'C:\Program Files (x86)\Inno Setup 6\ISCC.exe',
        'C:\Program Files\Inno Setup 6\ISCC.exe',
        'D:\Program Files\Inno Setup 7\ISCC.exe',
        'D:\Program Files\Inno Setup 6\ISCC.exe'
    )
    foreach ($p in $candidates) {
        if (Test-Path $p) { $Iscc = $p; break }
    }
}
if (-not $Iscc) {
    # 注册表里 Inno 自己登记的安装目录（卸载信息在 32 位视图下）
    foreach ($key in @(
            'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\Inno Setup 7_is1',
            'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\Inno Setup 6_is1',
            'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Inno Setup 7_is1')) {
        try {
            $dir = (Get-ItemProperty -Path $key -ErrorAction Stop).InstallLocation
            if ($dir -and (Test-Path (Join-Path $dir 'ISCC.exe'))) {
                $Iscc = Join-Path $dir 'ISCC.exe'
                break
            }
        } catch { }
    }
}
if (-not $Iscc) {
    $cmd = Get-Command ISCC.exe -ErrorAction SilentlyContinue
    if ($cmd) { $Iscc = $cmd.Source }
}
if (-not $Iscc -or -not (Test-Path $Iscc)) {
    throw '找不到 ISCC.exe，请安装 Inno Setup 6/7，或用 -Iscc 指定路径。'
}
Write-Host "ISCC: $Iscc"

# ---- 先 build ----
if (-not $SkipBuild) {
    Write-Host '正在编译 release...' -ForegroundColor Cyan
    cargo build --release
    if ($LASTEXITCODE -ne 0) { throw "cargo build 失败（退出码 $LASTEXITCODE）" }
}

$exe = Join-Path $root 'target\release\screencrosshair.exe'
if (-not (Test-Path $exe)) {
    throw "找不到 $exe，请先 cargo build --release"
}

# 编译前先把版本号念出来：安装包的版本直接来自这里，念一遍能立刻发现
# 「改了 Cargo.toml 但忘了 build」这类不一致。
$ver = (Get-Item $exe).VersionInfo.FileVersion
Write-Host "exe 版本: $ver" -ForegroundColor Cyan
if (-not $ver) {
    Write-Warning 'exe 没有版本资源（rc.exe 没跑成功？）安装包会因此报错。'
}

# ---- 打包 ----
# 输出交给 ISCC 自己打印，不接 Select-Object：那会吞掉报错。
& $Iscc 'installer\ScreenCrosshair.iss'
if ($LASTEXITCODE -ne 0) { throw "ISCC 失败（退出码 $LASTEXITCODE）" }

$setup = Get-ChildItem (Join-Path $root 'dist') -Filter '*-setup.exe' |
    Sort-Object LastWriteTime -Descending |
    Select-Object -First 1
if ($setup) {
    $mb = [math]::Round($setup.Length / 1MB, 2)
    Write-Host ''
    Write-Host "安装包: $($setup.FullName)  ($mb MB)" -ForegroundColor Green
}
