# Assemble dual-arch Windows dist trees:
#   dist/windows-aarch64/**
#   dist/windows-x86_64/**
# Each tree is a self-contained replica (UI + engine + assets + books + nnue + engines + tools).
#
# On Arm64 hosts, x86_64 is built with llvm-mingw + x86_64-pc-windows-gnullvm (static where possible).
# Top-level dist/*.exe always match the host OS architecture.

param(
    [switch]$PackageOnly,
    [switch]$Clean
)

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $PSScriptRoot
Set-Location $Root

$TargetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $Root "target" }
$Dist = Join-Path $Root "dist"
$LlvmMingwRoot = Join-Path $env:USERPROFILE ".llvm-mingw"

function Ensure-Dir([string]$Path) {
    if (-not (Test-Path $Path)) {
        New-Item -ItemType Directory -Force -Path $Path | Out-Null
    }
}

function Copy-Tree([string]$Source, [string]$Destination) {
    if (-not (Test-Path $Source)) {
        Write-Host "  skip missing $Source"
        return
    }
    Ensure-Dir (Split-Path -Parent $Destination)
    if (Test-Path $Destination) {
        Remove-Item -Recurse -Force $Destination
    }
    Copy-Item -Recurse -Force $Source $Destination
}

function Copy-File([string]$Source, [string]$Destination) {
    if (-not (Test-Path $Source)) {
        Write-Host "  skip missing $Source"
        return
    }
    Ensure-Dir (Split-Path -Parent $Destination)
    Copy-Item -Force $Source $Destination
}

function Get-PeMachine([string]$Path) {
    $bytes = [IO.File]::ReadAllBytes((Resolve-Path $Path))
    $pe = [BitConverter]::ToInt32($bytes, 0x3c)
    return [BitConverter]::ToUInt16($bytes, $pe + 4)
}

function Assert-PeMachine([string]$Path, [uint16]$Expected, [string]$Label) {
    if (-not (Test-Path $Path)) {
        throw "missing binary for $Label : $Path"
    }
    $machine = Get-PeMachine $Path
    if ($machine -ne $Expected) {
        $got = switch ($machine) { 0x8664 { "AMD64" }; 0xAA64 { "ARM64" }; default { "0x{0:X}" -f $machine } }
        $want = switch ($Expected) { 0x8664 { "AMD64" }; 0xAA64 { "ARM64" }; default { "0x{0:X}" -f $Expected } }
        throw "PE arch mismatch for $Label : got $got, expected $want ($Path)"
    }
}

function Get-PeImports([string]$Path) {
    $readobj = Join-Path $LlvmMingwRoot "bin\llvm-readobj.exe"
    if (-not (Test-Path $readobj)) {
        return @()
    }
    $lines = & $readobj --coff-imports $Path 2>$null
    $names = @()
    foreach ($line in $lines) {
        if ($line -match 'Name:\s+(\S+)') {
            $names += $Matches[1]
        }
    }
    return $names
}

function Filter-Engines([string]$SourceEngines, [string]$DestEngines, [string[]]$KeepDirs) {
    if (-not (Test-Path $SourceEngines)) {
        return
    }
    Ensure-Dir $DestEngines
    Get-ChildItem $SourceEngines -Directory | ForEach-Object {
        $engineName = $_.Name
        $destEngine = Join-Path $DestEngines $engineName
        Ensure-Dir $destEngine
        Get-ChildItem $_.FullName -File -ErrorAction SilentlyContinue | ForEach-Object {
            Copy-Item -Force $_.FullName (Join-Path $destEngine $_.Name)
        }
        $binSrc = Join-Path $_.FullName "bin"
        if (Test-Path $binSrc) {
            $binDst = Join-Path $destEngine "bin"
            Ensure-Dir $binDst
            foreach ($keep in $KeepDirs) {
                $archSrc = Join-Path $binSrc $keep
                if (Test-Path $archSrc) {
                    Copy-Tree $archSrc (Join-Path $binDst $keep)
                }
            }
        }
    }
}

