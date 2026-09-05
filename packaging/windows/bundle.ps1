[CmdletBinding()]
param(
    [string]$Runtime = $(if ($env:NVIM_GPUI_RIME_RUNTIME_OUTPUT) { $env:NVIM_GPUI_RIME_RUNTIME_OUTPUT } else { '.cache\rime-runtime' }),
    [string]$Output = $(if ($env:NVIM_GPUI_WINDOWS_BUNDLE_OUTPUT) { $env:NVIM_GPUI_WINDOWS_BUNDLE_OUTPUT } else { '.cache\windows\nvim-gpui' })
)

$ErrorActionPreference = 'Stop'

function Fail([string]$Message) {
    throw "nvim-gpui Windows bundle error: $Message"
}

function Resolve-RepoPath([string]$Value) {
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

function Invoke-Native([string]$FilePath, [string[]]$Arguments) {
    & $FilePath @Arguments
    if ($LASTEXITCODE -ne 0) {
        Fail "command failed with exit code $LASTEXITCODE`: $FilePath $($Arguments -join ' ')"
    }
}

$repoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
$runtimeTool = Join-Path $repoRoot 'scripts\rime_runtime.py'
$Runtime = Resolve-RepoPath $Runtime
$Output = Resolve-RepoPath $Output

if ($env:OS -ne 'Windows_NT') {
    Fail 'this bundle task only runs on Windows'
}
foreach ($command in @('cargo.exe', 'python.exe')) {
    if (-not (Get-Command $command -ErrorAction SilentlyContinue)) {
        Fail "required command not found: $command"
    }
}
if (-not (Test-Path -LiteralPath $runtimeTool -PathType Leaf)) {
    Fail "runtime staging tool does not exist: $runtimeTool"
}
if (-not (Test-Path -LiteralPath $Runtime -PathType Container)) {
    Fail "staged Rime runtime does not exist: $Runtime; run rime-runtime-windows first"
}

Assert-NotDirectory $Output $repoRoot 'output'
Invoke-Native 'python.exe' @(
    $runtimeTool, 'check', '--root', $Runtime, '--platform', 'windows',
    '--require-data'
)

Invoke-Native 'cargo.exe' @('build', '--release', '--bins')

$cargoTarget = if ($env:CARGO_TARGET_DIR) {
    Resolve-RepoPath $env:CARGO_TARGET_DIR
} else {
    Join-Path $repoRoot 'target'
}
$releaseDir = Join-Path $cargoTarget 'release'
$nvimExecutable = Join-Path $releaseDir 'nvim-gpui.exe'
$gpvimExecutable = Join-Path $releaseDir 'gpvim.exe'
foreach ($executable in @($nvimExecutable, $gpvimExecutable)) {
    if (-not (Test-Path -LiteralPath $executable -PathType Leaf)) {
        Fail "release executable does not exist: $executable"
    }
}

if (Test-Path -LiteralPath $Output) {
    # The runtime data is intentionally read-only; clear that attribute only
    # inside the exact generated output before replacing the directory.
    Get-ChildItem -LiteralPath $Output -Recurse -Force | ForEach-Object {
        $_.Attributes = $_.Attributes -band (-bnot [System.IO.FileAttributes]::ReadOnly)
    }
    Remove-Item -LiteralPath $Output -Recurse -Force
}
New-Item -ItemType Directory -Force -Path $Output | Out-Null
Copy-Item -LiteralPath $nvimExecutable -Destination (Join-Path $Output 'nvim-gpui.exe')
Copy-Item -LiteralPath $gpvimExecutable -Destination (Join-Path $Output 'gpvim.exe')
Copy-Item -LiteralPath $Runtime -Destination (Join-Path $Output 'rime') -Recurse -Force

Invoke-Native 'python.exe' @(
    $runtimeTool, 'check', '--root', (Join-Path $Output 'rime'),
    '--platform', 'windows', '--require-data'
)

Write-Output "created Windows bundle: $Output"
