[CmdletBinding()]
param(
    [string]$Bundle = ''
)

$ErrorActionPreference = 'Stop'

function Fail([string]$Message) {
    throw "nvim-gpui Windows smoke test error: $Message"
}

if ([string]::IsNullOrWhiteSpace($Bundle)) {
    if ($env:NVIM_GPUI_WINDOWS_BUNDLE_OUTPUT) {
        $Bundle = $env:NVIM_GPUI_WINDOWS_BUNDLE_OUTPUT
    } else {
        $Bundle = '.cache\windows\nvim-gpui'
    }
}

if ([System.IO.Path]::IsPathRooted($Bundle)) {
    $Bundle = [System.IO.Path]::GetFullPath($Bundle)
} else {
    $Bundle = [System.IO.Path]::GetFullPath((Join-Path (Get-Location) $Bundle))
}

foreach ($name in @('nvim-gpui.exe', 'gpvim.exe')) {
    $executable = Join-Path $Bundle $name
    if (-not (Test-Path -LiteralPath $executable -PathType Leaf)) {
        Fail "executable does not exist: $executable"
    }

    $process = Start-Process `
        -FilePath $executable `
        -ArgumentList @('--version') `
        -Wait `
        -PassThru
    if ($process.ExitCode -ne 0) {
        Fail "executable failed with exit code $($process.ExitCode): $executable"
    }
}
