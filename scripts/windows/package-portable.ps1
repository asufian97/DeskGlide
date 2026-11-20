param(
    [string]$OutDir = "dist"
)

$ErrorActionPreference = "Stop"
$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$DistPath = Join-Path $RepoRoot $OutDir
New-Item -ItemType Directory -Force -Path $DistPath | Out-Null

$ReleaseDir = Join-Path $RepoRoot "target\release"
$GuiExe = Join-Path $ReleaseDir "deskglide-app.exe"
$CliExe = Join-Path $ReleaseDir "deskglide.exe"

if (-not (Test-Path $GuiExe) -or -not (Test-Path $CliExe)) {
    throw "Release binaries missing in $ReleaseDir. Run 'cargo build --release --workspace' first."
}

# 1. Portable Windows bundle
$StageWin = Join-Path $DistPath "stage-win"
Remove-Item -Recurse -Force $StageWin -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $StageWin | Out-Null

Copy-Item $GuiExe $StageWin
Copy-Item $CliExe $StageWin
Copy-Item (Join-Path $RepoRoot "README.md") $StageWin
Copy-Item (Join-Path $RepoRoot "LICENSE-MIT") $StageWin
Copy-Item (Join-Path $RepoRoot "LICENSE-APACHE") $StageWin

$WinZip = Join-Path $DistPath "DeskGlide-Windows-Portable-x64.zip"
Remove-Item -Force $WinZip -ErrorAction SilentlyContinue
Compress-Archive -Path "$StageWin\*" -DestinationPath $WinZip -CompressionLevel Optimal
Remove-Item -Recurse -Force $StageWin

# 2. Compact source archive for building/packaging on Zorin OS / Linux
$StageSrc = Join-Path $DistPath "stage-src"
Remove-Item -Recurse -Force $StageSrc -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $StageSrc | Out-Null

$ItemsToCopy = @("Cargo.toml", "Cargo.lock", "README.md", "LICENSE-MIT", "LICENSE-APACHE", "crates", "scripts", "packaging", ".github")
foreach ($Item in $ItemsToCopy) {
    $SrcItem = Join-Path $RepoRoot $Item
    if (Test-Path $SrcItem) {
        Copy-Item -Recurse -Force $SrcItem (Join-Path $StageSrc $Item)
    }
}

$SrcZip = Join-Path $DistPath "DeskGlide-Source.zip"
Remove-Item -Force $SrcZip -ErrorAction SilentlyContinue
Compress-Archive -Path "$StageSrc\*" -DestinationPath $SrcZip -CompressionLevel Optimal
Remove-Item -Recurse -Force $StageSrc

Write-Host "Created portable Windows zip: $WinZip"
Write-Host "Created compact Linux source zip: $SrcZip"
