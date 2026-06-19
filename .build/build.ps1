<#
.SYNOPSIS
    Builds and releases the Rust application via Invoke-Build.
#>
[CmdletBinding()]
param
(
    # The name of the default branch
    [Parameter(
        Mandatory = $false
    )]
    [string]
    $DefaultBranch = 'main',

    # The name of the branch you are running on
    # this is used to work out if the release is production or pre-release
    [Parameter(
        Mandatory = $false
    )]
    [ValidateNotNullOrEmpty()]
    [string]
    $BranchName,

    # The build to run
    [Parameter(
        Mandatory = $false
    )]
    [ValidateSet(
        'Build',
        'BuildTestAndCheck',
        'Package',
        'StageRelease',
        'DryRun',
        'Release'
    )]
    [AllowEmptyString()]
    [string]
    $Build = 'Build',

    # When preparing a release this denotes the type of changes that have been made.
    # This is used to determine the version number to use for the release.
    [Parameter(
        Mandatory = $false
    )]
    [ValidateSet(
        'major',
        'minor',
        'patch'
    )]
    [string]
    $ReleaseType = 'minor',

    # The various places to publish to
    [Parameter(
        Mandatory = $false
    )]
    [ValidateNotNullOrEmpty()]
    [ValidateSet('GitHub')]
    [string[]]
    $PublishTo,

    # The GitHub organisation/account that owns this repo
    [Parameter(
        Mandatory = $false
    )]
    [ValidateNotNullOrEmpty()]
    [string]
    $GitHubRepoOwner = 'shoddyguard',

    # The GitHub repo name
    [Parameter(
        Mandatory = $false
    )]
    [ValidateNotNullOrEmpty()]
    [string]
    $GitHubRepoName,

    # GitHub token used during the StageRelease build, must have the following permissions:
    #   * Read/Write pull requests
    #   * Read issues
    [Parameter(
        Mandatory = $false
    )]
    [string]
    $GitHubStageReleaseToken,

    # GitHub token used during the Release build, must have the following permissions:
    #   * Read/write releases
    [Parameter(
        Mandatory = $false
    )]
    [string]
    $GitHubReleaseToken,

    # The Rust target triple to build for (e.g. x86_64-unknown-linux-gnu).
    # Defaults to the host target when not specified.
    [Parameter(
        Mandatory = $false
    )]
    [string]
    $Target
)
# Always stop on errors
$ErrorActionPreference = 'Stop'
# If we don't have a branch name then try to work it out automatically
if (!$BranchName)
{
    $BranchName = & git rev-parse --abbrev-ref HEAD
}
# If we still don't have a branch name then set it to something sensible
if (!$BranchName)
{
    $BranchName = 'preview'
}
# Depending on how we got the branch name we may need to remove the full ref
$BranchName = $BranchName -replace 'refs\/heads\/', ''

# Run the init script
try
{
    Write-Verbose 'Starting build script'
    $initScriptPath = Join-Path $PSScriptRoot -ChildPath '_init.ps1' | Convert-Path
    . $initScriptPath
}
catch
{
    Write-Error "Failed to init repo.`n$($_.Exception.Message)"
}

# Invoke our build task
try
{
    $BuildParams = @{
        File          = (Join-Path -Path $global:BrownserveRepoBuildTasksDirectory -ChildPath 'build_tasks.ps1' | Convert-Path)
        Task          = $Build
        BranchName    = $BranchName
        DefaultBranch = $DefaultBranch
        BinaryName    = 'sortah'
    }
    if ($ReleaseType)
    {
        $BuildParams.Add('ReleaseType', $ReleaseType)
    }
    if ($Target)
    {
        $BuildParams.Add('Target', $Target)
    }
    if ($GitHubRepoOwner)
    {
        $BuildParams.Add('GitHubRepoOwner', $GitHubRepoOwner)
    }
    if ($GitHubRepoName)
    {
        $BuildParams.Add('GitHubRepoName', $GitHubRepoName)
    }
    if ($GitHubStageReleaseToken)
    {
        $BuildParams.Add('GitHubStageReleaseToken', $GitHubStageReleaseToken)
    }
    if ($GitHubReleaseToken)
    {
        $BuildParams.Add('GitHubReleaseToken', $GitHubReleaseToken)
    }
    if ($PublishTo)
    {
        $BuildParams.Add('PublishTo', $PublishTo)
    }
    Write-Verbose "Invoking build: $Build"
    Invoke-Build @BuildParams -Verbose:($PSBoundParameters['Verbose'] -eq $true)
}
catch
{
    Write-Error $_.Exception.Message
}