function Test-LlvmMingw() {
    return (Test-Path (Join-Path $LlvmMingwRoot "bin\x86_64-w64-mingw32-gcc.exe"))
}

function Use-LlvmMingwEnv() {
    $mingwBin = Join-Path $LlvmMingwRoot "bin"
    if (-not (Test-Path (Join-Path $mingwBin "x86_64-w64-mingw32-gcc.exe"))) {
        $gcc = Get-Command "x86_64-w64-mingw32-gcc" -ErrorAction SilentlyContinue
        if ($gcc) {
            $mingwBin = Split-Path -Parent $gcc.Source
        } else {
            throw "llvm-mingw not found (expected ~/.llvm-mingw/bin/x86_64-w64-mingw32-gcc.exe)"
        }
    }
    $env:PATH = "$mingwBin;$env:PATH"
    $env:CC_x86_64_pc_windows_gnullvm = "x86_64-w64-mingw32-clang"
    $env:CXX_x86_64_pc_windows_gnullvm = "x86_64-w64-mingw32-clang++"
    $env:AR_x86_64_pc_windows_gnullvm = "llvm-ar"
    $env:CARGO_TARGET_X86_64_PC_WINDOWS_GNULLVM_LINKER = "x86_64-w64-mingw32-clang"
    # Prefer fully static CRT/unwind so dist needs no mingw DLLs beside the exe.
    $env:RUSTFLAGS = "-C link-arg=-static -C link-arg=-lunwind"
    Write-Host "  using llvm-mingw at $mingwBin (static link flags)"
}

function Clear-BuildCaches() {
    Write-Host "==> Cleaning cargo targets and dist program binaries (keeping dist/engines)"
    if (Test-Path $TargetDir) {
        Write-Host "  removing $TargetDir"
        Remove-Item -Recurse -Force $TargetDir
    }
    $repoTarget = Join-Path $Root "target"
    if (($repoTarget -ne $TargetDir) -and (Test-Path $repoTarget)) {
        Write-Host "  removing $repoTarget"
        Remove-Item -Recurse -Force $repoTarget
    }
    # Incremental / build leftovers under the default cargo home.
    $cargoIncremental = Join-Path $env:USERPROFILE ".cargo\incremental"
    if (Test-Path $cargoIncremental) {
        Write-Host "  removing $cargoIncremental"
        Remove-Item -Recurse -Force $cargoIncremental
    }

    foreach ($name in @("windows-aarch64", "windows-x86_64", "tools")) {
        $path = Join-Path $Dist $name
        if (Test-Path $path) {
            Write-Host "  removing $path"
            Remove-Item -Recurse -Force $path
        }
    }
    Get-ChildItem $Dist -File -Filter "*.exe" -ErrorAction SilentlyContinue | ForEach-Object {
        Write-Host "  removing $($_.FullName)"
        Remove-Item -Force $_.FullName
    }

    # Refresh only Mujrim engine bins; keep third-party engines intact.
    $mujrimBins = Join-Path $Dist "engines\mujrim\bin"
    if (Test-Path $mujrimBins) {
        Write-Host "  clearing $mujrimBins (third-party engines preserved)"
        Remove-Item -Recurse -Force $mujrimBins
    }
}

function Resolve-X64Triple() {
    $hostTriple = (rustc -vV | Select-String "host:").ToString().Split(":")[1].Trim()
    if ($hostTriple -eq "x86_64-pc-windows-msvc") {
        return "x86_64-pc-windows-msvc"
    }
    if (Test-LlvmMingw) {
        return "x86_64-pc-windows-gnullvm"
    }
    return "x86_64-pc-windows-msvc"
}

function Expected-Machine([string]$Triple) {
    if ($Triple -match '^aarch64-') { return [uint16]0xAA64 }
    if ($Triple -match '^x86_64-') { return [uint16]0x8664 }
    throw "unsupported triple for PE check: $Triple"
}

