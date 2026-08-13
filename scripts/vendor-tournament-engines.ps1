# Vendor third-party UCI engines into dist/engines/<id>/bin/<arch>/.
# Layout matches mujrim_protocols::catalog and package-dist-windows.ps1 Filter-Engines.
#
# Engines:
#   stockfish, reckless, plentychess, obsidian, akimbo, ethereal
# Architectures:
#   windows-x86_64 / windows-x86_64-avx2 (all of the above when published)
#   windows-aarch64 / windows-arm64 (Stockfish official WoA; others when available)
#
# Ethereal 14+ is paid-only; this script vendors the last free GitHub release (v13.00).
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

function Download-File([string]$Url, [string]$Destination) {
    if (Test-Path $Destination) {
        Write-Host "  cache hit $($Destination | Split-Path -Leaf)"
        return
    }
    Write-Host "  downloading $Url"
    Invoke-WebRequest -Uri $Url -OutFile $Destination -UserAgent "mujrim-engine-vendor"
}

function Place-Engine([string]$EngineId, [string]$ArchDir, [string]$SourceExe) {
    $destDir = Join-Path $DistEngines "$EngineId\bin\$ArchDir"
    Ensure-Dir $destDir
    $dest = Join-Path $destDir "$EngineId.exe"
    Copy-Item -Force $SourceExe $dest
    Write-Host "  placed $dest"
}

function Expand-ArchiveFindExe([string]$ArchivePath, [string]$NamePattern) {
    $extract = Join-Path $Cache ([IO.Path]::GetFileNameWithoutExtension($ArchivePath) + "_extract")
    if (Test-Path $extract) {
        Remove-Item -Recurse -Force $extract
    }
    Ensure-Dir $extract
    $ext = [IO.Path]::GetExtension($ArchivePath).ToLowerInvariant()
    if ($ext -eq ".zip") {
        Expand-Archive -Force -Path $ArchivePath -DestinationPath $extract
    } elseif ($ext -eq ".tar" -or $ArchivePath -like "*.tar.gz" -or $ArchivePath -like "*.tgz") {
        tar -xf $ArchivePath -C $extract
        if ($LASTEXITCODE -ne 0) { throw "tar extract failed for $ArchivePath" }
    } else {
        # Bare executable asset (no archive).
        return $ArchivePath
    }
    $exe = Get-ChildItem $extract -Recurse -File |
        Where-Object {
            $_.Extension -eq ".exe" -or $_.Name -match $NamePattern
        } |
        Where-Object { $_.Name -match $NamePattern } |
        Select-Object -First 1
    if (-not $exe) {
        throw "no matching executable for /$NamePattern/ in $ArchivePath"
    }
    return $exe.FullName
}

function Vendor-DirectExe {
    param(
        [string]$EngineId,
        [string]$Url,
        [string[]]$ArchDirs,
        [string]$CacheName
    )
    Write-Host "==> Vendoring $EngineId"
    $cachePath = Join-Path $Cache $CacheName
    try {
        Download-File $Url $cachePath
        foreach ($arch in $ArchDirs) {
            Place-Engine $EngineId $arch $cachePath
        }
    } catch {
        Write-Host "  warning: $EngineId not vendored: $_"
    }
}

function Vendor-ArchiveExe {
    param(
        [string]$EngineId,
        [string]$Url,
        [string[]]$ArchDirs,
        [string]$CacheName,
        [string]$NamePattern
    )
    Write-Host "==> Vendoring $EngineId"
    $cachePath = Join-Path $Cache $CacheName
    try {
        Download-File $Url $cachePath
        $exe = Expand-ArchiveFindExe $cachePath $NamePattern
        foreach ($arch in $ArchDirs) {
            Place-Engine $EngineId $arch $exe
        }
    } catch {
        Write-Host "  warning: $EngineId not vendored: $_"
    }
}

$x64Dirs = @("windows-x86_64-avx2", "windows-x86_64")
$armDirs = @("windows-aarch64", "windows-arm64")

# Stockfish 18 — official Windows x86_64 + Windows ARM64.
Vendor-ArchiveExe `
    -EngineId "stockfish" `
    -Url "https://github.com/official-stockfish/Stockfish/releases/download/sf_18/stockfish-windows-x86-64-avx2.zip" `
    -ArchDirs $x64Dirs `
    -CacheName "stockfish-windows-x86-64-avx2-sf18.zip" `
    -NamePattern "stockfish"

Vendor-ArchiveExe `
    -EngineId "stockfish" `
    -Url "https://github.com/official-stockfish/Stockfish/releases/download/sf_18/stockfish-windows-armv8-dotprod.zip" `
    -ArchDirs $armDirs `
    -CacheName "stockfish-windows-armv8-dotprod-sf18.zip" `
    -NamePattern "stockfish"

# Reckless
Vendor-DirectExe `
    -EngineId "reckless" `
    -Url "https://github.com/codedeliveryservice/Reckless/releases/download/v0.9.0/reckless-windows-avx2.exe" `
    -ArchDirs $x64Dirs `
    -CacheName "reckless-windows-avx2-v0.9.0.exe"

# PlentyChess
Vendor-DirectExe `
    -EngineId "plentychess" `
    -Url "https://github.com/Yoshie2000/PlentyChess/releases/download/b-v8.0.0/PlentyChess-8.0.0-windows-avx2.exe" `
    -ArchDirs $x64Dirs `
    -CacheName "PlentyChess-8.0.0-windows-avx2.exe"

# Obsidian
Vendor-DirectExe `
    -EngineId "obsidian" `
    -Url "https://github.com/gab8192/Obsidian/releases/download/v16.0/Obsidian160-avx2.exe" `
    -ArchDirs $x64Dirs `
    -CacheName "Obsidian160-avx2.exe"

# Akimbo
Vendor-DirectExe `
    -EngineId "akimbo" `
    -Url "https://github.com/jw1912/akimbo/releases/download/v1.0.0/akimbo-1.0.0-avx2.exe" `
    -ArchDirs $x64Dirs `
    -CacheName "akimbo-1.0.0-avx2.exe"

# Ethereal — last free GitHub Windows binaries (v13.00). Newer NNUE builds are paid.
Vendor-DirectExe `
    -EngineId "ethereal" `
    -Url "https://github.com/AndyGrant/Ethereal/releases/download/v13.00/Ethereal-avx2.exe" `
    -ArchDirs $x64Dirs `
    -CacheName "Ethereal-avx2-v13.00.exe"

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
        # Keep mujrim adapter tree intact; only refresh third-party engines here.
        if ($engineName -eq "mujrim") { return }
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
Write-Host "Note: Windows ARM64 prebuilds are only published for Stockfish among these engines."
Write-Host "      Reckless / PlentyChess / Obsidian / Akimbo / Ethereal ship x86_64 Windows builds."
Get-ChildItem $DistEngines -Recurse -Filter *.exe |
    Where-Object { $_.FullName -notmatch '\\mujrim\\' } |
    ForEach-Object {
        "{0,8:N1} MB  {1}" -f ($_.Length / 1MB), $_.FullName.Replace("$Root\", "")
    }
