[CmdletBinding()]
param(
    [string]$DataSource = $env:NVIM_GPUI_RIME_STARTER_DATA,
    [string]$Output = $(if ($env:NVIM_GPUI_RIME_RUNTIME_OUTPUT) { $env:NVIM_GPUI_RIME_RUNTIME_OUTPUT } else { '.cache\rime-runtime' }),
    [string]$WorkDir = $(if ($env:NVIM_GPUI_RIME_BUILD_DIR) { $env:NVIM_GPUI_RIME_BUILD_DIR } else { '.cache\rime-build\windows' })
)

$ErrorActionPreference = 'Stop'

function Fail([string]$Message) {
    throw "rime Windows build error: $Message"
}

function Resolve-RepoPath([string]$Value) {
    if ([string]::IsNullOrWhiteSpace($Value)) {
        return $Value
    }
    if ([System.IO.Path]::IsPathRooted($Value)) {
        return [System.IO.Path]::GetFullPath($Value)
    }
    return [System.IO.Path]::GetFullPath((Join-Path $repoRoot $Value))
}

function Assert-NotDirectory([string]$Path, [string]$Forbidden, [string]$Label) {
    $comparison = [System.StringComparison]::OrdinalIgnoreCase
    if ($Path.Equals($Forbidden, $comparison)) {
        Fail "$Label must not be $Forbidden"
    }
    $root = [System.IO.Path]::GetPathRoot($Path)
    if ($Path.Equals($root, $comparison)) {
        Fail "$Label must not be a filesystem root: $Path"
    }
}

