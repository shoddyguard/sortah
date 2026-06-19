<#
.SYNOPSIS
    This contains the build tasks for Invoke-Build to use.
    Each task is documented with a synopsis and description to explain what it does.
#>
[CmdletBinding()]
param
(
    # The name of the binary to build and package (without extension)
    [Parameter(
        Mandatory = $false
    )]
    [string]
    $BinaryName = $Global:BrownserveRepoName,

    # The branch this is being built from
    [Parameter(
        Mandatory = $true
    )]
    [string]
    $BranchName,

    # The default branch for this repository
    [Parameter(
        Mandatory = $true
    )]
    [string]
    $DefaultBranch,

    # The type of changes that this version contains (used to determine the version number)
    [Parameter(
        Mandatory = $false
    )]
    [ValidateSet(
        'major',
        'minor',
        'patch'
    )]
    [string]
    $ReleaseType,

    # The various places to publish to
    [Parameter(
        Mandatory = $false
    )]
    [ValidateNotNullOrEmpty()]
    [ValidateSet('GitHub')]
    [string[]]
    $PublishTo,

    # The GitHub organisation/account to publish the release to
    [Parameter(
        Mandatory = $true
    )]
    [ValidateNotNullOrEmpty()]
    [string]
    $GitHubRepoOwner,

    # The GitHub repo name
    [Parameter(
        Mandatory = $false
    )]
    [ValidateNotNullOrEmpty()]
    [string]
    $GitHubRepoName = $Global:BrownserveRepoName,

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
    # When not set the build uses the host target.
    [Parameter(
        Mandatory = $false
    )]
    [string]
    $Target
)

# Depending on how we got the branch name we may need to remove the full ref
$BranchName = $BranchName -replace 'refs\/heads\/', ''
$script:CurrentCommitHash = & git rev-parse HEAD

# Script-scoped variables populated by tasks
$script:Release          = $false
$script:Stage            = $false
$script:Changelog        = $null
$script:CurrentVersion   = $null
$script:NewVersion       = $null
$script:PrefixedVersion  = $null
$script:ReleaseVersion   = $null
$script:ReleaseNotes     = $null
$script:StagingBranchName = $null
$script:PRLink           = $null
$script:TrackedFiles     = @()
$script:ChangelogPath    = Join-Path $Global:BrownserveRepoRootDirectory 'CHANGELOG.md'

# Resolve the effective Rust target triple used by Package to name the archive.
# When -Target is supplied we use it directly; otherwise we ask rustc for the host triple.
if ($Target)
{
    $script:EffectiveTarget = $Target
}
else
{
    $RustcOutput = & rustc -vV 2>&1
    $HostLine = $RustcOutput | Select-String 'host: (.+)'
    $script:EffectiveTarget = if ($HostLine) { $HostLine.Matches[0].Groups[1].Value.Trim() } else { 'unknown-target' }
}

<#
.SYNOPSIS
    Validates that all parameters required to publish a release have been provided.
#>
task CheckPublishingParameters {
    if (!$PublishTo)
    {
        throw 'PublishTo must be set when performing a release'
    }
    if ('GitHub' -in $PublishTo)
    {
        if (!$GitHubReleaseToken)
        {
            throw 'GitHubReleaseToken must be provided when publishing to GitHub'
        }
    }
}

<#
.SYNOPSIS
    Validates that all parameters required to stage a release have been provided.
#>
task CheckStagingParameters {
    if (!$GitHubStageReleaseToken)
    {
        throw 'GitHubStageReleaseToken must be set when staging a release'
    }
}

<#
.SYNOPSIS
    Sets up additional variables required for staging a release.
#>
task SetStagingVariables {
    Write-Verbose 'Setting staging variables'
    $script:Stage = $true
}

<#
.SYNOPSIS
    Sets up additional variables required for a release.
#>
task SetReleaseVariables {
    Write-Verbose 'Setting release variables'
    $script:Release = $true
}

<#
.SYNOPSIS
    Reads the changelog and extracts the version history.