function Build-Core([string]$Triple) {
    Write-Host "==> Building core packages for $Triple"
    $installed = rustup target list --installed
    if ($installed -notcontains $Triple) {
        rustup target add $Triple
    }
    $hostTriple = (rustc -vV | Select-String "host:").ToString().Split(":")[1].Trim()

    if ($Triple -eq "x86_64-pc-windows-gnullvm") {
        Use-LlvmMingwEnv
    } elseif ($Triple -ne $hostTriple) {
        Remove-Item Env:RUSTFLAGS -ErrorAction SilentlyContinue
        cargo xwin build --release --target $Triple -p mujrim -p mujrim-ui -p mujrim-benchmarker -p mujrim-tooling
        if ($LASTEXITCODE -ne 0) { throw "cargo xwin build failed for $Triple" }
        return
    } else {
        Remove-Item Env:RUSTFLAGS -ErrorAction SilentlyContinue
    }

    cargo build --release --target $Triple -p mujrim -p mujrim-ui -p mujrim-benchmarker -p mujrim-tooling
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed for $Triple" }
}

function Snapshot-ReleaseExe([string]$ReleaseDir, [string]$DestName) {
    $src = Join-Path $ReleaseDir "mujrim.exe"
    $dest = Join-Path $ReleaseDir $DestName
    $tmp = Join-Path $ReleaseDir ("_" + $DestName + ".tmp")
    if (Test-Path $tmp) { Remove-Item -Force $tmp }
    Copy-Item -Force $src $tmp
    if (Test-Path $dest) { Remove-Item -Force $dest }
    Move-Item -Force $tmp $dest
}

