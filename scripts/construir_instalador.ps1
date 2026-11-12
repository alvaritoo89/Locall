<#
.SYNOPSIS
    Builds Locall's Windows installer (ADR 0032): target\release\bundle\nsis\Locall_<version>_x64-setup.exe.

.DESCRIPTION
    1. Compiles locall.exe (release).
    2. Prepares the verified engine in target\paquete\motor (scripts\preparar_motor.ps1, ADR 0031).
    3. Installs the UI's exact dependencies (npm ci) and writes target\paquete\THIRD-PARTY-NOTICES.txt, refusing if any
       license is not accepted (scripts\avisos_de_terceros.py).
    4. Builds the app and the installer with Tauri, adding the installer-only configuration
       (crates\locall-app\instalador\tauri.instalador.conf.json).
    5. Prints the installer's path, size and sha256.

    The first time, Tauri downloads its NSIS tools (checked by Tauri itself) into %LOCALAPPDATA%\tauri.

    Run it from the repository root:

        powershell -ExecutionPolicy Bypass -File scripts\construir_instalador.ps1

.PARAMETER Zip
    Optional: the official engine zip, already downloaded (see preparar_motor.ps1).

.PARAMETER SinNpmCi
    Skip `npm ci` and use the node_modules already installed (faster when developing).
#>
param(
    [string] $Zip,
    [switch] $SinNpmCi
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$raiz = Split-Path -Parent $PSScriptRoot
$app = Join-Path $raiz 'crates\locall-app'
$paquete = Join-Path $raiz 'target\paquete'

function Paso([string] $Texto) { Write-Host "`n== $Texto" -ForegroundColor Cyan }

# Native programs report failure through their exit code, not as PowerShell errors: check it after each one.
function Ejecutar([string] $Que, [scriptblock] $Orden) {
    & $Orden
    if ($LASTEXITCODE -ne 0) { throw "$Que ha fallado (código $LASTEXITCODE)." }
}

Push-Location $raiz
try {
    Paso 'locall.exe (release)'
    Ejecutar 'cargo build' { cargo build --release --locked -p locall }

    Paso 'Motor verificado'
    $argumentos = @{ Destino = $paquete }
    if ($Zip) { $argumentos.Zip = $Zip }
    & (Join-Path $PSScriptRoot 'preparar_motor.ps1') @argumentos

    Paso 'Dependencias de la interfaz'
    if (-not $SinNpmCi) {
        Push-Location $app
        try { Ejecutar 'npm ci' { npm ci --no-audit --no-fund } } finally { Pop-Location }
    } else {
        Write-Host '  (se usa node_modules tal cual)'
    }

    Paso 'Licencias de terceros'
    Ejecutar 'avisos_de_terceros.py' {
        python (Join-Path $PSScriptRoot 'avisos_de_terceros.py') --salida (Join-Path $paquete 'THIRD-PARTY-NOTICES.txt')
    }

    Paso 'App e instalador (Tauri)'
    Push-Location $app
    try {
        Ejecutar 'tauri build' { npx --no-install tauri build --config instalador/tauri.instalador.conf.json }
    } finally { Pop-Location }

    Paso 'Resultado'
    $instaladores = @(Get-ChildItem -LiteralPath (Join-Path $raiz 'target\release\bundle\nsis') -Filter '*-setup.exe' |
        Sort-Object LastWriteTime -Descending)
    if ($instaladores.Count -eq 0) { throw 'Tauri no ha generado ningún instalador.' }
    $instalador = $instaladores[0]
    $huella = (Get-FileHash -Algorithm SHA256 -LiteralPath $instalador.FullName).Hash.ToLowerInvariant()
    Write-Host ("{0}`n  {1:N1} MB`n  sha256 {2}" -f $instalador.FullName, ($instalador.Length / 1MB), $huella)
} finally {
    Pop-Location
}
