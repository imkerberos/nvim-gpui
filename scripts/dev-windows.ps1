[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'

function Fail([string]$Message) {
    throw "nvim-gpui Windows development setup error: $Message"
}

function Invoke-WingetPackage {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Id,
        [Parameter(Mandatory = $true)]
        [string]$Name
    )

    & winget.exe list --id $Id --exact --accept-source-agreements *> $null
    if ($LASTEXITCODE -eq 0) {
        Write-Output "already installed: $Name"
        return
    }

    Write-Output "installing: $Name"
    & winget.exe install --id $Id --exact --source winget `
        --accept-source-agreements --accept-package-agreements
    if ($LASTEXITCODE -ne 0) {
        Fail "winget could not install $Name ($Id)"
    }
}

function Refresh-ProcessPath {
    $machinePath = [Environment]::GetEnvironmentVariable('Path', 'Machine')
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    $paths = @($machinePath, $userPath, $env:Path) | Where-Object { $_ }
    $env:Path = $paths -join ';'
}

function Find-VsWhere {
    $paths = @()
    if ($env:ProgramFiles) {
        $paths += Join-Path $env:ProgramFiles 'Microsoft Visual Studio\Installer\vswhere.exe'
    }
    if (${env:ProgramFiles(x86)}) {
        $paths += Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    }
    foreach ($path in $paths) {
        if (Test-Path -LiteralPath $path -PathType Leaf) {
            return $path
        }
    }
    return $null
}

function Find-VisualStudioInstallation {
    $vswhere = Find-VsWhere
    if (-not $vswhere) {
        return $null
    }

    $installationPath = & $vswhere -latest -products * -property installationPath 2>$null
    if ($LASTEXITCODE -ne 0 -or -not $installationPath) {
        return $null
    }
    return ($installationPath | Select-Object -First 1).ToString().Trim()
}

function Find-MsvcLinker {
    $installationPath = Find-VisualStudioInstallation
    if (-not $installationPath) {
        return $null
    }

    $toolchainRoot = Join-Path $installationPath 'VC\Tools\MSVC'
    if (-not (Test-Path -LiteralPath $toolchainRoot -PathType Container)) {
        return $null
    }

    $linker = Get-ChildItem -LiteralPath $toolchainRoot -Directory |
        Sort-Object Name -Descending |
        ForEach-Object { Join-Path $_.FullName 'bin\Hostx64\x64\link.exe' } |
        Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } |
        Select-Object -First 1
    if ($linker) {
        return $linker.ToString()
    }
    return $null
}

function Ensure-VisualStudioCppTools {
    $vswhere = Find-VsWhere
    $installationPath = Find-VisualStudioInstallation
    if ($installationPath) {
        $setup = Join-Path (Split-Path $vswhere -Parent) 'setup.exe'
        if (-not (Test-Path -LiteralPath $setup -PathType Leaf)) {
            Fail "Visual Studio is installed, but its installer was not found: $setup"
        }

        Write-Output 'ensuring Visual Studio C++ and CMake components'
        & $setup modify --installPath $installationPath `
            --add Microsoft.VisualStudio.Workload.VCTools `
            --add Microsoft.VisualStudio.Component.VC.Tools.x86.x64 `
            --add Microsoft.VisualStudio.Component.VC.CMake.Project `
            --includeRecommended --quiet --norestart --wait
        if ($LASTEXITCODE -ne 0) {
            Fail 'Visual Studio could not be modified with the C++ workload'
        }
        return
    }

    Write-Output 'installing Visual Studio 2022 Build Tools with C++ workload'
    & winget.exe install --id Microsoft.VisualStudio.2022.BuildTools --exact `
        --architecture x64 `
        --source winget --accept-source-agreements --accept-package-agreements `
        --override '--wait --passive --norestart --add Microsoft.VisualStudio.Workload.VCTools --add Microsoft.VisualStudio.Component.VC.Tools.x86.x64 --add Microsoft.VisualStudio.Component.VC.CMake.Project --add Microsoft.VisualStudio.Component.Windows10SDK.20348 --includeRecommended'
    if ($LASTEXITCODE -ne 0) {
        Fail 'winget could not install Visual Studio 2022 Build Tools'
    }
}

$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = New-Object Security.Principal.WindowsPrincipal($identity)
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    Fail 'run this task from an elevated PowerShell or Command Prompt terminal'
}

if (-not (Get-Command winget.exe -ErrorAction SilentlyContinue)) {
    Fail 'winget.exe is required; install or update App Installer from Microsoft Store'
}

Ensure-VisualStudioCppTools

$packages = @(
    @{ Id = 'Git.Git'; Name = 'Git for Windows' },
    @{ Id = 'Kitware.CMake'; Name = 'CMake' },
    @{ Id = 'Python.Python.3.11'; Name = 'Python 3.11' },
    @{ Id = 'Rustlang.Rustup'; Name = 'Rustup' },
    @{ Id = 'Casey.Just'; Name = 'just' },
    @{ Id = 'Neovim.Neovim'; Name = 'Neovim' },
    @{ Id = 'GitHub.cli'; Name = 'GitHub CLI' },
    @{ Id = '7zip.7zip'; Name = '7-Zip' },
    @{ Id = 'aria2.aria2'; Name = 'aria2' }
)

foreach ($package in $packages) {
    Invoke-WingetPackage -Id $package.Id -Name $package.Name
}

Refresh-ProcessPath
$rustup = Get-Command rustup.exe -ErrorAction SilentlyContinue
if ($rustup) {
    Write-Output 'ensuring the stable Rust toolchain'
    & $rustup.Source toolchain install stable --profile minimal
    if ($LASTEXITCODE -ne 0) {
        Fail 'rustup could not install the stable toolchain'
    }
    & $rustup.Source default stable
    if ($LASTEXITCODE -ne 0) {
        Fail 'rustup could not select the stable toolchain'
    }

    Write-Output 'ensuring Rust MSVC target: x86_64-pc-windows-msvc'
    & $rustup.Source target add x86_64-pc-windows-msvc
    if ($LASTEXITCODE -ne 0) {
        Fail 'rustup could not install x86_64-pc-windows-msvc'
    }
}

$linker = Find-MsvcLinker
if (-not $linker) {
    Fail 'MSVC x64 linker was not found after installing Visual Studio Build Tools'
}
$linkerVariable = 'CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER'
[Environment]::SetEnvironmentVariable($linkerVariable, $linker, 'User')
Set-Item -Path "Env:$linkerVariable" -Value $linker
Write-Output "configured $linkerVariable = $linker"

Write-Output ''
Write-Output 'Windows development prerequisites are configured.'
Write-Output 'Restart the terminal so newly installed commands are available.'
Write-Output 'Then start an x64 Native Tools Command Prompt for VS 2022 and use PowerShell from it.'
