# Vendor third-party UCI engines into dist/engines/<id>/bin/<arch>/ next to packaging trees.
# Layout matches mujrim_protocols::catalog discovery and package-dist-windows.ps1 Filter-Engines.
#
# Currently vendors Stockfish for:
#   - windows-x86_64 / windows-x86_64-avx2 (official release)
#   - windows-aarch64 / windows-arm64 (community WoA build when available)
#
# Usage:
#   powershell -File scripts/vendor-tournament-engines.ps1

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $PSScriptRoot
$DistEngines = Join-Path $Root "dist\engines"
$Cache = Join-Path $Root "target\engine-vendor-cache"
New-Item -ItemType Directory -Force -Path $Cache | Out-Null
New-Item -ItemType Directory -Force -Path $DistEngines | Out-Null

function Ensure-Dir([string]$Path) {
    if (-not (Test-Path $Path)) {
        New-Item -ItemType Directory -Force -Path $Path | Out-Null
    }
}

function Place-Engine([string]$EngineId, [string]$ArchDir, [string]$SourceExe) {
    $destDir = Join-Path $DistEngines "$EngineId\bin\$ArchDir"
    Ensure-Dir $destDir
    $dest = Join-Path $destDir "$EngineId.exe"
    Copy-Item -Force $SourceExe $dest
    Write-Host "  placed $dest"
}

function Expand-ZipFindExe([string]$ZipPath, [string]$NamePattern) {
    $extract = Join-Path $Cache ([IO.Path]::GetFileNameWithoutExtension($ZipPath))
    if (Test-Path $extract) {
        Remove-Item -Recurse -Force $extract
    }
    Expand-Archive -Force -Path $ZipPath -DestinationPath $extract
    $exe = Get-ChildItem $extract -Recurse -Filter *.exe |
        Where-Object { $_.Name -match $NamePattern } |
        Select-Object -First 1
    if (-not $exe) {
        throw "no matching exe in $ZipPath"
    }
    return $exe.FullName
}

Write-Host "==> Vendoring Stockfish (Windows x86_64)"
$sfTag = "sf_17.1"
$sfZipName = "stockfish-windows-x86-64-avx2.zip"
$sfUrl = "https://github.com/official-stockfish/Stockfish/releases/download/$sfTag/$sfZipName"
$sfZip = Join-Path $Cache $sfZipName
if (-not (Test-Path $sfZip)) {
    Write-Host "  downloading $sfUrl"
    Invoke-WebRequest -Uri $sfUrl -OutFile $sfZip
}
$sfExe = Expand-ZipFindExe $sfZip "stockfish"
Place-Engine "stockfish" "windows-x86_64-avx2" $sfExe
Place-Engine "stockfish" "windows-x86_64" $sfExe

Write-Host "==> Vendoring Stockfish (Windows ARM64 / community build)"
$armUrl = "https://sjeng.org/dl/stockfish17_arm8dotprod_win.zip"
$armZip = Join-Path $Cache "stockfish17_arm8dotprod_win.zip"
try {
    if (-not (Test-Path $armZip)) {
        Write-Host "  downloading $armUrl"
        Invoke-WebRequest -Uri $armUrl -OutFile $armZip
    }
    $armExe = Expand-ZipFindExe $armZip "stockfish"
    Place-Engine "stockfish" "windows-aarch64" $armExe
    Place-Engine "stockfish" "windows-arm64" $armExe
} catch {
    Write-Host "  warning: Windows ARM64 Stockfish not vendored: $_"
}

# Mirror into per-arch dist trees when present (next to mujrim-ui.exe).
foreach ($arch in @("windows-aarch64", "windows-x86_64")) {
    $archRoot = Join-Path $Root "dist\$arch"
    if (-not (Test-Path $archRoot)) { continue }
    $keep = if ($arch -eq "windows-aarch64") {
        @("windows-aarch64", "windows-arm64")
    } else {
        @("windows-x86_64-avx2", "windows-x86_64")
    }
    $destEngines = Join-Path $archRoot "engines"
    Write-Host "==> Syncing engines into $destEngines"
    Ensure-Dir $destEngines
    Get-ChildItem (Join-Path $DistEngines "*") -Directory | ForEach-Object {
        $engineName = $_.Name
        $destEngine = Join-Path $destEngines $engineName
        Ensure-Dir $destEngine
        Get-ChildItem $_.FullName -File -ErrorAction SilentlyContinue | ForEach-Object {
            Copy-Item -Force $_.FullName (Join-Path $destEngine $_.Name)
        }
        $binSrc = Join-Path $_.FullName "bin"
        if (Test-Path $binSrc) {
            $binDst = Join-Path $destEngine "bin"
            Ensure-Dir $binDst
            foreach ($keepDir in $keep) {
                $archSrc = Join-Path $binSrc $keepDir
                if (Test-Path $archSrc) {
                    $archDst = Join-Path $binDst $keepDir
                    if (Test-Path $archDst) { Remove-Item -Recurse -Force $archDst }
                    Copy-Item -Recurse -Force $archSrc $archDst
                }
            }
        }
    }
}

Write-Host "==> Engine vendor complete"
Get-ChildItem $DistEngines -Recurse -Filter *.exe | Select-Object -ExpandProperty FullName
