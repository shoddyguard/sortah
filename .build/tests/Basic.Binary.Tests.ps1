#requires -Modules Pester
<#
.SYNOPSIS
    Smoke tests for the sortah binary.
    Verifies that the compiled binary exists and responds correctly to basic flags.
    These tests are intended to be run after a successful 'Build' task.
#>
Describe 'sortah binary' {
    BeforeAll {
        # Locate the binary under the build output directory set by CARGO_TARGET_DIR.
        # On Windows the executable has a .exe extension.
        $BinaryName = if ($IsWindows) { 'sortah.exe' } else { 'sortah' }
        $script:BinaryPath = Join-Path $Global:BrownserveRepoBuildOutputDirectory 'release' $BinaryName
    }

    Context 'Binary exists' {
        It 'should be present in the build output directory' {
            $script:BinaryPath | Should -Exist
        }
    }

    Context 'Basic invocation' {
        It 'should exit 0 for --help' {
            & $script:BinaryPath --help
            $LASTEXITCODE | Should -Be 0
        }

        It 'should exit 0 for --version' {
            & $script:BinaryPath --version
            $LASTEXITCODE | Should -Be 0
        }

        It 'should include the binary name in --version output' {
            $Output = & $script:BinaryPath --version 2>&1
            $Output | Should -Match 'sortah'
        }
    }
}