function Assert-OutsideDirectory([string]$Path, [string]$Forbidden, [string]$Label) {
    $comparison = [System.StringComparison]::OrdinalIgnoreCase
    $forbiddenPrefix = $Forbidden.TrimEnd('\', '/') + [System.IO.Path]::DirectorySeparatorChar
    if ($Path.Equals($Forbidden, $comparison) -or $Path.StartsWith($forbiddenPrefix, $comparison)) {
        Fail "$Label must not be inside $Forbidden"
    }
}

function Invoke-Native([string]$FilePath, [string[]]$Arguments) {
    & $FilePath @Arguments
    if ($LASTEXITCODE -ne 0) {
        Fail "command failed with exit code $LASTEXITCODE`: $FilePath $($Arguments -join ' ')"
    }
}

function Invoke-Batch([string]$BatchPath, [string[]]$Arguments = @()) {
    if (-not (Test-Path -LiteralPath $BatchPath -PathType Leaf)) {
        Fail "batch file does not exist: $BatchPath"
    }
    # Calling through cmd.exe is required for a .bat file to keep its local
    # environment and to run correctly in both PowerShell 5.1 and pwsh.
    $commandLine = 'call "{0}"' -f $BatchPath
    if ($Arguments.Count -gt 0) {
        $commandLine += ' ' + ($Arguments -join ' ')
    }
    & cmd.exe /d /s /c $commandLine
    if ($LASTEXITCODE -ne 0) {
        Fail "batch file failed with exit code $LASTEXITCODE`: $BatchPath"
    }
}

$repoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
$manifest = Join-Path $repoRoot 'packaging\rime\runtime.toml'
$runtimeTool = Join-Path $repoRoot 'scripts\rime_runtime.py'

if ($env:OS -ne 'Windows_NT') {
    Fail 'this builder only runs on Windows'
}

foreach ($command in @('cmake.exe', 'cmd.exe', 'git.exe', 'python.exe')) {
    if (-not (Get-Command $command -ErrorAction SilentlyContinue)) {
        Fail "required command not found: $command"
    }
}

if (-not (Test-Path -LiteralPath $manifest -PathType Leaf)) {
    Fail "runtime manifest does not exist: $manifest"
}
if (-not (Test-Path -LiteralPath $runtimeTool -PathType Leaf)) {
    Fail "runtime staging tool does not exist: $runtimeTool"
}
if ([string]::IsNullOrWhiteSpace($DataSource)) {
    Fail 'starter data is required; pass -DataSource DIR or set NVIM_GPUI_RIME_STARTER_DATA'
}

$manifestText = Get-Content -LiteralPath $manifest -Raw
$repositoryMatch = [regex]::Match($manifestText, '(?m)^repository\s*=\s*"([^"]+)"\s*$')
$revisionMatch = [regex]::Match($manifestText, '(?m)^revision\s*=\s*"([^"]+)"\s*$')
if (-not $repositoryMatch.Success -or -not $revisionMatch.Success) {
    Fail "runtime manifest is missing source repository or revision: $manifest"
}
$sourceRepository = $repositoryMatch.Groups[1].Value
$sourceRevision = $revisionMatch.Groups[1].Value

$DataSource = Resolve-RepoPath $DataSource
$Output = Resolve-RepoPath $Output
$WorkDir = Resolve-RepoPath $WorkDir
if (-not (Test-Path -LiteralPath $DataSource -PathType Container)) {
    Fail "starter data directory does not exist: $DataSource"
}

Assert-NotDirectory $Output $repoRoot 'output'
Assert-OutsideDirectory $Output $WorkDir 'output'
Assert-NotDirectory $WorkDir $repoRoot 'work directory'

$sourceDir = Join-Path $WorkDir 'librime'
$artifactDir = Join-Path $WorkDir 'artifact'
$artifactLib = Join-Path $artifactDir 'lib'
$artifactData = Join-Path $artifactDir 'data'
$artifactModules = Join-Path $artifactDir 'modules'
$distDir = Join-Path $WorkDir 'dist'
$depsInstallDir = Join-Path $WorkDir 'deps-install'

New-Item -ItemType Directory -Force -Path $WorkDir | Out-Null
if (-not (Test-Path -LiteralPath $sourceDir)) {
    Invoke-Native 'git.exe' @('clone', '--recursive', $sourceRepository, $sourceDir)
} elseif (-not (Test-Path -LiteralPath (Join-Path $sourceDir '.git') -PathType Container)) {
    Fail "source path exists but is not a git checkout: $sourceDir"
}

Push-Location $sourceDir
try {
    Invoke-Native 'git.exe' @('fetch', '--tags', 'origin')
    Invoke-Native 'git.exe' @('checkout', '--detach', $sourceRevision)
    Invoke-Native 'git.exe' @('submodule', 'sync', '--recursive')
    Invoke-Native 'git.exe' @('submodule', 'update', '--init', '--recursive')

    # Keep the dependency and install trees in the build cache. librime's
    # official script builds third-party dependencies statically, so these do
    # not need to be copied into the application runtime.
    $boostVersion = '1.89.0'
    $env:RIME_ROOT = $sourceDir
    $env:BOOST_ROOT = Join-Path $sourceDir "deps\boost-$boostVersion"
    $env:boost_version = $boostVersion
    $env:rime_install_prefix = $distDir
    $env:deps_install_prefix = $depsInstallDir
    $env:build_config = 'Release'
    $env:build_shared = 'ON'
    $env:build_test = 'OFF'
    $env:enable_logging = 'ON'
    if ($env:NVIM_GPUI_RIME_WINDOWS_ARCH) {
        $env:ARCH = $env:NVIM_GPUI_RIME_WINDOWS_ARCH
    } else {
        $env:ARCH = 'x64'
    }

    Invoke-Batch (Join-Path $sourceDir 'install-boost.bat')
    # build.bat imports env.bat when present and its template would otherwise
    # override ARCH with the obsolete Win32/VS2019 defaults. An empty file
    # keeps the official script's environment-driven configuration intact.
    $envBatch = Join-Path $sourceDir 'env.bat'
    if (Test-Path -LiteralPath $envBatch -PathType Leaf) {
        if ((Get-Item -LiteralPath $envBatch).Length -ne 0) {
            Fail "source checkout contains a non-empty env.bat; remove it from the build cache: $envBatch"
        }
    } else {
        New-Item -ItemType File -Path $envBatch | Out-Null
    }
    # build.bat uses its first argument as the build target. Invoke the two
    # official phases separately so a cached dependency build is reusable.
    Invoke-Batch (Join-Path $sourceDir 'build.bat') @('deps')
    Invoke-Batch (Join-Path $sourceDir 'build.bat') @('librime')
} finally {
    Pop-Location
}

if (Test-Path -LiteralPath $artifactDir) {
    Remove-Item -LiteralPath $artifactDir -Recurse -Force
}
New-Item -ItemType Directory -Force -Path $artifactLib, $artifactData, $artifactModules | Out-Null

$distLib = Join-Path $distDir 'lib'
$libraryCandidates = @(
    (Join-Path $distLib 'rime.dll'),
    (Join-Path $sourceDir 'build\bin\Release\rime.dll'),
    (Join-Path $sourceDir 'build\bin\rime.dll')
)
$rimeLibrary = $libraryCandidates | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
if (-not $rimeLibrary) {
    Fail "librime did not produce rime.dll; checked: $($libraryCandidates -join ', ')"
}

# Copy every installed DLL in lib, not just the main library. This keeps the
# runtime contract valid if a future librime build enables a shared module.
if (Test-Path -LiteralPath $distLib -PathType Container) {
    Get-ChildItem -LiteralPath $distLib -Filter '*.dll' -File | ForEach-Object {
        Copy-Item -LiteralPath $_.FullName -Destination $artifactLib -Force
    }
}
if (-not (Test-Path -LiteralPath (Join-Path $artifactLib 'rime.dll') -PathType Leaf)) {
    Copy-Item -LiteralPath $rimeLibrary -Destination (Join-Path $artifactLib 'rime.dll') -Force
}

$pluginDir = Join-Path $distLib 'rime-plugins'
if (Test-Path -LiteralPath $pluginDir -PathType Container) {
    Get-ChildItem -LiteralPath $pluginDir -Force | ForEach-Object {
        Copy-Item -LiteralPath $_.FullName -Destination $artifactModules -Recurse -Force
    }
}

# Accept either a data directory or a package root containing rime-data, then
# flatten it into the runtime contract. User dictionaries never belong here.
$starterData = $DataSource
$nestedShareData = Join-Path $DataSource 'share\rime-data'
$nestedData = Join-Path $DataSource 'rime-data'
if (Test-Path -LiteralPath $nestedShareData -PathType Container) {
    $starterData = $nestedShareData
} elseif (Test-Path -LiteralPath $nestedData -PathType Container) {
    $starterData = $nestedData
}
Get-ChildItem -LiteralPath $starterData -Force | ForEach-Object {
    Copy-Item -LiteralPath $_.FullName -Destination $artifactData -Recurse -Force
}

Invoke-Native 'python.exe' @(
    $runtimeTool, 'stage', '--source', $artifactDir, '--output', $Output,
    '--platform', 'windows'
)
Invoke-Native 'python.exe' @(
    $runtimeTool, 'check', '--root', $Output, '--platform', 'windows',
    '--require-data'
)

Write-Output "built Windows Rime runtime: $Output"
