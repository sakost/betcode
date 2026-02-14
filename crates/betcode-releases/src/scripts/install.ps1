#Requires -Version 5.1
param(
    [string]$Binary = "betcode",
    [string]$InstallDir = "$env:USERPROFILE\.betcode\bin"
)

$ErrorActionPreference = "Stop"
$Repo = "REPO_PLACEHOLDER"

# Only client binaries on Windows
if ($Binary -in @("betcode-relay", "betcode-setup")) {
    Write-Error "$Binary is only available on Linux"
    exit 1
}

$Arch = "amd64"
$Platform = "windows-$Arch"
$Url = "https://github.com/$Repo/releases/latest/download/$Binary-$Platform.zip"

Write-Host "Installing $Binary ($Platform)..."

$TmpDir = Join-Path $env:TEMP "betcode-install-$(Get-Random)"
New-Item -ItemType Directory -Path $TmpDir -Force | Out-Null

try {
    $ZipPath = Join-Path $TmpDir "$Binary.zip"
    Invoke-WebRequest -Uri $Url -OutFile $ZipPath -UseBasicParsing
    Expand-Archive -Path $ZipPath -DestinationPath $TmpDir -Force

    if (-not (Test-Path $InstallDir)) {
        New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    }

    $ExePath = Join-Path $TmpDir "$Binary.exe"
    $Dest = Join-Path $InstallDir "$Binary.exe"
    Move-Item -Path $ExePath -Destination $Dest -Force

    # Add to PATH if not already there
    $UserPath = [Environment]::GetEnvironmentVariable("PATH", "User")
    if ($UserPath -notlike "*$InstallDir*") {
        [Environment]::SetEnvironmentVariable("PATH", "$UserPath;$InstallDir", "User")
        Write-Host "Added $InstallDir to PATH (restart terminal to use)"
    }

    Write-Host "Installed $Binary to $Dest"
} finally {
    Remove-Item -Path $TmpDir -Recurse -Force -ErrorAction SilentlyContinue
}
