# Contributing

Pull requests are welcome. Please read this guide before submitting.

## Prerequisites

- [Rust toolchain](https://rustup.rs) (stable)
- [PowerShell 7+](https://github.com/PowerShell/PowerShell) (for the build scripts)

## Building locally

```sh
# Build in release mode
cargo build --release

# Run all tests
cargo test --workspace
```

You can also use the Brownserve build script for a full build-and-test run:

```powershell
./.build/build.ps1 -Build BuildTestAndCheck
```

## Commit and PR requirements

> **Please Note:**
> Our branch protection rules **require** all commits to be [signed](https://docs.github.com/en/github/authenticating-to-github/managing-commit-signature-verification/signing-commits).
> While we can rebase and sign commits for you it's much more likely that your PR will be merged promptly if you ensure your commits are signed before submitting the PR.

We use the [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/) standard for PR titles and **this is a hard requirement.** Your PR title must begin with a recognised prefix so that an automated workflow can classify the change for the changelog.

Supported prefixes (brackets are optional):

| Prefix examples | Type |
| --- | --- |
| `[feat]:` `feat:` `[feature]:` `feature:` | New feature or enhancement |
| `[fix]:` `fix:` `[bug]:` `bug:` | Bug fix |
| `[docs]:` `docs:` `[doc]:` `doc:` | Documentation update |
| `[ci]:` `ci:` `[cicd]:` `cicd:` | CI/CD changes |
| `[chore]:` `[refactor]:` `[ops]:` `[test]:` `[style]:` (and without brackets) | Maintenance |

Add `!` before the colon to flag a breaking change, e.g. `feat!: drop support for older platforms`.

> **Please Note:**
> If your PR title does not match a recognised prefix the check will fail and a comment will be posted on the PR explaining what to fix. Simply update the title and the checks will re-run automatically.
