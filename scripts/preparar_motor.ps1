<#
.SYNOPSIS
    Prepares the engine folder that ships with Locall (ADR 0031): the official llama.cpp build b12189 for the CPU,
    verified by its sha256, with only the files `llama-server` needs and the licenses that go with them.

.DESCRIPTION
    1. Downloads the official zip from the llama.cpp releases on GitHub (or uses -Zip if it is already on disk) and
       refuses to continue if its sha256 is not exactly the pinned one.
    2. Copies into <Destino>\motor only `llama-server.exe` and the libraries it loads. The other programs in the zip
       (benchmarks, quantizers, other command-line tools, the RPC backend for remote computers) are left out.
    3. Adds the licenses: llama.cpp's MIT license (not included in the zip: it is downloaded from the repository at
       the same tag and also checked by its sha256) and LLVM OpenMP's (included in the zip).
    4. Runs `llama-server --version` from the destination to check that it starts and is the expected build.

    Works with Windows PowerShell 5.1 and PowerShell 7. Run it from the repository root, for example:

        powershell -ExecutionPolicy Bypass -File scripts\preparar_motor.ps1 -Destino target\paquete

.PARAMETER Destino
    Folder where the `motor` folder is created (next to where locall.exe will be).

.PARAMETER Zip
    Optional: path to the official zip, already downloaded. It is checked exactly like a download.
#>
param(
    [Parameter(Mandatory = $true)] [string] $Destino,
    [string] $Zip
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

# ─── The pinned engine (change all of these together, and re-test, when updating llama.cpp) ───
$Version = 'b12189'
$UrlZip = "https://github.com/ggml-org/llama.cpp/releases/download/$Version/llama-$Version-bin-win-cpu-x64.zip"
$Sha256Zip = 'f912ea5e3e397c63405c81e732131ab0fa6e72411c80546bb5ad643f7dd82aac'
$UrlLicencia = "https://raw.githubusercontent.com/ggml-org/llama.cpp/$Version/LICENSE"
$Sha256Licencia = '94f29bbed6a22c35b992c5c6ebf0e7c92f13b836b90f36f461c9cf2f0f1d010d'

# What llama-server needs: the program, its libraries and the CPU backends (ggml picks the best one for the processor
# when it starts). Nothing else from the zip.
$Necesarios = @(
    'llama-server.exe', 'llama-server-impl.dll', 'llama-common.dll', 'llama.dll', 'mtmd.dll',
    'ggml.dll', 'ggml-base.dll', 'libomp.dll'
)
$Patrones = @('ggml-cpu-*.dll')
$Licencias = @('LICENSE-LLVM-OpenMP')

function Comprobar-Huella([string] $Ruta, [string] $Esperada, [string] $Que) {
    $huella = (Get-FileHash -Algorithm SHA256 -LiteralPath $Ruta).Hash.ToLowerInvariant()
    if ($huella -ne $Esperada) {
        throw "La huella no coincide ($Que).`n  esperada: $Esperada`n  obtenida: $huella`nNo se usa."
    }
    Write-Host "  huella correcta ($Que)"
}

function Descargar([string] $Url, [string] $Ruta) {
    Write-Host "  descargando $Url"
    # TLS 1.2 explicitly: Windows PowerShell 5.1 may not enable it by default.
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    Invoke-WebRequest -Uri $Url -OutFile $Ruta -UseBasicParsing
}

$temporal = Join-Path ([IO.Path]::GetTempPath()) ("locall-motor-" + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $temporal | Out-Null
try {
    Write-Host "Motor llama.cpp $Version (procesador)"

    # 1. The zip, checked.
    if ($Zip) {
        $rutaZip = (Resolve-Path -LiteralPath $Zip).Path
    } else {
        $rutaZip = Join-Path $temporal 'motor.zip'
        Descargar $UrlZip $rutaZip
    }
    Comprobar-Huella $rutaZip $Sha256Zip 'el zip del motor'

    # 2. Extract it apart and copy only what is needed.
    $extraido = Join-Path $temporal 'zip'
    Expand-Archive -LiteralPath $rutaZip -DestinationPath $extraido
    $motor = Join-Path $Destino 'motor'
    if (Test-Path -LiteralPath $motor) {
        # Only a folder this script created before is replaced: it must contain llama-server.exe and nothing that
        # looks like user data. Anything else is left alone and the script stops.
        if (-not (Test-Path -LiteralPath (Join-Path $motor 'llama-server.exe'))) {
            throw "$motor ya existe y no parece una carpeta del motor; no se toca."
        }
        Remove-Item -LiteralPath $motor -Recurse -Force
    }
    New-Item -ItemType Directory -Path $motor | Out-Null

    $copiados = 0
    foreach ($nombre in $Necesarios + $Licencias) {
        $origen = Join-Path $extraido $nombre
        if (-not (Test-Path -LiteralPath $origen)) { throw "Falta $nombre en el zip del motor." }
        Copy-Item -LiteralPath $origen -Destination $motor
        $copiados++
    }
    foreach ($patron in $Patrones) {
        $encontrados = @(Get-ChildItem -LiteralPath $extraido -Filter $patron -File)
        if ($encontrados.Count -eq 0) { throw "No hay ningún $patron en el zip del motor." }
        foreach ($archivo in $encontrados) {
            Copy-Item -LiteralPath $archivo.FullName -Destination $motor
            $copiados++
        }
    }

    # 3. llama.cpp's own license, checked too.
    $licencia = Join-Path $temporal 'LICENSE'
    Descargar $UrlLicencia $licencia
    Comprobar-Huella $licencia $Sha256Licencia 'la licencia de llama.cpp'
    Copy-Item -LiteralPath $licencia -Destination (Join-Path $motor 'LICENSE-llama.cpp')
    $copiados++

    # 4. It starts, and it is the expected build ("version: 0.9.0-dev (build 12189, commit …)"). It writes that to its
    # error output, which Windows PowerShell 5.1 would treat as a failure: errors are relaxed just for this call.
    $servidor = Join-Path $motor 'llama-server.exe'
    $antes = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    $salida = (& $servidor --version 2>&1 | ForEach-Object { "$_" } | Out-String)
    $codigo = $LASTEXITCODE
    $ErrorActionPreference = $antes
    if ($codigo -ne 0 -or $salida -notmatch "build $($Version.TrimStart('b'))\b") {
        throw "llama-server no arranca o no es la versión $Version`:`n$salida"
    }
    Write-Host "  arranca: $($salida.Trim())"

    $bytes = (Get-ChildItem -LiteralPath $motor -File | Measure-Object -Property Length -Sum).Sum
    Write-Host ("Listo: {0} archivos, {1:N1} MB en {2}" -f $copiados, ($bytes / 1MB), (Resolve-Path -LiteralPath $motor).Path)
} finally {
    Remove-Item -LiteralPath $temporal -Recurse -Force -ErrorAction SilentlyContinue
}
