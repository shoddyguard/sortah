<#
.SYNOPSIS
    Installs the latest sortah release from GitHub.
.DESCRIPTION
    Run as Administrator to install for all users to $env:ProgramFiles\sortah,
    or as a regular user to install to $env:LOCALAPPDATA\Programs\sortah.
    The install directory is added to the appropriate PATH scope automatically.
#>
[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'

$Owner  = 'shoddyguard'
$Repo   = 'sortah'
$Binary = 'sortah'

$Release = Invoke-RestMethod -Uri "https://api.github.com/repos/$Owner/$Repo/releases/latest" -ErrorAction Stop
$Version = $Release.tag_name

if (-not $Version)
{
    throw 'Could not determine the latest release version.'
}

$Target  = 'x86_64-pc-windows-msvc'
$Archive = "${Binary}-${Version}-${Target}.zip"
$Url     = "https://github.com/$Owner/$Repo/releases/download/$Version/$Archive"

Write-Host "Downloading $Binary $Version for $Target..."

$TempDir    = [System.IO.Path]::GetTempPath()
$ZipPath    = Join-Path $TempDir $Archive
$ExtractDir = Join-Path $TempDir "${Binary}-install"

Invoke-WebRequest -Uri $Url -OutFile $ZipPath -UseBasicParsing
Expand-Archive -Path $ZipPath -DestinationPath $ExtractDir -Force

$IsAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
    [Security.Principal.WindowsBuiltInRole]::Administrator
)

if ($IsAdmin)
{
    $InstallDir = Join-Path $env:ProgramFiles $Binary
    $PathScope  = 'Machine'
}
else
{
    $InstallDir = Join-Path $env:LOCALAPPDATA 'Programs' $Binary
    $PathScope  = 'User'
}

New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
Copy-Item -Path (Join-Path $ExtractDir "${Binary}.exe") -Destination $InstallDir -Force

$CurrentPath = [Environment]::GetEnvironmentVariable('Path', $PathScope)
if ($CurrentPath -notlike "*$InstallDir*")
{
    [Environment]::SetEnvironmentVariable('Path', "$CurrentPath;$InstallDir", $PathScope)
    Write-Host "Added $InstallDir to $PathScope PATH."
    Write-Host 'Restart your terminal for the PATH change to take effect.'
}

Remove-Item -Path $ZipPath    -Force         -ErrorAction SilentlyContinue
Remove-Item -Path $ExtractDir -Recurse -Force -ErrorAction SilentlyContinue

Write-Host ""
Write-Host "$Binary $Version installed successfully to $InstallDir."
