param(
    [string]$Bundle,
    [string]$Output,
    [string]$Iscc
)

$ErrorActionPreference = 'Stop'

function Fail([string]$Message) {
    throw "nvim-gpui Windows installer error: $Message"
}

function Resolve-RepoPath([string]$Value) {
    if ([System.IO.Path]::IsPathRooted($Value)) {
        return [System.IO.Path]::GetFullPath($Value)
    }
    return [System.IO.Path]::GetFullPath((Join-Path $repoRoot $Value))
}

function Find-Iscc {
    $command = Get-Command 'ISCC.exe' -ErrorAction SilentlyContinue
    if ($command -and $command.Path -and (Test-Path -LiteralPath $command.Path -PathType Leaf)) {
        return $command.Path
    }

    $candidates = @()
    foreach ($base in @($env:ProgramFiles, ${env:ProgramFiles(x86)}, $env:ProgramW6432)) {
        if ([string]::IsNullOrWhiteSpace($base)) {
            continue
        }
        foreach ($version in @('7', '6')) {
            $candidates += Join-Path $base "Inno Setup $version\ISCC.exe"
        }
    }

    if ($env:LOCALAPPDATA) {
        foreach ($version in @('7', '6')) {
            $candidates += Join-Path $env:LOCALAPPDATA "Programs\Inno Setup $version\ISCC.exe"
        }
        $candidates += Join-Path $env:LOCALAPPDATA 'Programs\Inno Setup\ISCC.exe'
    }

    foreach ($candidate in $candidates) {
        if (Test-Path -LiteralPath $candidate -PathType Leaf) {
            return [System.IO.Path]::GetFullPath($candidate)
        }
    }
    return $null
}

function Invoke-Native([string]$FilePath, [string[]]$Arguments) {
    & $FilePath @Arguments
    if ($LASTEXITCODE -ne 0) {
        Fail "command failed with exit code $LASTEXITCODE`: $FilePath $($Arguments -join ' ')"
    }
}

$repoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
$manifestPath = Join-Path $repoRoot 'Cargo.toml'
$scriptPath = Join-Path $PSScriptRoot 'installer.iss'

if ([string]::IsNullOrWhiteSpace($Bundle)) {
    $Bundle = $env:NVIM_GPUI_WINDOWS_BUNDLE_OUTPUT
}
if ([string]::IsNullOrWhiteSpace($Bundle)) {
    $Bundle = '.cache\windows\nvim-gpui'
}
if ([string]::IsNullOrWhiteSpace($Output)) {
    $Output = $env:NVIM_GPUI_WINDOWS_INSTALLER_OUTPUT
}
if ([string]::IsNullOrWhiteSpace($Output)) {
    $Output = 'dist\windows'
}
if ([string]::IsNullOrWhiteSpace($Iscc)) {
    $Iscc = $env:INNO_SETUP_COMPILER
}
if (-not [string]::IsNullOrWhiteSpace($Iscc) -and
    -not (Test-Path -LiteralPath $Iscc -PathType Leaf)) {
    Write-Output "configured ISCC.exe was not found: $Iscc"
    $Iscc = $null
}
if ([string]::IsNullOrWhiteSpace($Iscc)) {
    $Iscc = Find-Iscc
}
if ([string]::IsNullOrWhiteSpace($Iscc)) {
    Fail 'ISCC.exe was not found; run dev-windows.cmd or install Inno Setup'
}

$Bundle = Resolve-RepoPath $Bundle
$Output = Resolve-RepoPath $Output
if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
    Fail "Cargo.toml does not exist: $manifestPath"
}
if (-not (Test-Path -LiteralPath $scriptPath -PathType Leaf)) {
    Fail "Inno Setup script does not exist: $scriptPath"
}
if (-not (Test-Path -LiteralPath $Bundle -PathType Container)) {
    Fail "Windows bundle does not exist: $Bundle; run bundle-windows first"
}
foreach ($name in @('nvim-gpui.exe', 'gpvim.exe', 'rime')) {
    $path = Join-Path $Bundle $name
    if (-not (Test-Path -LiteralPath $path)) {
        Fail "Windows bundle is missing $name`: $path"
    }
}

$manifest = Get-Content -LiteralPath $manifestPath -Raw
$versionMatch = [regex]::Match($manifest, '(?ms)^\[package\].*?^version\s*=\s*"([^"]+)"')
if (-not $versionMatch.Success) {
    Fail "could not read the package version from $manifestPath"
}
$version = $versionMatch.Groups[1].Value

New-Item -ItemType Directory -Force -Path $Output | Out-Null
Write-Output "using ISCC: $Iscc"
Write-Output "building x86_64 Windows installer for nvim-gpui $version"

Invoke-Native $Iscc @(
    "/DAppVersion=$version",
    "/DBundleDir=$Bundle",
    "/DOutputDir=$Output",
    $scriptPath
)

$installer = Join-Path $Output "nvim-gpui-$version-setup.exe"
if (-not (Test-Path -LiteralPath $installer -PathType Leaf)) {
    Fail "ISCC completed without creating the installer: $installer"
}
Write-Output "created Windows installer: $installer"