function Assert-EngineSize([string]$Path, [long]$MinBytes, [long]$MaxBytes, [string]$Label) {
    if (-not (Test-Path $Path)) { throw "missing $Label at $Path" }
    $size = (Get-Item $Path).Length
    if ($size -lt $MinBytes -or $size -gt $MaxBytes) {
        throw ("{0} size {1:N1} MB out of expected range [{2:N1}, {3:N1}] MB" -f `
            $Label, ($size / 1MB), ($MinBytes / 1MB), ($MaxBytes / 1MB))
    }
    Write-Host ("  {0}: {1:N1} MB" -f $Label, ($size / 1MB))
}

function Build-Variants([string]$Triple) {
    Write-Host "==> Building Mujrim product engines for $Triple"
    Write-Host "  product set: mujrim-elite, mujrim-external, mujrim-v60, mujrim-ak"
    Write-Host "  note: never add embedded-networks on top of default features (that embeds every net)"
    $release = Join-Path $TargetDir "$Triple\release"
    Ensure-Dir $release

    if ($Triple -eq "x86_64-pc-windows-gnullvm") {
        Use-LlvmMingwEnv
    } else {
        Remove-Item Env:RUSTFLAGS -ErrorAction SilentlyContinue
    }

    # mujrim-elite: ONLY Stockfish NNUE embedded (~91 MB payload)
    cargo build --release --target $Triple -p mujrim --no-default-features `
        --features "xboard,book,nnue,simd,stockfish-nnue,embedded-networks"
    if ($LASTEXITCODE -ne 0) { throw "mujrim-elite build failed for $Triple" }
    Snapshot-ReleaseExe $release "mujrim-elite.exe"
    # Engine + SF net ≈ 95–170 MB depending on arch/link.
    Assert-EngineSize (Join-Path $release "mujrim-elite.exe") 80MB 200MB "mujrim-elite"

    # mujrim-external: NO embedded networks — discovers/loads NNUE at runtime.
    cargo build --release --target $Triple -p mujrim --no-default-features `
        --features "xboard,book,nnue,simd,akimbo-nnue,stockfish-nnue,reckless-nnue"
    if ($LASTEXITCODE -ne 0) { throw "mujrim-external build failed for $Triple" }
    Snapshot-ReleaseExe $release "mujrim-external.exe"
    # Must stay far below an embedded Stockfish payload.
    Assert-EngineSize (Join-Path $release "mujrim-external.exe") 1MB 40MB "mujrim-external"

    # mujrim-v60: Reckless NNUE embedded (~60 MB payload)
    cargo build --release --target $Triple -p mujrim-native-v60 --features "syzygy,embedded-network"
    if ($LASTEXITCODE -ne 0) { throw "mujrim-v60 build failed for $Triple" }
    Assert-EngineSize (Join-Path $release "mujrim-v60.exe") 40MB 120MB "mujrim-v60"

    # mujrim-ak: ONLY Akimbo NNUE embedded (~6 MB payload)
    cargo build --release --target $Triple -p mujrim --no-default-features `
        --features "xboard,book,nnue,simd,akimbo-nnue,embedded-networks"
    if ($LASTEXITCODE -ne 0) { throw "mujrim-ak build failed for $Triple" }
    Snapshot-ReleaseExe $release "mujrim-ak.exe"
    Assert-EngineSize (Join-Path $release "mujrim-ak.exe") 1MB 40MB "mujrim-ak"

    # Top-level mujrim.exe matches external (no embedded net).
    Copy-Item -Force (Join-Path $release "mujrim-external.exe") (Join-Path $release "mujrim.exe")
    Assert-EngineSize (Join-Path $release "mujrim.exe") 1MB 40MB "mujrim.exe (external alias)"
}

function Bundle-MingwRuntime([string]$Directory) {
    $dllDir = Join-Path $LlvmMingwRoot "x86_64-w64-mingw32\bin"
    if (-not (Test-Path $dllDir)) {
        return
    }
    foreach ($dll in @("libunwind.dll", "libwinpthread-1.dll", "libc++.dll")) {
        $src = Join-Path $dllDir $dll
        if (Test-Path $src) {
            Copy-Item -Force $src (Join-Path $Directory $dll)
        }
    }
}

function Package-Arch(
    [string]$Triple,
    [string]$ArchDir,
    [string[]]$EngineArchDirs,
    [string[]]$MujrimArchDirs,
    [bool]$TopLevelAlso
) {
    Write-Host "==> Packaging $ArchDir from $Triple"
    $out = Join-Path $Dist $ArchDir
    if (Test-Path $out) {
        Remove-Item -Recurse -Force $out
    }
    Ensure-Dir $out

    $release = Join-Path $TargetDir "$Triple\release"
    if (-not (Test-Path (Join-Path $release "mujrim.exe"))) {
        throw "release artifacts missing under $release"
    }

    $expected = Expected-Machine $Triple
    Assert-PeMachine (Join-Path $release "mujrim.exe") $expected "mujrim.exe/$Triple"
    Assert-PeMachine (Join-Path $release "mujrim-ui.exe") $expected "mujrim-ui.exe/$Triple"

    Copy-File (Join-Path $release "mujrim-ui.exe") (Join-Path $out "mujrim-ui.exe")
    Copy-File (Join-Path $release "mujrim.exe") (Join-Path $out "mujrim.exe")

    Copy-Tree (Join-Path $Dist "assets") (Join-Path $out "assets")
    Copy-Tree (Join-Path $Dist "books") (Join-Path $out "books")
    Copy-Tree (Join-Path $Dist "nnue") (Join-Path $out "nnue")
    # Third-party + foreign-ISA bins (e.g. x64 engines next to ARM UI for Prism).
    Filter-Engines (Join-Path $Dist "engines") (Join-Path $out "engines") $EngineArchDirs

    $mujrimVariants = @(
        "mujrim-elite.exe",
        "mujrim-external.exe",
        "mujrim-v60.exe",
        "mujrim-ak.exe"
    )
    foreach ($name in $mujrimVariants) {
        $src = Join-Path $release $name
        if (-not (Test-Path $src)) {
            throw "missing product engine $name under $release"
        }
        Assert-PeMachine $src $expected "$name/$Triple"
        $minBytes = 1MB
        $maxBytes = 40MB
        if ($name -eq "mujrim-elite.exe") { $minBytes = 80MB; $maxBytes = 200MB }
        elseif ($name -eq "mujrim-v60.exe") { $minBytes = 40MB; $maxBytes = 120MB }
        Assert-EngineSize $src $minBytes $maxBytes "$name (release/$Triple)"
    }
    # Only stamp mujrim products into arches matching this build's PE machine.
    foreach ($engineArch in $MujrimArchDirs) {
        $mujrimEngineBin = Join-Path $out "engines\mujrim\bin\$engineArch"
        Ensure-Dir $mujrimEngineBin
        Get-ChildItem $mujrimEngineBin -File -Filter "mujrim*.exe" -ErrorAction SilentlyContinue |
            Remove-Item -Force
        foreach ($name in $mujrimVariants) {
            Copy-File (Join-Path $release $name) (Join-Path $mujrimEngineBin $name)
        }
        $sharedMujrimBin = Join-Path $Dist "engines\mujrim\bin\$engineArch"
        Ensure-Dir $sharedMujrimBin
        Get-ChildItem $sharedMujrimBin -File -Filter "mujrim*.exe" -ErrorAction SilentlyContinue |
            Remove-Item -Force
        foreach ($name in $mujrimVariants) {
            Copy-File (Join-Path $release $name) (Join-Path $sharedMujrimBin $name)
        }
        if ($Triple -eq "x86_64-pc-windows-gnullvm") {
            Bundle-MingwRuntime $mujrimEngineBin
            Bundle-MingwRuntime $sharedMujrimBin
        }
    }

    # For gnullvm builds, ship runtime DLLs next to every exe directory as a safety net
    # (static link should remove the need, but missing DLLs produce 0xc000007b).
    if ($Triple -eq "x86_64-pc-windows-gnullvm") {
        Bundle-MingwRuntime $out
        $imports = Get-PeImports (Join-Path $out "mujrim.exe")
        if ($imports -contains "libunwind.dll") {
            Write-Host "  warning: mujrim.exe still imports libunwind.dll; bundled runtime DLLs beside binaries"
        } else {
            Write-Host "  mujrim.exe has no libunwind.dll import (static link ok)"
        }
    }

    Filter-Engines (Join-Path $out "engines") (Join-Path $Dist "engines") $EngineArchDirs

    $toolsOut = Join-Path $out "tools"
    Ensure-Dir $toolsOut
    $benchSrc = Join-Path $release "mujrim-benchmarker.exe"
    if (Test-Path $benchSrc) {
        Assert-PeMachine $benchSrc $expected "mujrim-benchmarker.exe/$Triple"
        $benchDir = Join-Path $toolsOut "benchmarker\bin\$ArchDir"
        Ensure-Dir $benchDir
        Copy-File $benchSrc (Join-Path $benchDir "mujrim-benchmarker.exe")
        if ($Triple -eq "x86_64-pc-windows-gnullvm") { Bundle-MingwRuntime $benchDir }
        $rootBench = Join-Path $Dist "tools\benchmarker\bin\$ArchDir"
        Ensure-Dir $rootBench
        Copy-File $benchSrc (Join-Path $rootBench "mujrim-benchmarker.exe")
        if ($Triple -eq "x86_64-pc-windows-gnullvm") { Bundle-MingwRuntime $rootBench }
    }
    $toolSrc = Join-Path $release "mujrim-tooling.exe"
    if (Test-Path $toolSrc) {
        Assert-PeMachine $toolSrc $expected "mujrim-tooling.exe/$Triple"
        $toolDir = Join-Path $toolsOut "tooling\bin\$ArchDir"
        Ensure-Dir $toolDir
        Copy-File $toolSrc (Join-Path $toolDir "mujrim-tooling.exe")
        if ($Triple -eq "x86_64-pc-windows-gnullvm") { Bundle-MingwRuntime $toolDir }
        $rootTool = Join-Path $Dist "tools\tooling\bin\$ArchDir"
        Ensure-Dir $rootTool
        Copy-File $toolSrc (Join-Path $rootTool "mujrim-tooling.exe")
        if ($Triple -eq "x86_64-pc-windows-gnullvm") { Bundle-MingwRuntime $rootTool }
    }

    if ($TopLevelAlso) {
        Copy-File (Join-Path $out "mujrim-ui.exe") (Join-Path $Dist "mujrim-ui.exe")
        Copy-File (Join-Path $out "mujrim.exe") (Join-Path $Dist "mujrim.exe")
        if ($Triple -eq "x86_64-pc-windows-gnullvm") {
            Bundle-MingwRuntime $Dist
        }
        Assert-PeMachine (Join-Path $Dist "mujrim.exe") $expected "dist/mujrim.exe"
        Assert-PeMachine (Join-Path $Dist "mujrim-ui.exe") $expected "dist/mujrim-ui.exe"
    }

    Write-Host "  packaged $out"
}

$hostArch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
Write-Host "Host architecture: $hostArch"
Write-Host "Cargo target dir: $TargetDir"

if ($Clean) {
    Clear-BuildCaches
    # After cleaning, recreate a fresh target dir path for this run.
    Ensure-Dir $TargetDir
    $env:CARGO_TARGET_DIR = $TargetDir
}

$x64Triple = Resolve-X64Triple
Write-Host "Selected x86_64 triple: $x64Triple"

if (-not $PackageOnly) {
    Build-Core "aarch64-pc-windows-msvc"
    Build-Variants "aarch64-pc-windows-msvc"
    Build-Core $x64Triple
    Build-Variants $x64Triple
} else {
    Write-Host "==> PackageOnly: skipping cargo builds"
}

# Ensure third-party engines exist under dist/engines/<id>/bin/<arch>/ before filtering.
$vendorScript = Join-Path $PSScriptRoot "vendor-tournament-engines.ps1"
$requiredEngines = @("stockfish", "reckless", "plentychess", "obsidian", "akimbo", "ethereal")
$missingEngines = @($requiredEngines | Where-Object {
        -not (Test-Path (Join-Path $Dist "engines\$_\bin"))
    })
if ((Test-Path $vendorScript) -and $missingEngines.Count -gt 0) {
    Write-Host "==> Vendoring tournament engines into dist/engines (missing: $($missingEngines -join ', '))"
    try {
        & $vendorScript
    } catch {
        Write-Host "  warning: engine vendor skipped: $_"
    }
}

Package-Arch `
    -Triple "aarch64-pc-windows-msvc" `
    -ArchDir "windows-aarch64" `
    -EngineArchDirs @("windows-aarch64", "windows-arm64", "windows-x86_64-avx2", "windows-x86_64") `
    -MujrimArchDirs @("windows-aarch64", "windows-arm64") `
    -TopLevelAlso ($hostArch -eq "Arm64")

Package-Arch `
    -Triple $x64Triple `
    -ArchDir "windows-x86_64" `
    -EngineArchDirs @("windows-x86_64-avx2", "windows-x86_64") `
    -MujrimArchDirs @("windows-x86_64-avx2", "windows-x86_64") `
    -TopLevelAlso ($hostArch -ne "Arm64")

Write-Host "==> Dist dual-arch packaging complete"
Get-ChildItem $Dist -Directory | Select-Object Name
Get-ChildItem (Join-Path $Dist "windows-aarch64") -ErrorAction SilentlyContinue | Select-Object Name
Get-ChildItem (Join-Path $Dist "windows-x86_64") -ErrorAction SilentlyContinue | Select-Object Name