#>
task GetReleaseHistory {
    Write-Build White 'Getting release history'
    $script:Changelog = Read-BrownserveChangelog `
        -ChangelogPath $script:ChangelogPath
    if ($script:Release -ne $true)
    {
        $script:CurrentVersion = ($script:Changelog.VersionHistory | Where-Object { !$_.PreRelease } | Select-Object -First 1).Version
    }
    else
    {
        $script:CurrentVersion = $script:Changelog.LatestVersion.Version
    }
    Write-Verbose "Current version: $script:CurrentVersion"
}

<#
.SYNOPSIS
    Determines the new version number based on the release type.
#>
task SetVersion GetReleaseHistory, {
    Write-Build White 'Setting version'
    if (!$ReleaseType)
    {
        throw 'ReleaseType must be set when staging or releasing'
    }
    $script:NewVersion = switch ($ReleaseType)
    {
        'major' { [semver]::new($script:CurrentVersion.Major + 1, 0, 0) }
        'minor' { [semver]::new($script:CurrentVersion.Major, $script:CurrentVersion.Minor + 1, 0) }
        'patch' { [semver]::new($script:CurrentVersion.Major, $script:CurrentVersion.Minor, $script:CurrentVersion.Patch + 1) }
    }
    $script:PrefixedVersion = "v$script:NewVersion"
    Write-Verbose "New version: $script:PrefixedVersion"
    $script:StagingBranchName = "release/$script:PrefixedVersion"
}

<#
.SYNOPSIS
    Writes the new version into the root Cargo.toml [workspace.package] table and regenerates Cargo.lock.
.DESCRIPTION
    Requires the root Cargo.toml to contain a [workspace.package] section with a version field.
    The version written is the plain semver string (e.g. 1.2.0) without the 'v' prefix, as required by Cargo.
    Both Cargo.toml and the regenerated Cargo.lock are added to the set of tracked files that will be
    committed to the staging branch alongside CHANGELOG.md.
    Note: this task requires Rust/Cargo to be available on the runner because it calls 'cargo generate-lockfile'.
    The stage-release GitHub Actions workflow installs Rust before calling this build script.
#>
task UpdateCargoVersion SetVersion, {
    Write-Build White 'Updating Cargo.toml version'
    $CargoTomlPath = Join-Path $Global:BrownserveRepoRootDirectory 'Cargo.toml' | Convert-Path
    $CargoContent  = Get-Content -Path $CargoTomlPath -Raw

    # Plain semver without 'v' prefix, as Cargo requires
    $NewSemver = "$($script:NewVersion.Major).$($script:NewVersion.Minor).$($script:NewVersion.Patch)"

    $CargoVersionPattern = '(?ms)(\[workspace\.package\].*?^version\s*=\s*")[^"]*(")'
    if (-not [regex]::IsMatch($CargoContent, $CargoVersionPattern))
    {
        throw "Failed to update version in Cargo.toml. Ensure the root Cargo.toml has a [workspace.package] section containing a version = '...' field."
    }
    $Updated = $CargoContent -replace $CargoVersionPattern, "`${1}$NewSemver`${2}"
    Set-Content -Path $CargoTomlPath -Value $Updated -NoNewline
    $script:TrackedFiles += $CargoTomlPath
    Write-Verbose "Cargo.toml version updated to $NewSemver"

    # Regenerate Cargo.lock so it stays consistent with the new Cargo.toml version
    Write-Build White 'Regenerating Cargo.lock'
    try
    {
        exec { cargo generate-lockfile }
    }
    catch
    {
        throw "Failed to regenerate Cargo.lock.`n$($_.Exception.Message)"
    }
    $CargoLockPath = Join-Path $Global:BrownserveRepoRootDirectory 'Cargo.lock' | Convert-Path
    $script:TrackedFiles += $CargoLockPath
}

<#
.SYNOPSIS
    Creates a new changelog entry for the upcoming release.
