## ADDED Requirements

### Requirement: Tag-triggered release workflow

The repository SHALL provide a GitHub Actions workflow that is triggered exclusively by pushing a git tag matching `v*` to the default branch, and SHALL refuse to run on `push` or `pull_request` events targeting branches.

#### Scenario: Tag push triggers release

- **WHEN** a maintainer pushes a tag `v0.5.2` (or any `v<semver>`) to `origin`
- **THEN** GitHub Actions starts the release workflow on a `macos-latest` runner
- **AND** the workflow checks out the tagged commit, not the default branch tip

#### Scenario: Non-tag push does not trigger release

- **WHEN** a commit is pushed to `main` without a tag, or a pull request is opened
- **THEN** the release workflow does not start
- **AND** the existing `CI` workflow (lint/test/build) runs as it does today

#### Scenario: Tag without `v` prefix is ignored

- **WHEN** a tag like `0.5.2` or `release-1` is pushed
- **THEN** the release workflow does not start

### Requirement: Release artifact build

The release workflow SHALL build the SafeMic DMG using the same recipe as the local `mise run release` task so that CI artifacts and local artifacts are byte-for-byte equivalent given the same toolchain.

#### Scenario: Workflow invokes mise

- **WHEN** the release workflow runs
- **THEN** it installs `mise` and runs `mise run release` (or invokes the underlying steps in the same order: `cargo build --locked --release --target aarch64-apple-darwin`, `cargo bundle --release --target aarch64-apple-darwin`, `rcodesign sign`, `hdiutil create`)
- **AND** the output is a file named `safemic-<version>-aarch64-apple-darwin.dmg` where `<version>` is the Cargo package version, matching the local recipe

#### Scenario: Cargo version mismatch with tag fails fast

- **WHEN** the tag is `v0.5.2` but `Cargo.toml` `package.version` is not `0.5.2`
- **THEN** the workflow fails before building, with an error message naming both versions
- **AND** no DMG is produced and no GitHub Release is created

### Requirement: GitHub Release publication

The release workflow SHALL publish a non-draft GitHub Release for the triggering tag with the DMG and its sha256 checksum file attached as assets.

#### Scenario: Release is created with assets

- **WHEN** the build step completes successfully
- **THEN** a GitHub Release is created (or updated) for the tag with `draft: false` and `prerelease: false`
- **AND** the DMG `safemic-<version>-aarch64-apple-darwin.dmg` is attached
- **AND** a file `safemic-<version>-aarch64-apple-darwin.dmg.sha256` is attached, containing the output of `shasum -a 256 <dmg>` (single line, `<hex>  <filename>` format)

#### Scenario: Re-running on the same tag does not duplicate assets

- **WHEN** the workflow is re-run for an existing tag (e.g., manual re-run after a transient failure)
- **THEN** existing release assets with the same filenames are replaced, not duplicated

### Requirement: Tap repo cask bump

The release workflow SHALL update the cask formula in the external `vdsmon/homebrew-tap` repository so that `brew install --cask vdsmon/tap/safemic` resolves to the newly released DMG.

#### Scenario: Cask is rendered from template

- **WHEN** the DMG is published
- **THEN** the workflow reads `packaging/homebrew/safemic.rb.tmpl` from this repo
- **AND** substitutes the placeholders `{{version}}`, `{{sha256}}`, and `{{url}}` (the GitHub Releases asset URL) to produce a concrete `safemic.rb`
- **AND** writes the result to `Casks/safemic.rb` in the tap repo

#### Scenario: Cask change lands as a PR

- **WHEN** the workflow has rendered the new cask
- **THEN** it pushes a branch `bump-safemic-<version>` to `vdsmon/homebrew-tap` and opens a pull request titled `safemic <version>` against the tap repo's default branch
- **AND** the PR body links back to the GitHub Release for this tag
- **AND** authentication uses the secret `TAP_REPO_TOKEN`, never the default `GITHUB_TOKEN`

#### Scenario: Tap PR for the same version is reused

- **WHEN** the workflow is re-run for a tag that already has an open tap PR
- **THEN** the existing branch is force-updated, not duplicated, and the PR remains the same

### Requirement: Cask installs and launches without Gatekeeper block

The cask formula SHALL install SafeMic to `/Applications/SafeMic.app` and SHALL ensure the bundle is not blocked by macOS Gatekeeper on first launch, even though the DMG is rcodesign self-signed rather than Apple notarized.

#### Scenario: arm64 install

- **WHEN** a user on a clean macOS arm64 machine runs `brew tap vdsmon/tap` then `brew install --cask safemic`
- **THEN** the cask downloads the DMG, verifies its sha256, mounts it, copies `SafeMic.app` to `/Applications/`
- **AND** runs the `postflight` block which executes `xattr -dr com.apple.quarantine /Applications/SafeMic.app`
- **AND** opening `/Applications/SafeMic.app` from Finder launches the tray app without showing the "damaged / cannot be opened" Gatekeeper dialog

#### Scenario: Intel rejection

- **WHEN** a user on macOS x86_64 runs `brew install --cask vdsmon/tap/safemic`
- **THEN** Homebrew refuses the install with an architecture mismatch error sourced from the cask's `depends_on arch: :arm64` declaration
- **AND** no DMG download is attempted

#### Scenario: Sha256 mismatch fails the install

- **WHEN** the DMG asset on GitHub Releases is modified after the cask was published, so its sha256 no longer matches the cask
- **THEN** `brew install --cask` aborts with a checksum mismatch error
- **AND** no files are copied to `/Applications/`

### Requirement: Cask uninstall and cleanup

The cask formula SHALL remove SafeMic and all user-state side-effects on `brew uninstall --cask --zap` so that uninstalling leaves no orphan files.

#### Scenario: Uninstall removes app

- **WHEN** a user runs `brew uninstall --cask safemic`
- **THEN** `/Applications/SafeMic.app` is removed
- **AND** the user's settings at `~/Library/Application Support/safemic/settings.json` are preserved

#### Scenario: Zap removes user state

- **WHEN** a user runs `brew uninstall --cask --zap safemic`
- **THEN** `~/Library/Application Support/safemic/` is removed
- **AND** `~/Library/LaunchAgents/com.vdsmon.safemic.plist` is removed (if present)
- **AND** any bundle caches under `~/Library/Caches/com.vdsmon.safemic/` are removed

### Requirement: Documentation surfaces Homebrew install path

The repository SHALL document Homebrew as the primary install method for end users so that the new path is discoverable from the README without the user having to dig through release notes.

#### Scenario: README has Homebrew section

- **WHEN** a reader visits the repository's `README.md`
- **THEN** the installation section's first option is `brew install --cask vdsmon/tap/safemic` (preceded by `brew tap vdsmon/tap` if the user has not tapped before)
- **AND** the manual DMG instructions remain available below the Homebrew section under a "Manual install" subsection

#### Scenario: CLAUDE.md release section is current

- **WHEN** a Claude Code session reads `CLAUDE.md`
- **THEN** the build/dev commands section notes that tagging `vX.Y.Z` triggers the release workflow
- **AND** that `mise run release` is the local dev mirror of what CI runs, not a separate release path
