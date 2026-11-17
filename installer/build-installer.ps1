# Gera installer\Output\SensoryxSetup-<versao>.exe e o SHA256SUMS.txt correspondente.
# Requisitos: Rust (cargo), Inno Setup 6.1+ (ISCC.exe) e a pasta vendor\ com o PawnIO
#   vendor\pawnio-modules\*.bin + COPYING  (https://github.com/namazso/PawnIO.Modules/releases)
#   vendor\pawnio-modules-src\*.p          (codigo-fonte dos modulos, mesma versao)
#   vendor\pawnio-license\COPYING-PawnIO-GPL2.txt
$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
Set-Location $root

$need = @(
  "vendor\pawnio-modules\IntelMSR.bin",
  "vendor\pawnio-modules\AMDFamily17.bin",
  "vendor\pawnio-modules\COPYING",
  "vendor\pawnio-modules-src\IntelMSR.p",
  "vendor\pawnio-modules-src\AMDFamily17.p",
  "vendor\pawnio-license\COPYING-PawnIO-GPL2.txt"
)
foreach ($f in $need) { if (-not (Test-Path $f)) { throw "Faltando: $f" } }

# Versao unica: a do Cargo.toml.
$m = Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1
if (-not $m) { throw "Nao achei a versao no Cargo.toml" }
$version = $m.Matches[0].Groups[1].Value
Write-Host "Versao: $version"

cargo build --release
if ($LASTEXITCODE -ne 0) { throw "cargo build falhou" }

$iscc = @(
  "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
  "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
  "$env:ProgramFiles\Inno Setup 6\ISCC.exe"
) | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $iscc) { throw "Inno Setup 6 nao encontrado (ISCC.exe)" }

& $iscc "/DAppVersion=$version" "installer\sensoryx.iss"
if ($LASTEXITCODE -ne 0) { throw "ISCC falhou" }

# SHA256SUMS.txt: a aba de atualizacao do app so baixa se este hash estiver publicado no release.
$setup = Get-Item "installer\Output\SensoryxSetup-$version.exe"
$hash = (Get-FileHash $setup.FullName -Algorithm SHA256).Hash.ToLower()
[IO.File]::WriteAllText((Join-Path $setup.DirectoryName "SHA256SUMS.txt"), "$hash  $($setup.Name)`n", (New-Object System.Text.UTF8Encoding($false)))
Write-Host "SHA-256: $hash"
Get-ChildItem installer\Output\* | Select-Object Name, @{n='MB';e={[math]::Round($_.Length/1MB,2)}}