#>
task CreateChangelogEntry SetVersion, {
    if ($script:CurrentVersion -eq $script:NewVersion)
    {
        throw 'Current version and new version are the same, cannot create changelog entry'
    }
    Write-Build White "Creating new changelog entry for '$script:NewVersion'"
    $NewChangelogEntryParams = @{
        Version         = $script:NewVersion
        RepositoryOwner = $GitHubRepoOwner
        RepositoryName  = $GitHubRepoName
        ChangelogObject = $script:Changelog
        SinceVersion    = $script:CurrentVersion
    }
    if ($GitHubStageReleaseToken)
    {
        $NewChangelogEntryParams.Add('Auto', $true)
        $NewChangelogEntryParams.Add('GitHubToken', $GitHubStageReleaseToken)
    }
    else
    {
        throw 'GitHub token not provided, cannot generate release notes'
    }
    try
    {
        $script:NewReleaseNotes = New-BrownserveChangelogEntry @NewChangelogEntryParams
    }
    catch
    {
        throw "Failed to create changelog entry.`n$($_.Exception.Message)"
    }
}

<#
.SYNOPSIS
    Updates the changelog with the new release notes.
#>
task UpdateChangelog CreateChangelogEntry, {
    Write-Build White 'Updating changelog'
    try
    {
        $script:Changelog | Add-BrownserveChangelogEntry `
            -NewContent $script:NewReleaseNotes `
            -ErrorAction 'Stop'
    }
    catch
    {
        throw "Failed to update changelog.`n$($_.Exception.Message)"
    }
    $script:TrackedFiles += ($script:ChangelogPath | Convert-Path)
}

<#
.SYNOPSIS
    Creates a remote staging branch via the GitHub API.
#>
task CreateStagingBranch SetVersion, {
    Write-Build White "Creating staging branch '$script:StagingBranchName'"
    try
    {
        New-GitHubBranch `
            -RepositoryOwner $GitHubRepoOwner `
            -RepositoryName  $GitHubRepoName `
            -BranchName      $script:StagingBranchName `
            -SHA             $script:CurrentCommitHash `
            -Token           $GitHubStageReleaseToken `
            -ErrorAction 'Stop'
    }
    catch
    {
        throw "Failed to create staging branch '$script:StagingBranchName'.`n$($_.Exception.Message)"
    }
}

<#
.SYNOPSIS
    Commits tracked file changes (CHANGELOG.md, Cargo.toml, Cargo.lock) to the staging branch.
