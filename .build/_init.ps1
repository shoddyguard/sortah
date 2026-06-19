<#
.SYNOPSIS
    Initializes this repository
.NOTES
    !!! THIS FILE IS MAINTAINED BY A TOOL !!!
    !!! MANUAL CHANGES WILL BE LOST UNLESS IN THE "user defined _init steps" SECTION  BELOW !!!
    !!! YOU CAN UPDATE THIS FILE BY RUNNING `Update-BrownserveRepository` IN THE ROOT OF THE REPOSITORY !!!

    This script is responsible for setting up the repository for use, it will:
        - Set up various well-known global variables to store important paths for the repository
        - Ensure ephemeral paths are recreated on each run
        - Install paket dependencies
        - Install the Brownserve.PSTools module
        - Install any external tooling
        - Set up any package aliases
        - Run any custom init steps
        - Output a list of commands that are now available to the user along with their synopsis
#>
#Requires -Version 6.0
[CmdletBinding()]
param
(
    # If set will disable the cmdlet output at the end of the script
    [Parameter(
        Mandatory = $false
    )]
    [switch]
    $SuppressOutput
)
# Stop on errors
$ErrorActionPreference = 'Stop'

Write-Host 'Initialising repository, please wait...'

# We use this well-known global variable across a variety of projects for storing cmdlet names/summaries so we can output them if desired.
$Global:BrownserveCmdlets = @()

# If we're on Teamcity set the well-known $env:CI variable, this is set on most other CI/CD providers but not Teamcity :(
if ($env:TEAMCITY_VERSION)
{
    Write-Verbose 'Running on Teamcity, setting $env:CI'
    $env:CI = $true
}

# Suppress output on CI/CD - it's noisy
if ($env:CI)
{
    $SuppressOutput = $true
}

# Set up our permanent paths
# This directory is the root of the repo, it's handy to reference sometimes
$Global:BrownserveRepoRootDirectory = (Resolve-Path (Get-Item $PSScriptRoot -Force).PSParentPath) | Convert-Path # -Force flag is needed to find dot folders on *.nix
# Contains build configuration along with this _init.ps1 script
$Global:BrownserveRepoBuildDirectory = Join-Path $global:BrownserveRepoRootDirectory -ChildPath '.build' | Convert-Path
# Stores any tasks that we pass to Invoke-Build
$Global:BrownserveRepoBuildTasksDirectory = Join-Path $global:BrownserveRepoRootDirectory -ChildPath '.build' -AdditionalChildPath 'tasks' | Convert-Path
# Stores any tests that we pass to Pester
$Global:BrownserveRepoTestsDirectory = Join-Path $global:BrownserveRepoRootDirectory -ChildPath '.build' -AdditionalChildPath 'tests' | Convert-Path



# Guess the name of the repo by it's name on disk
$Global:BrownserveRepoName = Split-Path $Global:BrownserveRepoRootDirectory -Leaf

# Set-up our ephemeral paths, that is those that will be destroyed and then recreated each time this script is called
# EphemeralDirectories are destroyed and recreated by this script
# EphemeralFiles are destroyed but are not recreated, we assume whatever created them in the first place will do so again (e.g. paket.lock)
$EphemeralDirectories = @(
    ($BrownserveRepoTempDirectory = Join-Path -Path $global:BrownserveRepoRootDirectory -ChildPath '.tmp'),
    ($BrownserveRepoLogDirectory = Join-Path -Path $global:BrownserveRepoRootDirectory -ChildPath '.tmp' -AdditionalChildPath 'logs'),
    ($BrownserveRepoBuildOutputDirectory = Join-Path -Path $global:BrownserveRepoRootDirectory -ChildPath '.tmp' -AdditionalChildPath 'output'),
    ($BrownserveRepoBinaryDirectory = Join-Path -Path $global:BrownserveRepoRootDirectory -ChildPath '.tmp' -AdditionalChildPath 'bin'),
    ($BrownserveRepoNugetPackagesDirectory = Join-Path -Path $global:BrownserveRepoRootDirectory -ChildPath 'packages'),
    ($BrownserveRepoPaketFilesDirectory = Join-Path -Path $global:BrownserveRepoRootDirectory -ChildPath 'paket-files')
)
$EphemeralFiles = @(
    Join-Path -Path $global:BrownserveRepoRootDirectory -ChildPath 'paket.lock'
)
try
{
    if ($EphemeralDirectories.Count -gt 0)
    {
        Write-Verbose 'Destroying any preexisting ephemeral directories'
        $EphemeralDirectories | ForEach-Object {
            if ((Test-Path $_))
            {
                Remove-Item $_ -Recurse -Force | Out-Null
            }
            New-Item $_ -ItemType Directory -Force | Out-Null
        }
    }
    if ($EphemeralFiles.Count -gt 0)
    {
        Write-Verbose 'Destroying any preexisting ephemeral files'
        $EphemeralFiles | ForEach-Object {
            if ((Test-Path $_))
            {
                Remove-Item $_ -Recurse -Force | Out-Null
            }
        }
    }
}
catch
{
    throw "Failed to process ephemeral paths.`n$($_.Exception.Message)"
}

# Now that the ephemeral paths definitely exist we are free to set their global variables

