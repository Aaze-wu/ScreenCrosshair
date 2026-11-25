param(
    [string]$Preset = '',
    [string]$Color = '',
    [string]$Opacity = '',
    [string]$Scale = '',
    [string]$Rotation = '',
    [string]$Visible = ''
)
$ErrorActionPreference = 'Stop'
$p = Join-Path $env:APPDATA 'ScreenCrosshair\config.json'
if (-not (Test-Path $p)) { Write-Output "no config at $p"; exit 1 }

$c = Get-Content $p -Raw | ConvertFrom-Json
$ov = [ordered]@{}
foreach ($prop in $c.overrides.PSObject.Properties) { $ov[$prop.Name] = $prop.Value }

if ($Preset) { $c.active = $Preset }
if ($Color) { $ov['color'] = $Color }
if ($Opacity) { $ov['opacity'] = [double]$Opacity }
if ($Scale) { $ov['scale'] = [double]$Scale }
if ($Rotation) { $ov['rotation_deg'] = [double]$Rotation }
if ($Visible) { $c.visible = ($Visible -eq 'true') }

$c.overrides = [pscustomobject]$ov
$json = $c | ConvertTo-Json -Depth 20
[System.IO.File]::WriteAllText($p, $json, (New-Object System.Text.UTF8Encoding $false))

Write-Output ("active={0} visible={1} overrides={2}" -f $c.active, $c.visible, ([pscustomobject]$ov | ConvertTo-Json -Compress))