#>
task CommitTrackedChanges UpdateChangelog, UpdateCargoVersion, CreateStagingBranch, {
    if ($script:TrackedFiles.Count -gt 0)
    {
        Write-Build White 'Committing tracked changes'
        try
        {
            $Files = $script:TrackedFiles | ForEach-Object {
                @{
                    Path    = [System.IO.Path]::GetRelativePath($Global:BrownserveRepoRootDirectory, $_).Replace('\', '/')
                    Content = Get-Content -Path $_ -Raw
                }
            }
            New-GitHubCommit `
                -RepositoryOwner $GitHubRepoOwner `
                -RepositoryName  $GitHubRepoName `
                -BranchName      $script:StagingBranchName `
                -CommitMessage   "docs: Prepare for $script:PrefixedVersion`n`nThis commit was automatically generated." `
                -Files           $Files `
                -Token           $GitHubStageReleaseToken `
                -ErrorAction 'Stop'
        }
        catch
        {
            throw "Failed to commit tracked changes.`n$($_.Exception.Message)"
        }
    }
    else
    {
        Write-Verbose 'No tracked files to commit.'
    }
}

<#
.SYNOPSIS
    Creates a pull request for the staged release.
#>
task CreatePullRequest CommitTrackedChanges, {
    Write-Build White 'Creating pull request'
    try
    {
        $Body = @'
This PR was automatically generated.
Please review the changes and merge if they look good.
'@
        $PullRequestParams = @{
            BaseBranch      = $DefaultBranch
            HeadBranch      = $script:StagingBranchName
            Title           = "Prepare for $script:PrefixedVersion"
            Body            = $Body
            GitHubToken     = $GitHubStageReleaseToken
            RepositoryName  = $GitHubRepoName
            RepositoryOwner = $GitHubRepoOwner
        }
        $PRDetails = New-GitHubPullRequest @PullRequestParams
        $script:PRLink = $PRDetails.html_url
        Write-Debug "PRLink: $script:PRLink"
    }
    catch
    {
        throw "Failed to create pull request.`n$($_.Exception.Message)"
    }
}

<#
.SYNOPSIS
    Compiles the Rust application in release mode.
.DESCRIPTION
    When -Target is specified the binary is placed under target/<triple>/release/.
    Otherwise it lands in target/release/ (host target).
#>
task Build {
    Write-Build White "Building '$BinaryName'"
    $CargoArgs = @('build', '--release')
    if ($Target)
    {
        $CargoArgs += '--target'
        $CargoArgs += $Target
    }
    try
    {
        exec { & cargo $CargoArgs }
    }
    catch
    {
        throw "Cargo build failed.`n$($_.Exception.Message)"
    }
}

<#
.SYNOPSIS
    Runs all Cargo tests for the workspace.
#>
task CargoTest {
    Write-Build White 'Running Cargo tests'
    try
    {
        exec { cargo test --workspace }
    }
    catch
    {
        throw "Cargo tests failed.`n$($_.Exception.Message)"
    }
}

<#
.SYNOPSIS
    Runs Pester tests (binary smoke tests) for the repository.
#>
task Tests {
    Write-Build White 'Running Pester tests'
    $Results = Invoke-Pester -Path $Global:BrownserveRepoTestsDirectory -PassThru
    assert ($Results.FailedCount -eq 0) "$($Results.FailedCount) test(s) failed."
}

<#
.SYNOPSIS
    Archives the compiled binary for the current platform and target.
.DESCRIPTION
    The archive is written to $Global:BrownserveRepoBuildOutputDirectory (.tmp/output) using the
    naming convention: <binary>-v<version>-<target-triple>.tar.gz (Linux/macOS) or .zip (Windows).
    In the release workflow each matrix runner runs this task, then uploads the archive as a
    GitHub Actions artifact for the final Release job to collect and attach to the GitHub release.
#>
task Package GetReleaseHistory, Build, {
    Write-Build White "Packaging '$BinaryName' for target '$script:EffectiveTarget'"
    $script:ReleaseVersion = $script:Changelog.LatestVersion.Version.ToString()
    $script:PrefixedVersion = "v$script:ReleaseVersion"

    # Locate the compiled binary
    $BinDir = if ($Target)
    {
        Join-Path $Global:BrownserveRepoRootDirectory 'target' $Target 'release'
    }
    else
    {
        Join-Path $Global:BrownserveRepoRootDirectory 'target' 'release'
    }
    $BinaryFileName = if ($IsWindows) { "$BinaryName.exe" } else { $BinaryName }
    $BinaryPath     = Join-Path $BinDir $BinaryFileName
    if (!(Test-Path $BinaryPath))
    {
        throw "Compiled binary not found at '$BinaryPath'. Ensure the Build task ran successfully."
    }

    # Create the archive in the build output directory
    if ($IsWindows)
    {
        $ArchiveName = "$BinaryName-$script:PrefixedVersion-$script:EffectiveTarget.zip"
        $ArchivePath = Join-Path $Global:BrownserveRepoBuildOutputDirectory $ArchiveName
        Write-Build White "Creating zip archive '$ArchiveName'"
        Compress-Archive -Path $BinaryPath -DestinationPath $ArchivePath -Force
    }
    else
    {
        $ArchiveName = "$BinaryName-$script:PrefixedVersion-$script:EffectiveTarget.tar.gz"
        $ArchivePath = Join-Path $Global:BrownserveRepoBuildOutputDirectory $ArchiveName
        Write-Build White "Creating tar archive '$ArchiveName'"
        exec { tar -czf $ArchivePath -C $BinDir $BinaryFileName }
    }
    Write-Build Green "Archive created: $ArchivePath"
}

<#
.SYNOPSIS
    Creates a GitHub release and uploads all archives from the build output directory as assets.
.DESCRIPTION
    This task is intended to run once on a single Linux runner after all platform binaries have been
    downloaded from GitHub Actions artifacts into $Global:BrownserveRepoBuildOutputDirectory.
    It does not compile anything; it only publishes.
#>
task PublishRelease GetReleaseHistory, {
    Write-Build White 'Publishing release'
    $script:ReleaseVersion = $script:Changelog.LatestVersion.Version.ToString()
    $script:PrefixedVersion = "v$script:ReleaseVersion"
    $script:ReleaseNotes    = $script:Changelog.LatestVersion.ReleaseNotes -join "`n"

    if ('GitHub' -in $PublishTo)
    {
        Write-Build White 'Checking for existing GitHub releases'
        $CurrentReleases = Get-GitHubRelease `
            -GitHubToken $GitHubReleaseToken `
            -RepoName    $GitHubRepoName `
            -GitHubOrg   $GitHubRepoOwner
        if ($CurrentReleases.tag_name -contains $script:PrefixedVersion)
        {
            throw "A GitHub release for $script:PrefixedVersion already exists!"
        }

        # Validate archives are present before creating the release so we never
        # end up with a published release that has no assets.
        $Archives = Get-ChildItem -Path $Global:BrownserveRepoBuildOutputDirectory -File |
            Where-Object { $_.Extension -in @('.gz', '.zip') }
        if (!$Archives)
        {
            throw "No release archives found in '$Global:BrownserveRepoBuildOutputDirectory'. Ensure the Package task ran on all platforms first."
        }

        Write-Build White "Creating GitHub release $script:PrefixedVersion"
        $ReleaseResponse = New-GitHubRelease `
            -Name            $script:PrefixedVersion `
            -Tag             $script:PrefixedVersion `
            -Description     $script:ReleaseNotes `
            -GitHubToken     $GitHubReleaseToken `
            -RepositoryName  $GitHubRepoName `
            -RepositoryOwner $GitHubRepoOwner
        foreach ($Archive in $Archives)
        {
            Write-Build White "Uploading '$($Archive.Name)' as release asset"
            Add-GitHubReleaseAsset `
                -UploadUrl $ReleaseResponse.upload_url `
                -Token     $GitHubReleaseToken `
                -FilePath  $Archive.FullName `
                -ErrorAction 'Stop'
        }
    }
    else
    {
        Write-Verbose 'GitHub not targeted, skipping...'
    }
}

<#
    Below are the meta tasks used to compose the above leaf tasks.
    These are the task names that build.ps1 passes to Invoke-Build via -Task.
    !! BE VERY CAREFUL WITH THE ORDERING OF DEPENDENCIES !!
#>

<#
.SYNOPSIS
    Meta task: build, run all Cargo tests and Pester smoke tests.
    Used by the CI pipeline on pull requests.
#>
task BuildTestAndCheck Build, CargoTest, Tests, {}

<#
.SYNOPSIS
    Meta task: stages a release by bumping CHANGELOG.md, Cargo.toml and Cargo.lock,
    then opening a pull request.
#>
task StageRelease CheckStagingParameters, SetStagingVariables, GetReleaseHistory, SetVersion, UpdateCargoVersion, CreateChangelogEntry, UpdateChangelog, CreateStagingBranch, CommitTrackedChanges, CreatePullRequest, {
    $BuildMessage = @"
The release has been successfully staged and a pull request has been created.
Please review the changes at $script:PRLink and merge if they look good.
"@
    Write-Build Green $BuildMessage
}

<#
.SYNOPSIS
    Meta task: performs all release steps except publishing to GitHub.
    Useful for verifying the release process locally without pushing anything.
#>
task DryRun SetReleaseVariables, CheckPublishingParameters, GetReleaseHistory, Build, CargoTest, Tests, Package, {}

<#
.SYNOPSIS
    Meta task: creates a GitHub release and uploads all platform archives as assets.
    Run this after the Package jobs have completed and their archives are in .tmp/output.
#>
task Release CheckPublishingParameters, SetReleaseVariables, GetReleaseHistory, PublishRelease, {}