# Used to store temporary files created for builds/tests
$global:BrownserveRepoTempDirectory = $BrownserveRepoTempDirectory | Convert-Path
# Used to store build logs, output from Invoke-NativeCommand and the like
$global:BrownserveRepoLogDirectory = $BrownserveRepoLogDirectory | Convert-Path
# Used to store any output from builds (e.g. Terraform plans, MSBuild artifacts etc)
$global:BrownserveRepoBuildOutputDirectory = $BrownserveRepoBuildOutputDirectory | Convert-Path
# Used to store any downloaded/copied binaries required for builds, cmdlets like Get-Vault make use of this variable
$global:BrownserveRepoBinaryDirectory = $BrownserveRepoBinaryDirectory | Convert-Path
# Paket/nuget will restore their dependencies to this directory, case sensitive on Linux
$global:BrownserveRepoNugetPackagesDirectory = $BrownserveRepoNugetPackagesDirectory | Convert-Path
# Paket will restore certain types of dependencies to this directory, case sensitive on Linux
$global:BrownserveRepoPaketFilesDirectory = $BrownserveRepoPaketFilesDirectory | Convert-Path


# We use paket for managing our dependencies and we get that via dotnet
Push-Location
Set-Location $Global:BrownserveRepoRootDirectory
Write-Verbose 'Restoring dotnet tools'
$DotnetOutput = & dotnet tool restore
if ($LASTEXITCODE -ne 0)
{
    Pop-Location
    $DotnetOutput
    throw 'dotnet tool restore failed'
}

Write-Verbose 'Installing paket dependencies'
$PaketOutput = & dotnet paket install
if ($LASTEXITCODE -ne 0)
{
    Pop-Location
    $PaketOutput
    throw 'Failed to install paket dependencies'
}
Pop-Location

# Import Brownserve.PSCommon, Brownserve.PSSourceControl, and Brownserve.PSBuildTools from the
# downloaded packages. These provide all build tooling needed by this repository.
# Load order matters: PSCommon first, then PSSourceControl, then PSBuildTools.
foreach ($ModuleSpec in @(
    @{ Name = 'Brownserve.PSCommon';        Filter = 'brownserve.pscommon.psd1' },
    @{ Name = 'Brownserve.PSSourceControl'; Filter = 'brownserve.pssourcecontrol.psd1' },
    @{ Name = 'Brownserve.PSBuildTools';    Filter = 'brownserve.psbuildtools.psd1' }
))
{
    if ((Get-Module $ModuleSpec.Name))
    {
        try
        {
            Write-Warning "The $($ModuleSpec.Name) module is already loaded in this PSSession and will be unloaded to ensure the correct version for this repository is used"
            Remove-Module $ModuleSpec.Name -Force -Confirm:$false -Verbose:$false
        }
        catch
        {
            throw "Failed to unload $($ModuleSpec.Name).`n$($_.Exception.Message)"
        }
    }
    try
    {
        Write-Verbose "Importing $($ModuleSpec.Name) module"
        $ModulePath = Get-ChildItem `
            -Filter $ModuleSpec.Filter `
            -Recurse `
            -Path (Join-Path $global:BrownserveRepoNugetPackagesDirectory $ModuleSpec.Name) `
            -ErrorAction 'Stop'
        if ($ModulePath.Count -gt 1)
        {
            throw "Found more than one instance of $($ModuleSpec.Name)!"
        }
        if (!$ModulePath)
        {
            throw "Couldn't find $($ModuleSpec.Name)"
        }
        Import-Module ($ModulePath | Convert-Path) -Force -Verbose:$false
    }
    catch
    {
        throw "Failed to import $($ModuleSpec.Name).`n$($_.Exception.Message)"
    }
}
# This repo makes use of Invoke-Build/Pester to run our builds so we need to import them.
try
{
    # Both modules should have been grabbed from nuget by paket, we simply need to import them
    Write-Verbose 'Importing Invoke-Build'
    Join-Path $Global:BrownserveRepoNugetPackagesDirectory 'Invoke-Build' -AdditionalChildPath 'tools', 'InvokeBuild.psd1' | Import-Module -Force -Verbose:$false
    Write-Verbose 'Importing Pester'
    Join-Path $Global:BrownserveRepoNugetPackagesDirectory 'Pester' -AdditionalChildPath 'tools', 'Pester.psd1' | Import-Module -Force -Verbose:$False
}
catch
{
    throw "Failed to import build/test modules.`n$($_.Exception.Message)"
}


# Place any custom code below, this will be preserved whenever you update your _init script (DO NOT REMOVE THIS SECTION)
### Start user defined _init steps

### End user defined _init steps

# If we're not suppressing output then we'll pipe out a list of cmdlets that are now available to the user along with
# Their synopsis.
if (!$SuppressOutput)
{
    if ($Global:BrownserveCmdlets)
    {
        Write-Host "The following modules have been loaded and their functions are now available:`n"
        $Global:BrownserveCmdlets | ForEach-Object {
            Write-Host "$($_.Module):" -ForegroundColor Yellow
            $_.Cmdlets | ForEach-Object {
                Write-Host "    $($_.Name) " -ForegroundColor Magenta -NoNewline; Write-Host "|  $($_.Synopsis)" -ForegroundColor Blue
            }
            ''
        }
        Write-Host "For more information please use the 'Get-Help <command-name>' command`n"
    }
}
