# install.ps1 — Quaxar installer for Windows
# Usage: irm https://raw.githubusercontent.com/TusharPardhe/quaxar/main/install.ps1 | iex
# The Windows installer is non-interactive.

$ErrorActionPreference = "Stop"

Write-Host ""
Write-Host "  ┌─────────────────────────────────────┐" -ForegroundColor Cyan
Write-Host "  │         Quaxar XRPL Node             │" -ForegroundColor Cyan
Write-Host "  │         Windows Installer            │" -ForegroundColor Cyan
Write-Host "  └─────────────────────────────────────┘" -ForegroundColor Cyan
Write-Host ""

# Check for Rust/Cargo
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Write-Host "  [!] Rust not found. Installing via rustup..." -ForegroundColor Yellow
    $rustupInit = "$env:TEMP\rustup-init.exe"
    Invoke-WebRequest -Uri "https://win.rustup.rs/x86_64" -OutFile $rustupInit
    & $rustupInit -y --default-toolchain stable
    $env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        Write-Host "  [X] Failed to install Rust. Please install manually: https://rustup.rs" -ForegroundColor Red
        exit 1
    }
    Write-Host "  [OK] Rust installed" -ForegroundColor Green
}

$rustVersion = (rustc --version) -replace "rustc ",""
Write-Host "  Rust        $rustVersion" -ForegroundColor Gray

# Check for Git
if (-not (Get-Command git -ErrorAction SilentlyContinue)) {
    Write-Host "  [X] Git is required. Install from https://git-scm.com/download/win" -ForegroundColor Red
    exit 1
}

# Check for C++ build tools (needed for RocksDB)
$hasVS = (Get-Command cl.exe -ErrorAction SilentlyContinue) -or
         (Test-Path "C:\Program Files\Microsoft Visual Studio") -or
         (Test-Path "C:\Program Files (x86)\Microsoft Visual Studio")
if (-not $hasVS) {
    Write-Host "  [!] Visual Studio Build Tools may be needed for native deps." -ForegroundColor Yellow
    Write-Host "      Install from: https://visualstudio.microsoft.com/visual-cpp-build-tools/" -ForegroundColor Yellow
}

# Clone or update
$cloneDir = "$env:USERPROFILE\quaxar"
if (Test-Path "$cloneDir\.git") {
    Write-Host "  Updating existing clone at $cloneDir..." -ForegroundColor Gray
    Push-Location $cloneDir
    git pull --ff-only
    $gitExitCode = $LASTEXITCODE
    Pop-Location
    if ($gitExitCode -ne 0) {
        Write-Host "  [X] Repository update failed with exit code $gitExitCode." -ForegroundColor Red
        exit $gitExitCode
    }
} else {
    Write-Host "  Cloning repository..." -ForegroundColor Gray
    git clone https://github.com/TusharPardhe/quaxar.git $cloneDir
    $gitExitCode = $LASTEXITCODE
    if ($gitExitCode -ne 0) {
        Write-Host "  [X] Repository clone failed." -ForegroundColor Red
        exit $gitExitCode
    }
}

# Build
Write-Host ""
Write-Host "  Building quaxar (this may take a few minutes)..." -ForegroundColor Cyan
Push-Location $cloneDir
cargo install --path xrpld/main --locked --force
$cargoExitCode = $LASTEXITCODE
Pop-Location
if ($cargoExitCode -ne 0) {
    Write-Host "  [X] Build failed with exit code $cargoExitCode." -ForegroundColor Red
    exit $cargoExitCode
}

# Verify
$bin = "$env:USERPROFILE\.cargo\bin\quaxar.exe"
if (Test-Path $bin) {
    Write-Host ""
    Write-Host "  [OK] quaxar installed to $bin" -ForegroundColor Green
    Write-Host ""
    Write-Host "  Usage:" -ForegroundColor Gray
    Write-Host "    quaxar --conf quaxar.cfg       Start the node" -ForegroundColor White
    Write-Host "    quaxar status                 Check node status" -ForegroundColor White
    Write-Host "    quaxar health                 Health check" -ForegroundColor White
    Write-Host "    quaxar export-snapshot -o snapshot.xrpls  Export snapshot" -ForegroundColor White
    Write-Host ""
} else {
    Write-Host "  [X] Build failed. Check output above for errors." -ForegroundColor Red
    exit 1
}
