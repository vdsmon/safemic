## Why

SafeMic currently ships only as an unsigned DMG that users download from GitHub, drag to `/Applications`, then manually run `xattr -dr com.apple.quarantine` (or dance through "Open Anyway" in Privacy & Security). Three friction points: discovery, install ergonomics, and update cadence. Homebrew Cask collapses all three into `brew install --cask vdsmon/tap/safemic` and `brew upgrade`, which matches how the target audience (macOS power users running a system-wide mic mute) installs everything else.

Doing it via a personal tap rather than upstream `homebrew-cask` is the pragmatic first step: upstream requires Apple Developer ID notarization, which SafeMic does not have yet, so any submission would be rejected. The personal tap unblocks distribution today and leaves notarization + upstream submission as a follow-up change once we're ready to pay for an Apple developer account.

## What Changes

- Add a tag-triggered GitHub Actions release workflow (`.github/workflows/release.yaml`) that runs `mise run release` on `macos-latest`, attaches the resulting DMG + sha256 to a GitHub Release for the tag, then opens (or pushes) a PR in the external `vdsmon/homebrew-tap` repo updating `Casks/safemic.rb` with the new version, url, and sha256.
- Add a cask template file at `packaging/homebrew/safemic.rb.tmpl` in this repo. The release workflow renders it (substituting version + sha256) and ships it to the tap. Template declares `depends_on arch: :arm64`, `name`, `desc`, `homepage`, `app "SafeMic.app"`, a `zap` stanza that removes `~/Library/Application Support/safemic/`, the `~/Library/LaunchAgents/com.vdsmon.safemic.plist`, and the bundle caches, and a `postflight` block that strips `com.apple.quarantine` from the installed `.app` so first launch isn't blocked by Gatekeeper (necessary because the DMG is `rcodesign` self-signed, not Apple notarized).
- Bootstrap the external tap repo `vdsmon/homebrew-tap` (create the GitHub repo, seed it with `Casks/safemic.rb` rendered from the template at the current `v0.5.x` release, plus a one-paragraph README explaining `brew tap vdsmon/tap` + `brew install --cask safemic`). Bootstrapping is a one-time human step performed as part of applying this change, not a recurring task in the release workflow.
- Add a fine-grained personal access token as repo secret `TAP_REPO_TOKEN` (scoped to `vdsmon/homebrew-tap`, contents:write + pull-requests:write only). Documented in the workflow's job env and in the new release section of `CLAUDE.md`.
- Update `README.md`: new "Install via Homebrew" section at the top of installation instructions, demoting the existing manual DMG steps to a secondary "Manual install" subsection.
- Update `CLAUDE.md` "Build / dev commands" table with a new `Release` row pointing at the tag-push trigger, plus a one-paragraph note that tagging `vX.Y.Z` is the canonical release action and that `mise run release` is now the dev-time mirror of what CI runs.

## Capabilities

### New Capabilities

- `homebrew-distribution`: end-to-end flow from a git tag in this repo to an installable `brew install --cask vdsmon/tap/safemic` on a clean macOS arm64 machine. Covers the GitHub Actions release workflow, the cask template, the tap-repo update mechanism, and the Gatekeeper postflight. Does not cover the existing local `mise run release` task or the unrelated build/test pipeline.

### Modified Capabilities

(none — no existing capability specs in this repo yet)

## Impact

- **New files**:
  - `.github/workflows/release.yaml` — tag-triggered release + tap-bump workflow.
  - `packaging/homebrew/safemic.rb.tmpl` — cask template with placeholder substitution markers.
- **Modified files**:
  - `README.md` — new Homebrew install section.
  - `CLAUDE.md` — release section update.
- **External repos** (created by this change, lives outside this repo):
  - `github.com/vdsmon/homebrew-tap` — bootstrapped with `Casks/safemic.rb` and a minimal README. Subsequent updates land via PR from this workflow.
- **Secrets**:
  - `TAP_REPO_TOKEN` added to this repo's Actions secrets. Fine-grained PAT scoped to the tap repo only.
- **Dependencies**: no new runtime dependencies. CI gains `rcodesign` install (already done by `mise run release:deps`) and a one-liner to compute `sha256` (`shasum -a 256`, in macOS base image).
- **Risk**: a leaked `TAP_REPO_TOKEN` would let an attacker push arbitrary cask changes to `vdsmon/homebrew-tap`, which would let them install arbitrary code on anyone who runs `brew install --cask vdsmon/tap/safemic` afterward. Mitigated by fine-grained scope (only the tap repo, only the two needed permissions) and by users having to opt in to the tap. Not mitigated by code signing (cask postflight removes quarantine).
- **No breaking changes** for existing users; the manual DMG path stays available.
